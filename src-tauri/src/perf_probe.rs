//! 性能探针的宿主侧（Rust）部分——方案 `.scratch/glance-bench/issues/05`。
//!
//! 两件事：
//!
//! 1. **分段计时**（§1.7）：把贴边收起/呼出链路里 `sync_peek_window` 的各步耗时
//!    拆开记，target 是 `tum.window`。判读表第 1 行（「宿主侧开销成立」）要靠
//!    `build_ms` 判定——建窗耗时若不显著，整条 §2.1 分支就该被否决。
//! 2. **`perf_drive`**：让前端按「轮数 × 间隔」跑一遍固定动作，宿主侧等回执。
//!    这是 §1.4 测量协议（贴边并唤出、重复 ≥30 次）的自动化入口。
//!
//! ## 探针开关：为什么 Rust 侧**不**读 `VITE_PERF`
//!
//! 前端的开关是 `import.meta.env.DEV || import.meta.env.VITE_PERF === "1"`
//! （`src/lib/perf.ts`）。Rust 侧**刻意不**用 `env!("VITE_PERF")` /
//! `option_env!("VITE_PERF")` 跟着前端走，理由是编译缓存：
//!
//! - `cargo` **不会**因为编译期环境变量变化而让 crate 重新编译（除非在
//!   `build.rs` 里显式 `cargo:rerun-if-env-changed=VITE_PERF`）。于是同一个
//!   `target/` 目录里，「上次带探针 / 这次不带」会互相污染：release 测量拿到
//!   一个**静默不记日志**的二进制，比没有探针更糟——数据看着像「这段不慢」。
//! - 前端 build 与 Rust build 是两个进程：VITE_PERF 确实会同时出现在两者的
//!   环境里（`tauri build` 先跑 vite 再跑 cargo），但「出现在环境里」不等于
//!   「这次编译读到了它」。
//!
//! 所以 Rust 侧改用**运行时**开关，三条来源按或关系合并：
//!
//! 1. debug 构建恒开（`cfg!(debug_assertions)`）——对应前端的 `import.meta.env.DEV`；
//! 2. 启动时环境变量 `TUM_PERF_PROBE=1`——release 测量时用 `set TUM_PERF_PROBE=1`
//!    启动即可，**不用重编译**；
//! 3. 前端在运行时调 `set_perf_probe(true)`——前端自己知道 `PERF_PROBE_ENABLED`，
//!    由它来对齐最准（release + `VITE_PERF=1` 的构建就靠这条）。
//!
//! 开关关闭时，计时代码**一次 `Instant::now()` 都不调用**（见 [`Segments`]），
//! 生产构建的热路径开销 = 一次 relaxed 原子读 + 若干分支。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager};

/// 运行时环境变量名：值为 `1` / `true` / `on` 时打开 Rust 侧探针。
pub const ENV_FLAG: &str = "TUM_PERF_PROBE";

/// 宿主 → 前端：驱动指令的事件名。
pub const PERF_DRIVE_EVENT: &str = "perf-drive";

/// 前端 → 宿主：驱动回执的事件名。
pub const PERF_DRIVE_DONE_EVENT: &str = "perf-drive-done";

/// 探针总开关。默认关，由 [`init`] 定初值、可由 [`set_enabled`] 运行时改写。
static ENABLED: AtomicBool = AtomicBool::new(false);

/// 当前是否开启探针。热路径只做一次 relaxed 原子读。
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// 打开/关闭探针，返回切换后的状态。
pub fn set_enabled(on: bool) -> bool {
    ENABLED.store(on, Ordering::Relaxed);
    tracing::info!(target: "tum.perf", enabled = on, "perf probe toggled");
    on
}

/// 启动时决定探针初值（debug 构建恒开；其余看环境变量）。
pub fn init() {
    let env_on = env_flag_on();
    let on = cfg!(debug_assertions) || env_on;
    ENABLED.store(on, Ordering::Relaxed);
    tracing::info!(
        target: "tum.perf",
        enabled = on,
        debug_build = cfg!(debug_assertions),
        env_flag = env_on,
        "perf probe init"
    );
}

/// 读环境变量并解析成布尔。拆成纯函数是为了能单测（改进程环境变量不适合并行测试）。
fn env_flag_on() -> bool {
    match std::env::var(ENV_FLAG) {
        Ok(v) => parse_flag(&v),
        Err(_) => false,
    }
}

/// 解析开关值：`1` / `true` / `on`（大小写不敏感）算开，其余算关。
fn parse_flag(raw: &str) -> bool {
    let v = raw.trim();
    v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on")
}

// ───────────────────────────────────────────────────────────────────────────
// 分段计时
// ───────────────────────────────────────────────────────────────────────────

/// 单步计时结果。
#[derive(Clone, Copy, Debug, Default)]
pub struct Step {
    /// 该步耗时（毫秒，整数）。不足 1ms 记 0。
    pub ms: u64,
    /// 该步是否真的执行过（0ms 既可能是「很快」也可能是「没做」，靠它区分）。
    pub done: bool,
}

/// `sync_peek_window` 的分段计时器。
///
/// **探针关闭时零开销**：`start()` 不取时钟，每个 `mark_*` 直接返回，
/// 于是生产构建里既没有 `Instant::now()`，也没有日志。
pub struct Segments {
    /// 是否计时。false 时下面所有 `mark_*` 都是空操作。
    on: bool,
    /// 上一步结束的时刻（探针关闭时为 None）。
    last: Option<Instant>,
    /// 整段的起始时刻（探针关闭时为 None）。
    start_at: Option<Instant>,
    /// 贴死边缘 + 回写持久化锚点（仅 docked 态）。
    pub anchor: Step,
    /// 把手窗的获取或创建（建窗，含 WebView2 初始化）。
    pub build: Step,
    /// 改把手窗尺寸（当前分支没有这一步，§2.1 门禁②若加入即在此计）。
    pub set_size: Step,
    /// 定位（`peek_placement` + `set_position`）。
    pub set_pos: Step,
    /// 鼠标捕获归属切换（peek 与 dashboard 两次调用合计）。
    pub ignore_cursor: Step,
    /// 显示把手窗。
    pub show: Step,
    /// 向把手窗发射 `peek-show`。
    pub emit: Step,
    /// 本次把手窗是**新建**的（false = 复用了已有窗口）。
    ///
    /// 必须单独记：`build_ms ≈ 0` 既可能是「建窗很快」，也可能是「根本没建
    /// （复用）」，不区分就会把两种情况混成同一条数据。
    pub peek_created: bool,
}

impl Segments {
    /// 新建一个计时器。探针关闭时不做任何取时钟的动作。
    pub fn start() -> Self {
        Self::new(enabled())
    }

    /// 显式指定是否计时（单测用；生产走 [`Segments::start`]）。
    fn new(on: bool) -> Self {
        let now = if on { Some(Instant::now()) } else { None };
        Self {
            on,
            last: now,
            start_at: now,
            anchor: Step::default(),
            build: Step::default(),
            set_size: Step::default(),
            set_pos: Step::default(),
            ignore_cursor: Step::default(),
            show: Step::default(),
            emit: Step::default(),
            peek_created: false,
        }
    }

    /// 推进时钟：返回「距上一步标记过了多久」，并把游标移到当前。
    fn tick(&mut self) -> Option<Duration> {
        if !self.on {
            return None;
        }
        let now = Instant::now();
        let d = self.last.map(|l| now.saturating_duration_since(l));
        self.last = Some(now);
        d
    }

    fn set(slot: &mut Step, d: Option<Duration>) {
        if let Some(d) = d {
            slot.ms = d.as_millis() as u64;
            slot.done = true;
        }
    }

    /// 记「贴死边缘 + 回写持久化锚点」这一步。
    pub fn mark_anchor(&mut self) {
        let d = self.tick();
        Self::set(&mut self.anchor, d);
    }

    /// 记「获取或创建把手窗」这一步。
    pub fn mark_build(&mut self) {
        let d = self.tick();
        Self::set(&mut self.build, d);
    }

    /// 记「改把手窗尺寸」这一步。
    pub fn mark_set_size(&mut self) {
        let d = self.tick();
        Self::set(&mut self.set_size, d);
    }

    /// 记「定位把手窗」这一步。
    pub fn mark_set_pos(&mut self) {
        let d = self.tick();
        Self::set(&mut self.set_pos, d);
    }

    /// 记「鼠标捕获归属切换」这一步。
    pub fn mark_ignore_cursor(&mut self) {
        let d = self.tick();
        Self::set(&mut self.ignore_cursor, d);
    }

    /// 记「显示把手窗」这一步。
    pub fn mark_show(&mut self) {
        let d = self.tick();
        Self::set(&mut self.show, d);
    }

    /// 记「发射 peek-show」这一步。
    pub fn mark_emit(&mut self) {
        let d = self.tick();
        Self::set(&mut self.emit, d);
    }

    /// 整段耗时（毫秒）。
    pub fn total_ms(&self) -> u64 {
        match (self.on, self.start_at) {
            (true, Some(t0)) => t0.elapsed().as_millis() as u64,
            _ => 0,
        }
    }

    /// 打一行分段计时日志。target 固定 `tum.window`（见 CONTRIBUTING.md）。
    ///
    /// 字段名是前端对齐用的契约，改名要同步改 issue 05 §1.7 与前端 agent 的清单：
    /// `build_ms` / `set_size_ms` / `set_pos_ms` / `ignore_cursor_ms` /
    /// `show_ms` / `emit_ms`，外加 `anchor_ms`、`total_ms` 与三个 `*_done` 标记。
    pub fn log(&self, state: &str, side: &str) {
        if !self.on {
            return;
        }
        tracing::info!(
            target: "tum.window",
            state = state,
            side = side,
            peek_created = self.peek_created,
            anchor_ms = self.anchor.ms,
            build_ms = self.build.ms,
            set_size_ms = self.set_size.ms,
            set_pos_ms = self.set_pos.ms,
            ignore_cursor_ms = self.ignore_cursor.ms,
            show_ms = self.show.ms,
            emit_ms = self.emit.ms,
            total_ms = self.total_ms(),
            set_size_done = self.set_size.done,
            set_pos_done = self.set_pos.done,
            emit_done = self.emit.done,
            "peek sync segments"
        );
    }
}

// ───────────────────────────────────────────────────────────────────────────
// perf_drive：驱动前端跑固定动作并等回执
// ───────────────────────────────────────────────────────────────────────────

/// 一次驱动最多允许的轮数（防止参数写错把宿主挂住）。
const MAX_ROUNDS: u32 = 1000;

/// 一次驱动允许的最大轮间隔（毫秒）。
const MAX_INTERVAL_MS: u64 = 60_000;

/// 超时下限：120s（覆盖 §1.4 的常规协议：30 轮 × 1s）。
const MIN_TIMEOUT: Duration = Duration::from_secs(120);

/// 超时余量：估算值之上再留这么久，给前端收尾（落盘、最后一帧）用。
const TIMEOUT_MARGIN: Duration = Duration::from_secs(30);

/// 超时上限：30 分钟。超过这个值就说明参数写错了，早点失败比干等强。
const MAX_TIMEOUT: Duration = Duration::from_secs(1800);

/// 每轮的**额外**开销（毫秒）。
///
/// 前端驱动器的一轮不是「一个间隔」那么短：唤出 + 320ms 离开宽限 + 180ms 收起
/// 时间线 + 轮间隔，另外还有一轮 warmup。Rust 侧不知道前端的宽限/动效常量，
/// 只能按一个保守下界估——估小了会**误报超时**（比超时报错更糟：数据会被
/// 当成失败丢掉），所以这里给每轮留 1s 余量。
const PER_ROUND_OVERHEAD_MS: u64 = 1000;

/// 宿主 → 前端的驱动载荷。
///
/// 字段名必须精确是 `rounds` / `intervalMs`——前端按这两个名字解构。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRequest {
    /// 要跑多少轮。
    pub rounds: u32,
    /// 每轮之间的间隔（毫秒）。
    pub interval_ms: u64,
}

/// 前端 → 宿主的回执载荷。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveDone {
    /// 前端实际跑完的轮数。
    #[serde(default)]
    pub rounds: u32,
    /// 前端报上来的错误（可选）。
    #[serde(default)]
    pub error: Option<String>,
}

/// 按「轮数 ×（间隔 + 每轮额外开销）」估算本次驱动的超时时间。
///
/// 估算值 + 30s 余量，再夹在 [120s, 30min] 之间。**绝不无限等待**——
/// 前端没回执时必须明确失败，不能把调用方挂住。
pub fn drive_timeout(rounds: u32, interval_ms: u64) -> Duration {
    // saturating_mul 的乘数是 u32：先夹住再乘，避免 u64 溢出回绕成一个小值
    // （回绕出来的「小超时」比超时的危害大得多——它会误报失败）。
    let per_round = interval_ms.saturating_add(PER_ROUND_OVERHEAD_MS);
    let mult = per_round.min(u32::MAX as u64) as u32;
    let est = Duration::from_millis(rounds as u64).saturating_mul(mult);
    est.saturating_add(TIMEOUT_MARGIN)
        .max(MIN_TIMEOUT)
        .min(MAX_TIMEOUT)
}

/// 同一时刻只允许一次驱动。
///
/// 回执载荷里**没有请求号**，所以并发时无法把回执对回请求——「先到先得」会张冠李戴
/// （A 轮的回执被 B 收走）。宁可直接报错，也不要给出一份错配的数据。
static DRIVE_RUNNING: AtomicBool = AtomicBool::new(false);

/// 驱动的单飞守卫。离开作用域（含出错、超时）自动释放。
struct DriveGuard;

impl DriveGuard {
    fn acquire() -> Result<Self, String> {
        if DRIVE_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("已有 perf_drive 在运行中：请等它结束后再发起（回执没有请求号，并发会张冠李戴）".to_string());
        }
        Ok(DriveGuard)
    }
}

impl Drop for DriveGuard {
    fn drop(&mut self) {
        DRIVE_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 驱动前端按固定节奏跑一遍动作，宿主侧等它回执。
///
/// 协议（三方约定，字段名不能改）：
///
/// 1. 宿主向 **dashboard 窗口** emit `"perf-drive"`，载荷
///    `{"rounds": <轮数>, "intervalMs": <间隔毫秒>}`；
/// 2. 前端每轮做一次「贴边/呼出」动作，全部做完后 emit `"perf-drive-done"`，
///    载荷 `{"rounds": <实际轮数>, "error": <可选错误串>}`；
/// 3. 本命令**等这个回执**再返回。
///
/// 返回值：
/// - `Ok(一句人话)`：前端正常跑完，形如「perf-drive 完成：实际 30 轮（请求 30 轮，间隔 1000ms）」；
/// - `Err(原因)`：前端报了错（带已完成轮数）、回执没来（超时，附超时秒数）、
///   或 dashboard 窗口不存在。
///
/// 超时按 `rounds × intervalMs` 估算再加余量（见 [`drive_timeout`]），
/// 绝不无限等待——前端没回执时明确失败，不挂住调用方。
#[tauri::command]
pub async fn perf_drive(app: AppHandle, rounds: u32, interval_ms: u64) -> Result<String, String> {
    let rounds = rounds.clamp(1, MAX_ROUNDS);
    let interval_ms = interval_ms.min(MAX_INTERVAL_MS);

    let _guard = DriveGuard::acquire()?;

    let dash = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    // 先挂监听再发指令：反过来的话前端秒回执，Rust 还没准备好收。
    let (tx, rx) = tokio::sync::oneshot::channel::<DriveDone>();
    let listener_id = app.once(PERF_DRIVE_DONE_EVENT, move |event| {
        let done = match serde_json::from_str::<DriveDone>(event.payload()) {
            Ok(d) => d,
            Err(e) => DriveDone {
                rounds: 0,
                error: Some(format!("回执解析失败：{e}")),
            },
        };
        let _ = tx.send(done);
    });

    let req = DriveRequest { rounds, interval_ms };
    if let Err(e) = dash.emit(PERF_DRIVE_EVENT, &req) {
        app.unlisten(listener_id);
        return Err(format!("emit {PERF_DRIVE_EVENT} 失败：{e}"));
    }

    let timeout = drive_timeout(rounds, interval_ms);
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(done)) => {
            // once 触发后已自动注销；再注销一次是无害的幂等操作。
            app.unlisten(listener_id);
            match done.error {
                None => Ok(format!(
                    "perf-drive 完成：实际 {} 轮（请求 {rounds} 轮，间隔 {interval_ms}ms）",
                    done.rounds
                )),
                Some(err) => Err(format!(
                    "perf-drive 中断：前端报错「{err}」（已完成 {} 轮）",
                    done.rounds
                )),
            }
        }
        Ok(Err(_)) => {
            app.unlisten(listener_id);
            Err(format!(
                "perf-drive 失败：回执通道已关闭，前端没有回执 {PERF_DRIVE_DONE_EVENT}"
            ))
        }
        Err(_) => {
            app.unlisten(listener_id);
            Err(format!(
                "perf-drive 超时：等待前端回执 {PERF_DRIVE_DONE_EVENT} 超过 {}s，前端未回执",
                timeout.as_secs()
            ))
        }
    }
}

/// 运行时开关探针，返回切换后的状态。
///
/// 前端知道自己的 `PERF_PROBE_ENABLED`（`src/lib/perf.ts`），启动时调一次即可让
/// Rust 侧与前端严格对齐——release + `VITE_PERF=1` 的探针构建就靠这条，
/// Rust 侧不必（也不该）在编译期读前端的构建变量。
#[tauri::command]
pub fn set_perf_probe(enabled: bool) -> bool {
    set_enabled(enabled)
}

// ───────────────────────────────────────────────────────────────────────────
// set_size 探针：验证「peek 窗口常驻」方案的地基
// ───────────────────────────────────────────────────────────────────────────

/// 探针专用：验证 `set_size` 能否把被系统钳制的把手窗缩回目标尺寸。
///
/// ## 为什么这个验证是「地基」
///
/// 系统最小尺寸会把 `inner_size(7, 58)` 撑到约 **136×39**。而
/// `set_size` 走 SetWindowPos，**不经过创建期的 WM_GETMINMAXINFO 钳制**，
/// 所以**很可能有效——但至今没人验证过**（方案 §2.1 门禁②）。
///
/// 「让 peek 窗口常驻、不再每次重建」这个方案的全部收益来自消除
/// **619~794ms 的建窗卡顿**（实测见 issue 05 §1.9.1）；而它的全部风险
/// 也集中在跨边复用时的尺寸/位置错位（`5bba9c6` 的失败形态）。
///
/// **这个数字决定方案走哪条路**：
/// - **能缩回** → 可以做「常驻 + 跨边复用」，直接消除建窗卡顿；
/// - **缩不回** → 只能「同轴复用、跨轴销毁重建」，收益打对折。
///
/// ## 为什么放在探针里而不是生产路径
///
/// 在**确认有效之前**把它加进生产路径，就是拿用户的环境赌一个未验证的假设
/// ——而这正是 `5bba9c6` 当初犯的错。所以这里做成独立 command，
/// **只在探针开启时可达**，生产构建下不做任何事。
#[tauri::command]
pub async fn probe_peek_resize(app: AppHandle) -> Result<String, String> {
    if !enabled() {
        return Err("探针未启用（PERF_PROBE 关闭）".to_string());
    }
    let Some(peek) = app.get_webview_window("peek") else {
        return Err("peek window not found（把手窗还没建）".to_string());
    };
    let before = peek.outer_size().map_err(|e| e.to_string())?;
    // 目标取左右边把手的尺寸（四个方向里最小的那个），最能暴露钳制。
    let target = tauri::LogicalSize::new(7.0, 58.0);
    let t0 = Instant::now();
    let set_err = peek.set_size(target).err();
    let set_ms = t0.elapsed().as_millis();
    // 等系统把窗口消息处理完再读回——set_size 返回不代表尺寸已经落地。
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = peek.outer_size().map_err(|e| e.to_string())?;
    let shrunk = after.width <= 64 && after.height <= 64;
    let msg = format!(
        "resize-back probe: before={}x{} after={}x{} target={}x{} set_ms={} shrunk={} set_err={}",
        before.width, before.height, after.width, after.height,
        target.width, target.height, set_ms, shrunk,
        set_err.as_ref().map(|e| e.to_string()).unwrap_or_else(|| "none".to_string())
    );
    tracing::info!(target: "tum.window", "{}", msg);
    Ok(msg)
}

// ───────────────────────────────────────────────────────────────────────────
// 自动触发：让 release 构建也能无人值守地跑一次测量
// ───────────────────────────────────────────────────────────────────────────

/// 启动时额外跑一次「跨边重建」探针的环境变量名。值为任意非空串即生效。
///
/// 独立审查指出（issue 05 §1.12 的 P0）：`destroy()` 是**异步**的，
/// 而建窗的 label 校验是**同步**的，于是「销毁后立刻建同名窗」在
/// Tauri 2.12.0 下可能必然撞 `a window with label 'peek' already exists`。
/// 修复手段是「轮询等旧窗从窗口管理器里消失（上限 1s）」。
///
/// 这条**不能只靠推理**：跑一次就知道两件事——
/// 1. `immediate=ERR(...)` 证实 P0 成立；
/// 2. `after_wait=OK` 证实修复有效。
///
/// 挂在自动驱动**之后**跑：把手窗是在第一轮 `sync_peek_window("docked")`
/// 里建的，探针必须在它存在之后才能拆。
pub const ENV_REBUILD_PROBE: &str = "TUM_PERF_REBUILD_PROBE";

/// 启动时自动触发一次驱动的环境变量名。值为 `轮数,间隔毫秒`。
///
/// ## 为什么必须有这个入口
///
/// `perf_drive` 是 Tauri command，**只能由前端调用**。而 release 构建不带
/// DevTools 面板，没有一个「在控制台敲一行」的入口——于是这个 command 在
/// release 下等于不存在，§1.4 的测量协议每次都得靠人守着点。
///
/// 有了它，测量就变成一条命令：
///
/// ```powershell
/// $env:TUM_PERF_PROBE='1'; $env:TUM_PERF_DRIVE='30,1000'
/// & 'src-tauri\target\release\token-usage-monitor.exe'
/// ```
///
/// 宿主侧会把「发了指令 → 前端回执」的结果打进日志（target `tum.perf`），
/// 前端侧的 span / LoAF 数据由前端的探针周期落盘，两边各读各的。
pub const ENV_DRIVE: &str = "TUM_PERF_DRIVE";

/// 自动触发前等多久（毫秒）。要等前端 `onMount` 把监听器注册上。
const AUTO_DRIVE_DELAY_MS: u64 = 5_000;

/// 解析 `TUM_PERF_DRIVE` 的值：`轮数,间隔毫秒`。缺省 30 轮 / 1s 间隔。
fn parse_drive_spec(raw: &str) -> (u32, u64) {
    let mut it = raw.split(',');
    let rounds = it
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(30)
        .clamp(1, MAX_ROUNDS);
    let interval_ms = it
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1000)
        .min(MAX_INTERVAL_MS);
    (rounds, interval_ms)
}

/// 启动时按环境变量自动跑一次驱动。**没有设环境变量时什么都不做**（生产零开销）。
pub fn maybe_auto_drive(app: &AppHandle) {
    let Ok(raw) = std::env::var(ENV_DRIVE) else {
        return;
    };
    if raw.trim().is_empty() {
        return;
    }
    let (rounds, interval_ms) = parse_drive_spec(&raw);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(AUTO_DRIVE_DELAY_MS)).await;
        let Some(dash) = app.get_webview_window("dashboard") else {
            tracing::warn!(target: "tum.perf", "auto drive: dashboard window not found");
            return;
        };
        // 先把主窗**真正贴到最近的一条边**。没有这一步，宿主侧的 docked 路径
        // 整条不会执行：current_anchor 在窗口不贴边时返回 None，
        // sync_peek_window 会在第一行 Err 早退，peek 窗口既不建也不定位，
        // 分段计时一行都记不到（实测：30 轮测量零条 tum.window 日志）。
        // 这一步等价于用户把胶囊拖到屏幕边缘松手——它复刻的是真实前置条件。
        match crate::ipc::force_dock_to_nearest(&dash) {
            Some(anchor) => {
                crate::ipc::persist_placement(&app, Some(anchor)).await;
                tracing::info!(
                    target: "tum.perf",
                    side = ?anchor.side,
                    "auto drive: forced dock to nearest edge"
                );
            }
            None => {
                tracing::warn!(target: "tum.perf", "auto drive: cannot resolve a nearest edge");
            }
        }
        // 先挂监听再发指令：反过来的话前端秒回执，Rust 还没准备好收。
        let (tx, rx) = tokio::sync::oneshot::channel::<DriveDone>();
        let listener_id = app.once(PERF_DRIVE_DONE_EVENT, move |event| {
            if let Ok(d) = serde_json::from_str::<DriveDone>(event.payload()) {
                let _ = tx.send(d);
            }
        });
        let req = DriveRequest { rounds, interval_ms };
        if let Err(e) = dash.emit(PERF_DRIVE_EVENT, &req) {
            app.unlisten(listener_id);
            tracing::warn!(target: "tum.perf", error = %e, "auto drive: emit failed");
            return;
        }
        let timeout = drive_timeout(rounds, interval_ms);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(done)) => {
                app.unlisten(listener_id);
                tracing::info!(
                    target: "tum.perf",
                    requested_rounds = rounds,
                    completed_rounds = done.rounds,
                    error = done.error.as_deref().unwrap_or(""),
                    "auto drive finished"
                );
                // 驱动跑完后再验 set_size：**顺序要紧**——把手窗是在第一轮
                // sync_peek_window("docked") 里建的，探针必须在它存在之后跑，
                // 所以不能放在发指令之前。
                match probe_peek_resize(app.clone()).await {
                    Ok(msg) => tracing::info!(target: "tum.perf", "{}", msg),
                    Err(e) => tracing::warn!(target: "tum.perf", error = %e, "resize-back probe failed"),
                }
                // 「跨边重建」探针：验证 P0 的**存在性**与**修复的有效性**。
                // 放在驱动之后——把手窗是驱动第一轮建的，探针必须等它存在。
                if std::env::var(ENV_REBUILD_PROBE)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
                {
                    let side = app
                        .state::<crate::AppState>()
                        .peek_side
                        .lock()
                        .ok()
                        .and_then(|g| *g);
                    match side {
                        Some(side) => {
                            match crate::ipc::probe_peek_rebuild(app.clone(), side).await {
                                Ok(msg) => tracing::info!(target: "tum.perf", "{}", msg),
                                Err(e) => {
                                    tracing::warn!(target: "tum.perf", error = %e, "rebuild probe failed")
                                }
                            }
                            // 探针把把手窗拆了重建：重建出来的窗是 hidden 且
                            // 没定位（build_peek_window 只负责建，不负责摆位）。
                            // 测量跑完就该退出，所以这里显式说一声——
                            // 免得事后把「把手不见了」误当成产品缺陷。
                            tracing::warn!(
                                target: "tum.perf",
                                "rebuild probe left the peek window rebuilt-but-unpositioned; \
                                 restart the app before judging the UI"
                            );
                        }
                        None => {
                            tracing::warn!(target: "tum.perf", "rebuild probe: no recorded peek side")
                        }
                    }
                }
            }
            Ok(Err(_)) => {
                app.unlisten(listener_id);
                tracing::warn!(target: "tum.perf", "auto drive: ack channel closed, frontend did not ack");
            }
            Err(_) => {
                app.unlisten(listener_id);
                tracing::warn!(
                    target: "tum.perf",
                    timeout_s = timeout.as_secs(),
                    "auto drive: timeout waiting for frontend ack"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_parsing_accepts_the_usual_truthy_spellings() {
        assert!(parse_flag("1"));
        assert!(parse_flag(" true "));
        assert!(parse_flag("TRUE"));
        assert!(parse_flag("on"));
        assert!(!parse_flag("0"));
        assert!(!parse_flag(""));
        assert!(!parse_flag("yes"));
    }

    #[test]
    fn timeout_has_a_floor_of_two_minutes() {
        // 30 轮 × 1s = 30s + 30s 余量 = 60s < 下限 → 取 120s。
        assert_eq!(drive_timeout(30, 1000), MIN_TIMEOUT);
    }

    #[test]
    fn timeout_scales_with_rounds_times_interval() {
        // 200 轮 × (1s 间隔 + 1s 每轮额外开销) = 400s + 30s 余量 = 430s。
        assert_eq!(drive_timeout(200, 1000), Duration::from_secs(430));
    }

    #[test]
    fn timeout_covers_the_frontend_cycle_overhead() {
        // 前端一轮 = 唤出 + 320ms 宽限 + 180ms 收起 + 间隔。30 轮 @1s 的实际
        // 耗时约 45s（实测口径），估算必须明显大于它，否则会误报超时。
        let t = drive_timeout(30, 1000);
        assert!(t >= Duration::from_secs(45), "timeout too tight: {t:?}");
        assert_eq!(t, MIN_TIMEOUT);
    }

    #[test]
    fn timeout_is_capped() {
        // 参数写错时也不能无限等。
        assert_eq!(drive_timeout(100_000, 60_000), MAX_TIMEOUT);
    }

    #[test]
    fn timeout_never_overflows() {
        // u64 毫秒 × u64 毫秒会溢出：必须夹住而不是回绕成一个小值。
        let t = drive_timeout(u32::MAX, u64::MAX);
        assert!(t <= MAX_TIMEOUT);
    }

    #[test]
    fn disabled_timer_never_reads_the_clock() {
        let mut seg = Segments::new(false);
        seg.mark_build();
        seg.mark_set_pos();
        seg.mark_show();
        assert_eq!(seg.build.ms, 0);
        assert!(!seg.build.done);
        assert_eq!(seg.total_ms(), 0);
    }

    #[test]
    fn enabled_timer_marks_every_step() {
        let mut seg = Segments::new(true);
        seg.mark_build();
        seg.mark_set_pos();
        seg.mark_show();
        seg.mark_emit();
        assert!(seg.build.done);
        assert!(seg.set_pos.done);
        assert!(seg.show.done);
        assert!(seg.emit.done);
        // 步与步之间没有任何可观测的等待，耗时必然是 0ms（不足 1ms）。
        assert_eq!(seg.build.ms, 0);
    }

    #[test]
    fn done_payload_tolerates_a_missing_error_field() {
        let d: DriveDone = serde_json::from_str(r#"{"rounds": 7}"#).unwrap();
        assert_eq!(d.rounds, 7);
        assert!(d.error.is_none());
    }

    #[test]
    fn done_payload_reads_the_error_string() {
        let d: DriveDone =
            serde_json::from_str(r#"{"rounds": 3, "error": "boom"}"#).unwrap();
        assert_eq!(d.rounds, 3);
        assert_eq!(d.error.as_deref(), Some("boom"));
    }

    #[test]
    fn request_payload_uses_camel_case_field_names() {
        let json = serde_json::to_string(&DriveRequest {
            rounds: 30,
            interval_ms: 1000,
        })
        .unwrap();
        assert_eq!(json, r#"{"rounds":30,"intervalMs":1000}"#);
    }

    #[test]
    fn drive_guard_is_exclusive() {
        let g = DriveGuard::acquire().expect("first acquire");
        assert!(DriveGuard::acquire().is_err());
        drop(g);
        assert!(DriveGuard::acquire().is_ok());
    }

    #[test]
    fn drive_spec_falls_back_to_thirty_rounds() {
        // 空串 / 解析失败 → 缺省 30 轮 × 1s。
        assert_eq!(parse_drive_spec(""), (30, 1000));
        assert_eq!(parse_drive_spec("abc"), (30, 1000));
        assert_eq!(parse_drive_spec("30"), (30, 1000));
    }

    #[test]
    fn drive_spec_reads_rounds_and_interval() {
        assert_eq!(parse_drive_spec("30,1000"), (30, 1000));
        // 允许空格：环境变量里手写很容易带上。
        assert_eq!(parse_drive_spec(" 12 , 250 "), (12, 250));
    }

    #[test]
    fn drive_spec_clamps_out_of_range_values() {
        // 轮数下限 1（0 轮没有意义），上限 MAX_ROUNDS。
        assert_eq!(parse_drive_spec("0,1000").0, 1);
        assert_eq!(parse_drive_spec("999999,1000").0, MAX_ROUNDS);
        // 间隔上限 MAX_INTERVAL_MS。
        assert_eq!(parse_drive_spec("30,999999").1, MAX_INTERVAL_MS);
    }
}
