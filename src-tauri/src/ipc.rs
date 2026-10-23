//! Tauri IPC commands exposed to the frontend.
//!
//! Naming convention: snake_case in Rust, frontend calls via `invoke('get_usage')`.

use crate::build_account_provider;
use crate::dock::{
    anchor_from_rect, anchor_rect, approach_step, nearest_side, peek_rect, pill_span_for, DockAnchor,
    DockSide,
};
use crate::hub::HubDevice;
use crate::local::LocalToolsPayload;
use crate::providers::{
    AccountMeta, Credentials, HeatmapCell, PRESETS, Preset, ProviderState, UsageSnapshot,
};
use crate::settings::Settings;
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WebviewWindowBuilder};

/// A configured account plus its credential/registry state, for the Settings UI.
#[derive(serde::Serialize, Clone)]
pub struct AccountWithInfo {
    #[serde(flatten)]
    pub account: AccountMeta,
    /// Whether the user has saved credentials for this account.
    pub has_credentials: bool,
    /// Whether a polling task is live for this account right now.
    pub live: bool,
}

/// The "provider" catalogue handed to the Settings UI: the built-in presets a
/// user can add + the accounts they've already created.
#[derive(serde::Serialize)]
pub struct ProviderCatalog {
    pub presets: Vec<Preset>,
    pub accounts: Vec<AccountWithInfo>,
}

/// Returns the provider catalogue (presets + configured accounts) for the
/// Settings UI.
#[tauri::command]
pub async fn get_providers(state: State<'_, AppState>) -> Result<ProviderCatalog, String> {
    let settings = state.settings.get().await;
    let registry = state.registry.read().await;
    let live: std::collections::HashSet<String> =
        registry.list().into_iter().map(|p| p.id()).collect();
    drop(registry);
    let accounts = settings
        .accounts
        .iter()
        .map(|a| AccountWithInfo {
            account: a.clone(),
            has_credentials: state.settings.load_credentials(&a.instance_id).is_some(),
            live: live.contains(&a.instance_id),
        })
        .collect();
    Ok(ProviderCatalog {
        presets: PRESETS.to_vec(),
        accounts,
    })
}

/// Returns the latest `UsageSnapshot` for every provider that has one.
#[tauri::command]
pub async fn get_usage(state: State<'_, AppState>) -> Result<Vec<UsageSnapshot>, String> {
    let guard = state.state.read().await;
    Ok(guard
        .values()
        .filter_map(|ps: &ProviderState| ps.snapshot.clone())
        .collect())
}

/// Returns per-provider state including errors, for surfacing failures in UI.
#[tauri::command]
pub async fn get_provider_states(
    state: State<'_, AppState>,
) -> Result<HashMap<String, ProviderState>, String> {
    Ok(state.state.read().await.clone())
}

/// Heatmap data for one provider. Prefer the daily breakdown carried by the
/// latest snapshot when present (the Volcengine native breakdown, or up to 90
/// locally captured days embedded by MiniMax / DeepSeek); otherwise read the
/// local daily rows from SQLite, oldest first.
///
/// The snapshot branch ignores `days` (its window is provider-defined);
/// `days` only limits the SQLite fallback.
#[tauri::command]
pub async fn get_heatmap(
    state: State<'_, AppState>,
    provider_id: String,
    days: u32,
) -> Result<Vec<HeatmapCell>, String> {
    {
        let guard = state.state.read().await;
        if let Some(ps) = guard.get(&provider_id) {
            if let Some(heatmap) = ps.snapshot.as_ref().and_then(|s| s.heatmap.clone()) {
                return Ok(heatmap);
            }
        }
    }
    state
        .storage
        .load_heatmap(&provider_id, days)
        .map_err(|e| e.to_string())
}

/// 统一历史视图数据源：日历/趋势/模型三个页签共用 `usage_daily` 账本。
/// 返回原始行（source/kind/date/model/四分量/成本），前端按维度聚合。
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageHistoryResult {
    pub rows: Vec<crate::storage::UsageDailyRow>,
}

#[tauri::command]
pub async fn get_usage_history(
    state: State<'_, AppState>,
    days: u32,
) -> Result<UsageHistoryResult, String> {
    let days = days.clamp(1, 400);
    let rows = state
        .storage
        .load_usage_daily(None, days)
        .map_err(|e| e.to_string())?;
    Ok(UsageHistoryResult { rows })
}


///

/// Return the aggregated local-tool usage (C8). Scans the supported local AI
/// tools' logs/DBs (Claude Code JSONL + Cherry Studio + MiniMax Code SQLite),
/// then serves a short-lived cache; `force` bypasses the cache for a rescan.
#[tauri::command]
pub async fn get_local_tools(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<LocalToolsPayload, String> {
    if !force.unwrap_or(false) {
        if let Some(cached) = state.local.cached().await {
            return Ok(cached);
        }
        // Second-level (disk) cache: cheap metadata fingerprint; if the sources
        // are unchanged since the last scan, replay the persisted result
        // instead of re-parsing every log file.
        let fp = crate::local::cache::src_fingerprint();
        if let Some((stored_fp, payload_json)) = state.storage.load_local_scan_cache() {
            if stored_fp == fp {
                if let Ok(payload) = serde_json::from_str::<LocalToolsPayload>(&payload_json) {
                    state.local.store(payload.clone()).await;
                    return Ok(payload);
                }
            }
        }
    }

    let tools = crate::local::scan_all(&state.storage);
    // 全量重扫后重建 jsonl 工具的续读指针，保证后续 watch 增量从新增处读、不重复计数。
    crate::local::delta::record_tool_offsets(&state.storage, "claude-code");
    crate::local::delta::record_tool_offsets(&state.storage, "codex");
    let sessions_parsed: u64 = tools.iter().map(|t| t.session_count).sum();
    let payload = LocalToolsPayload {
        tools: tools.clone(),
        sessions_parsed,
    };
    state.local.store(payload.clone()).await;

    // 把全部本机工具的逐日用量写入统一账本 usage_daily（日历/趋势/模型三个
    // 历史视图的唯一口径），全量回放幂等。
    crate::local::persist_all_tools(&state.storage, &tools);

    // Persist the result + source fingerprint for the second-level cache.
    let _ = state.storage.save_local_scan_cache(
        &crate::local::cache::src_fingerprint(),
        &serde_json::to_string(&payload).unwrap_or_default(),
    );
    Ok(payload)
}

/// Metadata for the local device, shown by the Devices view (B8 placeholder).
/// Multi-device sync (hub) is a later phase; for now the page renders this
/// machine as the single "device". Only std env info is used — no new deps.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub pid: u32,
}

#[tauri::command]
pub fn get_device_report() -> DeviceInfo {
    let (device_id, hostname, os, arch, version) = crate::hub::machine_info();
    DeviceInfo {
        device_id,
        hostname,
        os,
        arch,
        version,
        pid: std::process::id(),
    }
}

/// 设备页拉取结果：设备列表 + 可选的刷新/拉取失败原因。
#[derive(Serialize, Clone)]
pub struct HubDevicesResult {
    pub devices: Vec<HubDevice>,
    pub warning: Option<String>,
}

/// All devices known to the hub this instance participates in (self first).
/// Hub/off mode: reads the local hub store (filled by peers via `/ingest`).
/// Agent mode: additionally pulls the remote hub's device list via `GET /devices`
/// so the devices view shows every peer even when this machine only reports up.
/// Wire failures / 401s on the remote fetch are surfaced as `warning` instead of
/// failing the whole request.
#[tauri::command]
pub async fn get_hub_devices(
    state: State<'_, AppState>,
) -> Result<HubDevicesResult, String> {
    let (id, host, os, arch, ver) = crate::hub::machine_info();
    let mut tool_tokens = 0.0;
    let mut tool_count = 0u64;
    if let Some(payload) = state.local.cached().await {
        tool_tokens = payload.tools.iter().flat_map(|t| t.daily.iter()).map(|d| d.total).sum();
        tool_count = payload.tools.len() as u64;
    }
    let provider_count = state.settings.get().await.accounts.len() as u64;
    let usage = crate::hub::tool_usage_from_cache(&state.local).await;
    let self_device = crate::hub::build_device_usage(
        &id, &host, &os, &arch, &ver, tool_tokens, provider_count, tool_count, usage,
    );

    // 候选设备：单机模式（off）不合并任何远端/存储设备——用户关掉多端同步
    // 后，面板上残留的旧设备卡片和"全端汇总"会误导（看起来同步还开着）。
    // 仅 hub/agent/lan 模式拉取；存储行保留（切回多端模式后自动恢复显示）。
    let s = state.settings.get().await;
    let mut warning: Option<String> = None;
    let mut sources: Vec<Vec<HubDevice>> = Vec::new();
    if s.hub_mode != "off" {
        if s.hub_mode == "agent" && !s.hub_base.is_empty() {
            let client = state.http.read().await.clone();
            match crate::hub::fetch_devices(&client, &s.hub_base, &s.hub_token).await {
                Ok(remote) => sources.push(remote),
                Err(e) => warning = Some(format!("远端 hub 拉取失败：{e}")),
            }
        }
        if let Ok(local) = state.storage.list_hub_devices() {
            sources.push(local);
        }
    }

    // 去重合并：本机恒在首位，余下按来源顺序去重（避免重复 device_id）。
    let mut seen = std::collections::HashSet::new();
    seen.insert(self_device.device_id.clone());
    let mut devices = vec![self_device];
    for list in sources {
        for device in list {
            if !seen.contains(&device.device_id) {
                seen.insert(device.device_id.clone());
                devices.push(device);
            }
        }
    }
    Ok(HubDevicesResult { devices, warning })
}

/// 移除一台设备的上报记录。注意：若该设备仍在同步（lan mesh 30s 上报 /
/// agent 周期上报），它会在下一个周期重新出现——本命令主要用于清理改名
/// 残留、已退役机器等"幽灵设备"。本机行由自报维护，不允许移除。
#[tauri::command]
pub async fn remove_hub_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    let (id, _, _, _, _) = crate::hub::machine_info();
    if device_id == id {
        return Err("本机设备无法移除".to_string());
    }
    state
        .storage
        .remove_hub_device(&device_id)
        .map_err(|e| e.to_string())
}

/// 手动添加远端设备：从指定 hub 地址拉取设备列表并入本地存储，返回并入条数。
/// 适合"用量集中在另一台机器，本机想直接看它"的一次性拉取；之后该机器正常
/// 同步时自动刷新。共享密钥沿用设置里的 hub_token（对端启用鉴权时必需）。
#[tauri::command]
pub async fn add_remote_device(
    state: State<'_, AppState>,
    base: String,
) -> Result<usize, String> {
    let base = base.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("hub 地址不能为空".to_string());
    }
    let base = if base.starts_with("http://") || base.starts_with("https://") {
        base
    } else {
        format!("http://{base}")
    };
    let client = state.http.read().await.clone();
    let token = state.settings.get().await.hub_token;
    let devices = crate::hub::fetch_devices(&client, &base, &token)
        .await
        .map_err(|e| e.to_string())?;
    let count = devices.len();
    for d in &devices {
        let _ = state.storage.upsert_hub_device(d);
    }
    Ok(count)
}

/// 当前生效的汇率快照（覆盖 > 缓存 > 内置默认）。缓存陈旧时顺带触发一次
/// 后台刷新；网络失败以 `warning` 返回，不影响快照本身。
#[tauri::command]
pub async fn get_exchange_rates(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<crate::exchange::RatesSnapshot, String> {
    let settings = state.settings.get().await;
    let client = state.http.read().await.clone();
    let snap = crate::exchange::refresh_rates(&state.storage, &settings, &client, false).await;
    // 合并结果（含刚保存的覆盖值）广播给其它窗口，实现跨窗口即时同步
    let _ = app.emit("rates-updated", &snap);
    Ok(snap)
}

/// 强制重新拉取汇率并落库，返回刷新后的合并快照。
#[tauri::command]
pub async fn refresh_exchange_rates(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<crate::exchange::RatesSnapshot, String> {
    let settings = state.settings.get().await;
    let client = state.http.read().await.clone();
    let snap = crate::exchange::refresh_rates(&state.storage, &settings, &client, true).await;
    // 广播给其它窗口同步（设置窗口自己直接消费返回值）
    let _ = app.emit("rates-updated", &snap);
    Ok(snap)
}

/// Trigger an immediate refresh for one provider (or all enabled providers
/// when `provider_id` is None). Runs through the same unified pipeline as
/// periodic polling (burn diff, notifications, UsageUpdate event), and works
/// even while polling is paused.
#[tauri::command]
pub async fn force_refresh(
    state: State<'_, AppState>,
    app: AppHandle,
    provider_id: Option<String>,
) -> Result<(), String> {
    let registry = state.registry.read().await;

    let targets: Vec<_> = match provider_id {
        Some(ref id) => registry
            .get(id)
            .map(|p| vec![p])
            .ok_or_else(|| format!("unknown provider: {id}"))?,
        None => {
            // Refresh only enabled accounts when no specific id is given.
            let enabled: std::collections::HashSet<String> = state
                .settings
                .get()
                .await
                .accounts
                .into_iter()
                .filter(|a| a.enabled)
                .map(|a| a.instance_id)
                .collect();
            registry
                .list()
                .into_iter()
                .filter(|p| enabled.contains(&p.id()))
                .collect()
        }
    };
    drop(registry);

    for provider in targets {
        // Per-provider failures are already surfaced via the provider-error
        // event inside poll_one; keep refreshing the remaining targets.
        if let Err(e) = crate::scheduler::poll_one(&app, &provider).await {
            tracing::warn!(target: "tum.poll", provider = %provider.id(), error = %e,
                "manual refresh failed");
        }
    }
    Ok(())
}

/// Pause/resume background polling. Returns the effective new state.
/// Manual `force_refresh` keeps working while paused.
#[tauri::command]
pub async fn toggle_polling(
    state: State<'_, AppState>,
    paused: bool,
) -> Result<bool, String> {
    state.pause_tx.send_replace(paused);
    Ok(*state.pause_tx.borrow())
}

fn clamp_rect(
    x: f64, y: f64, w: f64, h: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
    margin: f64,
) -> (f64, f64) {
    let max_x = (area_x + area_w - w - margin).max(area_x + margin);
    let max_y = (area_y + area_h - h - margin).max(area_y + margin);
    (x.clamp(area_x + margin, max_x), y.clamp(area_y + margin, max_y))
}

pub fn clamp_window_to_work_area(window: &WebviewWindow, margin_logical: f64) -> bool {
    let Ok(pos) = window.outer_position() else { return false; };
    let Ok(size) = window.outer_size() else { return false; };
    let Ok(Some(monitor)) = window.current_monitor() else { return false; };
    let area = monitor.work_area();
    let margin = margin_logical * f64::from(monitor.scale_factor());
    let (nx, ny) = clamp_rect(
        f64::from(pos.x),
        f64::from(pos.y),
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
        margin,
    );
    let changed = nx != f64::from(pos.x) || ny != f64::from(pos.y);
    if changed {
        let _ = window.set_position(PhysicalPosition::new(
            nx.round() as i32,
            ny.round() as i32,
        ));
    }
    changed
}

/// 胶囊距离屏幕边多少逻辑像素内算「已贴边」。判定为贴边后 Rust 会**立刻**把窗口
/// 贴死到边缘（anchor_rect），所以这个阈值同时也是「松手瞬间胶囊瞬移多远」的上限。
///
/// 原值 40 太宽：胶囊离边缘还有 40px（约自身宽度的 1/4）时就被吸过去，那一下瞬移
/// 没有任何过渡，读起来就是「凭空被拽走」——生硬感的来源之一。
/// 收到 16px：仍然是明确的「贴到边了」，瞬移距离小到读不出来。
///
/// 与前端无关：落点判定只在这里做（`nearest_side`），前端只报拖拽位移。
const DOCK_SNAP_MARGIN_PX: f64 = 16.0;

/// 读出胶囊当前**已贴边**时的锚点；离四条边都超过阈值时返回 `None`（浮动）。
///
/// 语义说明（这是一次有意的行为变化，不是缺陷）：判据是「离边够近」，
/// 而不是「前端此刻认为它是 docked」。`set_window_mode` 是 Rust 命令、
/// 拿不到前端的 `pillDocked`，而 `pillDocked` 本身就是拖拽松手那一刻用
/// 同一套阈值判出来的。后果是：一个恰好停在边缘 30px 处的浮动胶囊，
/// 展开成主窗时会被吸附到那条边。我认为这符合直觉（用户把它放在那儿，
/// 多半就是想靠边），但**要写在这里**，以免日后被当成 bug 重新「修」回去。
pub fn current_anchor(window: &WebviewWindow) -> Option<DockAnchor> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitor = window.current_monitor().ok()??;
    let area = monitor.work_area();
    let (ax, ay) = (f64::from(area.position.x), f64::from(area.position.y));
    let (aw, ah) = (f64::from(area.size.width), f64::from(area.size.height));
    let side = nearest_side(
        f64::from(pos.x), f64::from(pos.y),
        f64::from(size.width), f64::from(size.height),
        ax, ay, aw, ah,
        DOCK_SNAP_MARGIN_PX * f64::from(monitor.scale_factor()),
        None,
    )?;
    Some(anchor_from_rect(f64::from(pos.x), f64::from(pos.y), ax, ay, side))
}

/// 探针专用：把主窗**强制**贴到最近的一条边，并返回该锚点。
///
/// 等价于用户把胶囊拖到屏幕边缘松手——但不需要真人操作。
///
/// ## 为什么需要它（issue 05 实测踩到的坑）
///
/// [`current_anchor`] 的语义是「读出**已贴边**时的锚点」：离四条边都超过
/// [`DOCK_SNAP_MARGIN_PX`] 时返回 `None`（浮动）。而 `sync_peek_window` 开头
/// 就拿它当闸门：
///
/// ```text
/// let Some(anchor) = current_anchor(&dash) else {
///     return Err("dashboard window has no monitor".to_string());
/// };
/// ```
///
/// （标 `text` 而不是 `rust`：这段是**示意**，`current_anchor` 与 `dash` 都来自
/// 调用方上下文，标成 `rust` 会被 cargo 当 doctest 真去编译，然后报
/// `cannot find function current_anchor` ——这不是代码的问题，是示例的问题。）
///
/// 于是「窗口不贴边」时，宿主侧**整条 docked 路径都不会被执行**：同步直接
/// Err 返回，前端 catch 掉，peek 窗口既不建也不定位，分段计时也一行都记不到。
/// 实测中这表现为一整轮 30 次测量**零条** `tum.window` 日志——数据看着像
/// 「宿主侧很快」，实际是「宿主侧什么都没做」。
///
/// 测量要覆盖真实的贴边路径，就必须先把窗口**真的贴到边**。本函数用一个
/// 大到不设限的阈值调 [`nearest_side`]，无视贴边阈值选出最近的一条边，
/// 再用 [`apply_anchor`] 把它贴死。
///
/// ⚠️ 副作用：窗口会移动到屏幕边缘，且锚点需要调用方持久化（见
/// [`persist_placement`]）。这是**有意的**——它复刻的正是「用户把胶囊拖到边」
/// 这个真实前置条件，而不是在伪造一条不存在的路径。
pub fn force_dock_to_nearest(window: &WebviewWindow) -> Option<DockAnchor> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitor = window.current_monitor().ok()??;
    let area = monitor.work_area();
    let (ax, ay) = (f64::from(area.position.x), f64::from(area.position.y));
    let (aw, ah) = (f64::from(area.size.width), f64::from(area.size.height));
    // 阈值给足：让「最近边」的判定不受贴边阈值限制。
    let side = nearest_side(
        f64::from(pos.x),
        f64::from(pos.y),
        f64::from(size.width),
        f64::from(size.height),
        ax,
        ay,
        aw,
        ah,
        1.0e9,
        None,
    )?;
    let anchor = anchor_from_rect(f64::from(pos.x), f64::from(pos.y), ax, ay, side);
    apply_anchor(window, anchor);
    Some(anchor)
}

/// 把窗口落到锚点上。`anchor_rect` 保证贴边轴永远贴死且不出屏，
/// 所以窗口尺寸怎么变都不会「掉出贴边态」。
pub fn apply_anchor(window: &WebviewWindow, a: DockAnchor) -> bool {
    let Ok(pos) = window.outer_position() else { return false };
    let Ok(size) = window.outer_size() else { return false };
    let Ok(Some(monitor)) = window.current_monitor() else { return false };
    let area = monitor.work_area();
    let (nx, ny) = anchor_rect(
        a,
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    );
    if nx == f64::from(pos.x) && ny == f64::from(pos.y) {
        return false;
    }
    let _ = window.set_position(PhysicalPosition::new(nx.round() as i32, ny.round() as i32));
    true
}

/// [`apply_anchor`] 的滑动版：目标矩形算法一致，把一次性 set_position 换成
/// glide_window_to。返回语义与 apply_anchor 一致（true = 需要落盘）。
///
/// 拖拽中不滑动：拖拽期是 OS 拉拽与吸附的实时博弈，必须瞬时；滑动只属于
/// 「松手落位」（sync_peek_window docked）路径。
pub fn apply_anchor_glide(window: &WebviewWindow, a: DockAnchor) -> bool {
    let Ok(pos) = window.outer_position() else { return false };
    let Ok(size) = window.outer_size() else { return false };
    let Ok(Some(monitor)) = window.current_monitor() else { return false };
    let area = monitor.work_area();
    let (nx, ny) = anchor_rect(
        a,
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    );
    if nx == f64::from(pos.x) && ny == f64::from(pos.y) {
        return false;
    }
    glide_window_to(
        window.clone(),
        (nx.round() as i32, ny.round() as i32),
        &GLIDE_GEN_MAIN,
        Some("main"),
    );
    true
}

/// 回写「贴边锚点 + 自由位置」到 `config.toml`，让下次启动回到同一形态。
///
/// 与 `apply_anchor` 分开是刻意的：几何函数保持同步纯函数，回写是**副作用**，
/// 交给调用方在落位成功后决定何时做。`apply_anchor` 返回 false（位置本就没动）
/// 时调用方可以跳过这次写盘。
///
/// 两个字段必须**一起**写。贴边态的位置是从锚点反算的（不是用户拖出来的坐标），
/// 浮动态则没有锚点可依；只写其一会留下半新半旧的状态——例如退出时胶囊已浮到
/// 屏幕正中，配置里却还留着上次贴右的锚点，下次启动就被拉回右缘。
///
/// `anchor == None` 即浮动态，此时把当前位置存进 `dashboard_x/y`——浮动态没有
/// 锚点可依，下次启动要靠这两个字段回到原处（`settings::Settings::get_xy_now()`
/// 读的就是它们）。贴边态**不写**这两个字段：几何由锚点反算，而落位滑动期间
/// 读 live 位置会拿到半路过渡坐标。
///
/// ⚠️ 这层是「读缓存 → 改 → 写文件」的**读改写**，必须串行：两个来源同时
/// 落盘时，后写的那份可能带着过期字段把先写的覆盖掉。锁在这里拿，
/// 落盘本身在 [`persist_placement_inner`] 里。
pub async fn persist_placement(app: &AppHandle, anchor: Option<DockAnchor>) {
    let _guard = persist_lock().lock().await;
    persist_placement_inner(app, anchor).await;
}

/// 落盘用的串行锁。
///
/// 用 `OnceLock` 而不是 `LazyLock`：tokio 的 `Mutex` 构造不需要上下文，
/// 但 `OnceLock` 免去「static 里放 tokio 类型」的初始化顺序顾虑。
fn persist_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// 落盘的真正实现（调用方需已持有 [`persist_lock`]）。
async fn persist_placement_inner(app: &AppHandle, anchor: Option<DockAnchor>) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let settings = state.settings.clone();
    let mut next = settings.get().await.clone();
    let mut changed = next.dock != anchor;
    // 贴边态几何由锚点反算，dashboard_x/y 只在浮动态有意义；滑动落位期间读
    // live 位置会拿到半路过渡坐标（apply_anchor_glide 正把窗口滑向目标），
    // 而浮动态启动恢复会用到该字段（get_xy_now）。所以只在浮动态读、写它。
    let live_pos = if anchor.is_none() {
        app.get_webview_window("dashboard")
            .and_then(|w| w.outer_position().ok())
    } else {
        None
    };
    if let Some(pos) = live_pos {
        if next.dashboard_x != Some(pos.x) || next.dashboard_y != Some(pos.y) {
            next.dashboard_x = Some(pos.x);
            next.dashboard_y = Some(pos.y);
            changed = true;
        }
    }
    if !changed {
        return;
    }
    next.dock = anchor;
    if let Err(e) = settings.save(next).await {
        tracing::warn!(target: "tum.window", "persist placement failed: {e}");
    }
}

/// 把落盘挪到后台，**不挡在调用方返回前面**。
///
/// ## 为什么不能挡
///
/// `settings.save` 走**同步**文件写（tmp + rename），Windows 上可能十几到
/// 上百毫秒（杀软扫、文件被占用）。它一旦排在「窗口已经改完尺寸」之后，
/// 命令的返回就被拖到几十毫秒之后——而前端要等命令返回才翻 `mode`，
/// 那段时间里前端还在渲染**旧形态**。于是用户看到「全窗 dashboard 挤在
/// 168 宽的窄窗里」，过一会儿才恢复正常（实测：缩回胶囊形态时偶尔可见，
/// 像是延时）。
///
/// 而且它只在**真的会改到配置**时才发生：贴边态切模式要重新贴边 → 位置变了
/// → 落盘；浮动态位置不变 → `changed = false` → **根本不写盘** → 就看不到
/// 这个现象。这正好解释了「为什么只有收起成把手时才会出现」。
///
/// ## 为什么位置晚几十毫秒写盘没关系
///
/// 写的是「上次停在哪」，晚写几十毫秒没有任何可感知的副作用；而挡住返回
/// 是直接可见的。所以它必须挪到后台。
///
/// ## 串行
///
/// 落盘是「读缓存 → 改 → 写文件」的读改写。多个来源同时落盘时，后写的那份
/// 可能带着过期字段把先写的覆盖掉。`persist_lock` 保证它们串行。
pub fn persist_placement_detached(app: &AppHandle, anchor: Option<DockAnchor>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = persist_lock().lock().await;
        persist_placement_inner(&app, anchor).await;
    });
}

/// 把手窗的物理位置。
/// 位置**从锚点算**，不再读 dashboard 的当前位置——锚点是唯一真值，
/// 胶囊在滑出动画里怎么动都不会带着把手抖。
fn peek_placement(
    window: &WebviewWindow,
    side: DockSide,
    along: i32,
) -> Option<PhysicalPosition<i32>> {
    let Ok(Some(monitor)) = window.current_monitor() else { return None; };
    let area = monitor.work_area();
    let scale = f64::from(monitor.scale_factor());
    let (vis_w, vis_h) = side.peek_size();
    let thickness = vis_w * scale;
    let length = vis_h * scale;
    // 把手窗可能已被系统最小尺寸撑大：按真实尺寸定位，把垂直于贴边轴的
    // 多余部分推出屏幕（否则那一整条透明区域会持续吞掉桌面上的鼠标事件）。
    let actual = window
        .app_handle()
        .get_webview_window("peek")
        .and_then(|p| p.outer_size().ok())
        .map(|s| (f64::from(s.width), f64::from(s.height)));
    let (aw, ah) = actual.unwrap_or((thickness, length));
    let (aw, ah) = (aw.max(thickness), ah.max(length));
    let (px, py) = peek_rect(
        side,
        f64::from(along),
        // 沿边方向的胶囊尺寸随边翻转：左右边取高、上下边取宽。硬编码成
        // PILL_ROW_H 会让贴上/下边时把手与胶囊横向错开 56px。
        pill_span_for(side) * scale,
        thickness,
        length,
        aw,
        ah,
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    );
    Some(PhysicalPosition::new(px.round() as i32, py.round() as i32))
}

/// 让把手跟随 Dashboard 胶囊的纵向位置。把手尚未创建时为空操作。
///
/// 从锚点算而非读胶囊当前位置：胶囊在滑出动画里怎么动都不会带着把手抖。
pub fn reposition_peek(window: &WebviewWindow) {
    let Some(peek) = window.app_handle().get_webview_window("peek") else { return; };
    let Some(anchor) = current_anchor(window) else { return };
    let Some(pos) = peek_placement(window, anchor.side, anchor.along) else { return };
    if peek.outer_position().map(|cur| cur == pos).unwrap_or(false) {
        return;
    }
    let dragging = window
        .app_handle()
        .try_state::<AppState>()
        .map(|s| s.pill_drag.load(Ordering::SeqCst))
        .unwrap_or(false);
    if dragging {
        let _ = peek.set_position(pos);
    } else {
        glide_window_to(peek, (pos.x, pos.y), &GLIDE_GEN_PEEK, None);
    }
}

/// 滑动动画代际：主窗与把手窗各一个计数器。新动画使旧线程退出，
/// 避免两个线程互抢 set_position。
static GLIDE_GEN_MAIN: AtomicU64 = AtomicU64::new(0);
static GLIDE_GEN_PEEK: AtomicU64 = AtomicU64::new(0);

const GLIDE_STEP_MS: u64 = 8; // glance：拖拽/跟随节拍
const GLIDE_TAU_MS: f64 = 60.0; // glance：跟随时间常数
// 600ms 封顶 = τ=60ms 的 10 个时间常数：残余位移 e^-10 ≈ 4.5e-5，
// 即使 5000px 的大位移也收在 0.23px 内，必由 <0.5px 吸附规则落地。
// （50 步/400ms 只够 393px 内落地：7.5·ln(2·gap) 步起才进吸附区。）
const GLIDE_MAX_STEPS: u32 = 75;

/// 让窗口从当前位置指数趋近 `to`（物理像素）。不阻塞调用方；
/// 短命线程 + 8ms 步进；期间同窗新请求会取消旧动画（代际计数）。
///
/// `label` 只给主窗传 `Some("main")`：一次落位也就 1~2 条日志，噪声可忽略，
/// 而「贴边卡一下」（issues/05 §1.7）正是这类日志要观测的现象。把手窗每次
/// 落位要滑 26~37 步，逐条打只会把真正有用的行淹掉，故传 `None`。
fn glide_window_to(
    window: WebviewWindow,
    to: (i32, i32),
    gen: &'static AtomicU64,
    label: Option<&'static str>,
) {
    // 即使目标等于当前位置也推进代际：这一步的目的是让「在跑的同窗旧动画」
    // 停下（旧线程下一帧发现代际变了就退出），避免两条线程交替写同一窗口。
    let my_gen = gen.fetch_add(1, Ordering::SeqCst) + 1;
    let Ok(from) = window.outer_position() else { return };
    if (from.x, from.y) == to {
        return;
    }
    std::thread::spawn(move || {
        if let Some(label) = label {
            tracing::debug!(
                target: "tum.window",
                label = label,
                gap_x = to.0 - from.x,
                gap_y = to.1 - from.y,
                target_x = to.0,
                target_y = to.1,
                "glide start"
            );
        }
        let started = std::time::Instant::now();
        let mut frames = 0u32;
        let mut cur = (f64::from(from.x), f64::from(from.y));
        for _ in 0..GLIDE_MAX_STEPS {
            if gen.load(Ordering::SeqCst) != my_gen {
                glide_end(label, "cancelled", frames, started); // 已被同窗新动画取代
                return;
            }
            std::thread::sleep(Duration::from_millis(GLIDE_STEP_MS));
            let (nx, d1) = approach_step(
                cur.0,
                f64::from(to.0),
                GLIDE_STEP_MS as f64,
                GLIDE_TAU_MS,
            );
            let (ny, d2) = approach_step(
                cur.1,
                f64::from(to.1),
                GLIDE_STEP_MS as f64,
                GLIDE_TAU_MS,
            );
            cur = (nx, ny);
            let _ = window.set_position(tauri::PhysicalPosition::new(
                nx.round() as i32,
                ny.round() as i32,
            ));
            frames += 1;
            if d1 && d2 {
                glide_end(label, "settled", frames, started);
                return;
            }
        }
        glide_end(label, "capped", frames, started);
    });
}

/// 主窗滑动终态日志：三种出口（settled / cancelled / capped）共用一条消息，
/// 靠 `reason` 区分，`frames` 是实际步数。`label == None` 时静默（把手窗）。
fn glide_end(
    label: Option<&'static str>,
    reason: &'static str,
    frames: u32,
    started: std::time::Instant,
) {
    let Some(label) = label else { return };
    tracing::debug!(
        target: "tum.window",
        label = label,
        reason = reason,
        frames = frames,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "glide end"
    );
}

/// Snap the window to the nearest screen edge when it is dragged within
/// `margin_logical` of one. Called from the dashboard's `Moved` window event.
pub fn snap_to_edges(window: &WebviewWindow, margin_logical: f64) -> bool {
    // 拖拽贴边是瞬时几何改写；先作废在跑的主窗滑动，否则滑动线程下一帧
    // 会把窗口写回按旧几何算的目标。（dashboard 态目前没有主窗 glide，
    // 但补上这一行可以让「所有几何写入者都推进代际」这条协议没有例外。）
    GLIDE_GEN_MAIN.fetch_add(1, Ordering::SeqCst);
    let Ok(pos) = window.outer_position() else { return false; };
    let Ok(size) = window.outer_size() else { return false; };
    let Ok(Some(monitor)) = window.current_monitor() else { return false; };
    let area = monitor.work_area();
    let margin = margin_logical * f64::from(monitor.scale_factor());
    let mut nx = f64::from(pos.x);
    let mut ny = f64::from(pos.y);
    let w = f64::from(size.width);
    let h = f64::from(size.height);
    let (ax, ay) = (f64::from(area.position.x), f64::from(area.position.y));
    let (aw, ah) = (f64::from(area.size.width), f64::from(area.size.height));

    // Snap horizontally: near the left edge, or (window's right edge) near the
    // work area's right edge.
    if (nx - ax).abs() <= margin {
        nx = ax;
    } else if ((ax + aw) - (nx + w)).abs() <= margin {
        nx = ax + aw - w;
    }
    // Snap vertically: near the top edge, or (window's bottom edge) near the
    // work area's bottom edge.
    if (ny - ay).abs() <= margin {
        ny = ay;
    } else if ((ay + ah) - (ny + h)).abs() <= margin {
        ny = ay + ah - h;
    }

    if nx != f64::from(pos.x) || ny != f64::from(pos.y) {
        let _ = window.set_position(PhysicalPosition::new(
            nx.round() as i32,
            ny.round() as i32,
        ));
        return true;
    }
    false
}

/// 前端拖拽松手时问一次「这次该贴哪条边」。判定逻辑只有一份（在 dock.rs 的
/// `nearest_side`），前端不再自己算——旧实现前端算 min(距左,距右)、Rust 算
/// 中心比较，两套口径迟早对不上。
#[tauri::command]
pub async fn dock_side_of(
    window: tauri::Window,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    drag_x: Option<f64>,
    drag_y: Option<f64>,
) -> Result<Option<String>, String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor".to_string())?;
    let area = monitor.work_area();
    let drag = match (drag_x, drag_y) {
        (Some(dx), Some(dy)) => Some((dx, dy)),
        _ => None,
    };
    Ok(nearest_side(
        x, y, w, h,
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
        DOCK_SNAP_MARGIN_PX * f64::from(monitor.scale_factor()),
        drag,
    )
    .map(|s| s.as_str().to_string()))
}

/// Switch the dashboard window between full and compact modes.
///
/// 关键：换尺寸**必须同时换位置**，且位置由锚点算。旧实现只 `set_size`
/// 不 `set_position`——Win32 保持左上原点，于是贴右的胶囊展开成 400 宽
/// 后右侧溢出 232px 出屏，随后的 clamp 把它拉到「留 8px 边距」处，
/// 再收回胶囊时它就停在离右边 240px 的地方，不再贴边。
#[tauri::command]
pub async fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    use tauri::LogicalSize;

    let window = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    let compact = app.state::<AppState>().compact_mode.clone();

    // 模式切换是瞬时几何改写：作废在跑的滑动，否则旧线程下一帧会按旧尺寸把
    // 窗口写回旧目标（贴右 168 宽 → 切 400 宽后会右侧探出屏幕）。
    // 不能改成挂在 Resized 事件上：t=180ms 的 collapsePill 缩窗不需要重新锚定，
    // 挂上去会把正常落位截停在半路。
    GLIDE_GEN_MAIN.fetch_add(1, Ordering::SeqCst);

    // 改尺寸**之前**读锚点：旧尺寸才是胶囊的贴边尺寸。
    let anchor = current_anchor(&window);

    match mode.as_str() {
        "dashboard" => {
            window
                .set_size(LogicalSize::new(400u32, 680u32))
                .map_err(|e| e.to_string())?;
            // 全窗态是普通窗口：恢复鼠标交互，贴边把手在此模式下没有意义。
            let _ = window.set_ignore_cursor_events(false);
            if let Some(peek) = app.get_webview_window("peek") {
                let _ = peek.close();
                // 窗口没了，边记录也必须跟着清——否则下次建窗会被误判成
                // 「同边可复用」，而实际上尺寸是按上一条边烧死的。
                set_peek_side(&app, None);
            }
            compact.store(false, Ordering::SeqCst);
            match anchor {
                // 贴着边来的：按同一锚点重算 400×680 的位置，仍然贴死且不出屏。
                Some(a) => { apply_anchor(&window, a); }
                // 浮动来的：只把越界的位置拉回屏内，不强行贴边。
                None => { clamp_window_to_work_area(&window, 8.0); }
            }
        }
        "compact" => {
            // 迷你胶囊 168x56：主行 = 圆环 + 百分比 + 分隔线 + 品牌芯片。
            window
                .set_size(LogicalSize::new(168u32, 56u32))
                .map_err(|e| e.to_string())?;
            compact.store(true, Ordering::SeqCst);
            // 常态浮动：胶囊停在原处，只把越界的位置拉回屏内 —— 不再强制贴边；
            // 只有用户把它拖到屏幕边缘松手，才由 sync_peek_window("docked") 贴死。
            match anchor {
                Some(a) => { apply_anchor(&window, a); }
                None => { clamp_window_to_work_area(&window, 8.0); }
            }
        }
        other => return Err(format!("unknown window mode: {other}")),
    }

    // 浮动位置也要持久化：dashboard_x/y 只在浮动态有意义（贴边态几何由锚点
    // 反算），但浮动态的启动恢复确实读这两个字段（get_xy_now），所以这里必须
    // 写；锚点一并落盘，免得切完模式后配置里还留着上一次的形态。
    //
    // ⚠️ 必须**后台**落盘：这一步是同步写盘（tmp + rename），可能几十毫秒。
    // 它排在「窗口已经缩到 168 宽」之后，挡住返回就意味着「命令返回前前端
    // 一直渲染着旧形态」——用户会看到全窗 dashboard 挤在窄窗里（实测）。
    // 详见 [`persist_placement_detached`]。
    persist_placement_detached(&app, current_anchor(&window));

    Ok(())
}

/// 把前端性能探针的当前读数写到应用数据目录（方案 issues/05 §1）。
///
/// **为什么需要它**：探针数据只活在 WebView 的 JS 上下文里，宿主侧读不到；
/// release 构建又不带 DevTools 面板。让人在控制台手敲命令既容易错、也不可复现。
/// 探针开启时前端周期性调这个命令，测量就变成「应用自己写文件」，
/// 剩下的分析在宿主侧做。
///
/// ⚠️ 仅在 `VITE_PERF=1`（或 dev）构建下才会被前端调用；命令本身始终存在，
/// 但写出的文件名带 `perf-probe`，不会与任何真实数据混淆。
#[tauri::command]
pub async fn write_perf_probe(app: AppHandle, json: String) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("perf-probe.json");
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 当前设置的语言（窗口标题等 Rust 侧文案用）。读不到应用状态（测试桩）
/// 或值非法时按默认中文。
pub(crate) fn current_lang(app: &AppHandle) -> crate::i18n::Lang {
    app.try_state::<AppState>()
        .map(|s| crate::i18n::resolve(&s.settings.read_blocking().language))
        .unwrap_or(crate::i18n::Lang::ZhCn)
}

/// 记录把手窗当前**属于哪条边**。
///
/// 用途只有一处：复用前判断「能不能复用」。把手窗的尺寸在建窗时按边烧死
/// （左右 7×58 / 上下 58×7），跨边复用会让定位按错误尺寸算——那正是
/// 5bba9c6 的失败形态。所以同边复用、跨边重建。
fn set_peek_side(app: &AppHandle, side: Option<DockSide>) {
    if let Ok(mut g) = app.state::<AppState>().peek_side.lock() {
        *g = side;
    }
}

/// 新建贴边把手窗。尺寸按边烧死（左右 7×58 / 上下 58×7）。
///
/// 把手页通过初始化脚本拿到所在边，决定内容对齐与圆角朝向。
fn build_peek_window(app: &AppHandle, side: DockSide) -> Result<WebviewWindow, String> {
    let (pw, ph) = side.peek_size();
    let built = WebviewWindowBuilder::new(
        app,
        "peek",
        tauri::WebviewUrl::App("peek.html".into()),
    )
    .title(crate::i18n::window_title(
        current_lang(app),
        crate::i18n::WindowPage::PeekHandle,
    ))
    .inner_size(pw, ph)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .visible(false)
    .initialization_script(format!("window.__PEEK_SIDE__ = {:?};", side.as_str()))
    .build()
    .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&built);
    Ok(built)
}

/// 探针专用：实测「销毁后立刻建同名窗」到底会不会失败。
///
/// ## 为什么要有这个探针
///
/// 独立审查指出：`destroy()` 是**异步**的（只往事件循环投一条 Destroy 消息），
/// 而建窗的 label 校验是**同步**的（`prepare_window` 里
/// `windows_lock().contains_key(&pending.label)`）。于是「destroy 之后立刻
/// build 同名窗」在 Tauri 2.12.0 下**可能必然撞**
/// `a window with label 'peek' already exists`——旧窗还没从管理器里摘掉，
/// 新窗建不出来。
///
/// 这条**不能只靠推理**：直接跑一次就知道，而且它决定「跨边重建」这条路
/// 到底能不能走。本探针把两种做法都跑一遍：
///
/// 1. destroy → **立刻** build（预期失败）
/// 2. destroy → 等旧窗从管理器消失 → build（预期成功）
///
/// 跑完会重建一个把手窗，并把边记录同步好，不污染后续测量。
pub(crate) async fn probe_peek_rebuild(app: AppHandle, side: DockSide) -> Result<String, String> {
    if !crate::perf_probe::enabled() {
        return Err("探针未启用（PERF_PROBE 关闭）".to_string());
    }
    let Some(old) = app.get_webview_window("peek") else {
        return Err("peek window not found（把手窗还没建）".to_string());
    };
    let _ = old.destroy();
    // 第 1 遍：destroy 之后**立刻**建。
    let immediate = match build_peek_window(&app, side) {
        Ok(_) => "OK".to_string(),
        Err(e) => format!("ERR({e})"),
    };
    // 第 2 遍：先确保「当前没有同名窗」，再走「等它消失 → build」。
    //
    // 第 1 遍如果**建成功了**，此刻管理器里有一个活的同名窗——直接再 build
    // 必然撞 label 冲突，那测到的就不是「等一等能不能建回来」，而是
    // 「同名窗还在」，量纲错了。所以把第 1 遍的产物也一并拆掉，
    // 两遍才可比。
    if let Some(w) = app.get_webview_window("peek") {
        let _ = w.destroy();
    }
    let mut waited_ms = 0u32;
    while app.get_webview_window("peek").is_some() && waited_ms < 1000 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        waited_ms += 10;
    }
    let after_wait = match build_peek_window(&app, side) {
        Ok(_) => "OK".to_string(),
        Err(e) => format!("ERR({e})"),
    };
    // 探针结束时保证「窗口存在 + 边记录一致」，不污染后续测量。
    set_peek_side(&app, app.get_webview_window("peek").is_some().then_some(side));
    let msg = format!(
        "rebuild probe: immediate={immediate} after_wait_{waited_ms}ms={after_wait}"
    );
    tracing::info!(target: "tum.window", "{}", msg);
    Ok(msg)
}

/// 取得（必要时重建）贴边把手窗。
///
/// - **同边**：复用现有窗口，**零建窗**。这是高频路径——反复贴边/唤出都在同一条边。
/// - **跨边或首次**：销毁旧窗 → **等它真的从窗口管理器里消失** → 建新窗。
///
/// ## 为什么跨边必须「等」
///
/// 把手窗的尺寸在建窗时按边烧死（左右 7×58 / 上下 58×7），而系统最小尺寸
/// 还会把它撑到约 136×39。跨边复用会让 `peek_placement` 按**错误的尺寸**算位置
/// ——那正是 5bba9c6 的失败形态（点击胶囊放大/变回胶囊失效）。
///
/// 而「销毁后立刻建同名窗」是**必然失败**的：
///
/// - `destroy()` 是**异步**的：只往事件循环投一条 Destroy 消息就返回；
///   窗口真正从管理器里摘掉（`manager.on_window_close` → `map.remove`）
///   要等事件循环处理完消息、销毁 HWND 与 WebView2 之后。
/// - 建窗的 label 校验是**同步**的：`prepare_window` 里
///   `windows_lock().contains_key(&pending.label)`，排在任何主线程往返之前。
///
/// 于是 destroy 与 build 之间只隔几微秒，而旧窗拆除要走一趟跨进程的
/// WebView2 IPC（毫秒量级）——**必然撞上
/// `a window with label 'peek' already exists`**，旧窗被销毁却建不回来。
/// 所以必须轮询等旧窗消失（上限 1s：正常几毫秒就够，超时不至于挂住调用方）。
async fn acquire_peek_window(
    app: &AppHandle,
    side: DockSide,
    seg: &mut crate::perf_probe::Segments,
) -> Result<WebviewWindow, String> {
    let existing = app.get_webview_window("peek");
    let recorded = app
        .state::<AppState>()
        .peek_side
        .lock()
        .ok()
        .and_then(|g| *g);
    match existing {
        Some(w) if crate::dock::peek_reuse_decision(recorded, side) => {
            // 同边：只是一次查表，build_ms 会很小。
            seg.mark_build();
            Ok(w)
        }
        Some(w) => {
            tracing::info!(
                target: "tum.window",
                recorded = ?recorded,
                current = ?side,
                "peek side changed: rebuild instead of reuse"
            );
            let _ = w.destroy();
            set_peek_side(app, None);
            // 等旧窗从窗口管理器里消失（见上面「为什么跨边必须等」）。
            let mut waited_ms = 0u32;
            while app.get_webview_window("peek").is_some() && waited_ms < 1000 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                waited_ms += 10;
            }
            if app.get_webview_window("peek").is_some() {
                tracing::warn!(
                    target: "tum.window",
                    waited_ms,
                    "old peek window still present after destroy; rebuild may fail"
                );
            }
            let built = build_peek_window(app, side)?;
            // 建窗（含 WebView2 初始化）——判读表第 1 行的关键数字：
            // 它若相对 peekLead 的 span 时长并不显著，「建窗阻塞导致主窗卡顿」
            // 这个假设就不成立，§2.1 整条分支该被否决。
            seg.mark_build();
            seg.peek_created = true;
            set_peek_side(app, Some(side));
            Ok(built)
        }
        None => {
            let built = build_peek_window(app, side)?;
            seg.mark_build();
            seg.peek_created = true;
            set_peek_side(app, Some(side));
            Ok(built)
        }
    }
}

/// 创建/定位贴边把手，并把「谁捕获鼠标」交给本命令统一托管（三态）：
/// `floating`（常态浮动）→ 销毁把手、主窗可交互、不移动主窗；
/// `revealed`（胶囊已贴边滑入）→ 把手存在但不捕获、主窗捕获、不移动主窗；
/// `docked`（胶囊收起贴边）→ 先贴死边缘，再让把手捕获鼠标、主窗穿透。
/// 返回胶囊贴靠的边，供前端决定滑入方向。
#[tauri::command]
pub async fn sync_peek_window(app: AppHandle, state: String) -> Result<String, String> {
    let dash = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;
    // 边与沿边坐标**先算出来并落位**，再拿去建/放把手。旧实现先按胶囊当前位置
    // 推断边、再 dock，于是把手拿到的边可能与最终贴定的边不一致。
    let Some(anchor) = current_anchor(&dash) else {
        return Err("dashboard window has no monitor".to_string());
    };
    let side = anchor.side;
    let side_str = side.as_str();

    // 只在 compact 胶囊态才有把手；dashboard 态（可能是模式切换竞态）直接
    // 返回所在边，不重建刚被 set_window_mode 关掉的窗口。
    if !app.state::<AppState>().compact_mode.load(Ordering::SeqCst) {
        return Ok(side_str.to_string());
    }

    match state.as_str() {
        "floating" => {
            // 常态浮动：把手窗不该可见，主窗接管鼠标，位置保持不变（用户可自由拖动）。
            //
            // ⚠️ 这里**隐藏**而不是销毁。
            //
            // 销毁的代价是实测出来的（issue 05 §1.9.1）：把手窗一被销毁，
            // 下一次贴边就要重新初始化一个 WebView2，**619~794ms**——
            // 用户可感知的 0.6 秒卡顿。而「把胶囊拖离边缘 → 再拖回来」
            // 是最常见的操作路径，它不该为这件事付 0.6 秒。
            //
            // 代价（已知取舍）：浮动态下把手窗常驻，多占一份隐藏的 WebView2 内存。
            // 换来的是同边反复贴边/唤出**零建窗**。
            //
            // 尺寸问题由「跨边时销毁重建」兜住（见下方 revealed/docked 分支）：
            // 把手窗的尺寸在建窗时按边烧死（左右 7×58 / 上下 58×7），
            // 跨边复用会让定位按错误尺寸算——那正是 5bba9c6 的失败形态。
            if let Some(peek) = app.get_webview_window("peek") {
                // 先让页面回到「视觉隐藏」态，再隐藏窗口。
                //
                // 顺序不能反：把手页的 `is-hidden` 是**视觉**隐藏，而窗口隐藏后
                // 页面仍在运行（WebView 没销毁），下一次窗口被 show 出来时，
                // 若页面还停在「已显示」态，把手会在胶囊滑出**之前**就出现在
                // 屏幕边缘（幽灵把手）。旧行为里窗口每次重建，`is-hidden` 天然
                // 是初始态，所以没有这个问题。
                // ⚠️ 必须**定向**发到 peek 窗：`WebviewWindow::emit` 实际走的是
                // `self.manager.emit`，那是**全局广播**（所有窗口都收得到）。
                // 今天别的窗口没有同名监听器所以无害，但语义上必须用
                // `emit_to("peek", ...)`——将来谁给主窗加个同名监听就会踩坑。
                let _ = app.emit_to("peek", "peek-hide", ());
                // 先设穿透再隐藏：避免隐藏生效前的那一帧还能吃到鼠标。
                let _ = peek.set_ignore_cursor_events(true);
                let _ = peek.hide();
            }
            if let Err(e) = dash.set_ignore_cursor_events(false) {
                tracing::warn!(target: "tum.window", "dashboard set_ignore_cursor_events failed: {e}");
            }
        }
        "revealed" | "docked" => {
            // 分段计时（方案 issues/05 §1.7）。探针关闭时整段退化成「一次 relaxed
            // 原子读 + 若干分支」——不取时钟、不打日志，生产构建零开销。
            let mut seg = crate::perf_probe::Segments::start();

            // docked 才贴死；revealed 时窗口已贴边，不再移动，避免每帧校正抖动。
            if state == "docked" {
                // 贴死成功即回写持久化锚点，让下次启动回到同一条边、同一位置。
                // 位置本就没动（apply_anchor 返回 false）时不写盘。
                if apply_anchor_glide(&dash, anchor) {
                    // ⚠️ 落盘**放后台**：它走同步文件写（tmp + rename），几十毫秒量级，
                    // 而这一步就在「贴边收起」的热路径上——挡住它就等于把落盘延迟
                    // 加到每一次贴边上（这正是用户报告的「贴边有时候卡一下」）。
                    // 详见 [`persist_placement_detached`]。
                    persist_placement_detached(&app, Some(anchor));
                }
                seg.mark_anchor();
            }

            let peek = match acquire_peek_window(&app, side, &mut seg).await {
                Ok(p) => p,
                Err(e) => {
                    // 建窗失败：**不能**把这一轮 docked 同步的后半段（定位、显示、
                    // 鼠标捕获切换）一起吞掉——那会留下「一段看不见但可点」的边缘
                    // 区域：主窗已穿透、把手窗却不存在，用户在屏幕上按不到任何东西。
                    //
                    // 降级：把手窗缺席 → 主窗**保持可交互**（不穿透），并请前端把
                    // 胶囊唤出来（peek-reveal）。宁可少一层交互，也不能让应用不可达。
                    tracing::warn!(
                        target: "tum.window",
                        error = %e,
                        "peek window unavailable; falling back to an interactive dashboard"
                    );
                    let _ = dash.set_ignore_cursor_events(false);
                    let _ = dash.emit("peek-reveal", ());
                    return Ok(side_str.to_string());
                }
            };

            // 刚建出来时还不知道系统实际最小尺寸：拿到真实 outer_size 后由
            // peek_placement 重算，把垂直于贴边轴的多余部分推出屏幕
            // （贴左边/上边时尤其关键）。
            //
            // ℹ️ 本分支**没有** set_size 调用（把手尺寸在建窗时定死），所以
            // `set_size_ms` 恒为 0、`set_size_done` 恒为 false。§2.1 门禁②
            // 「set_size 能不能把被系统钳制的 136×39 缩回 7×58」若要落地，
            // set_size 应插在这里，并在其后调 `seg.mark_set_size()`。
            if let Some(p) = peek_placement(&dash, side, anchor.along) {
                let _ = peek.set_position(p);
            }
            seg.mark_set_pos();
            let _ = peek.show();
            seg.mark_show();
            // 鼠标捕获：两个窗口必须一致切换。走 best-effort 但带兜底 —— 停靠态
            // 主窗是穿透的，把手一旦不能捕获鼠标就再也唤不醒胶囊，此时直接让主窗
            // 把胶囊唤出来（peek-reveal），宁可少一层交互也不能让应用不可达。
            let docked = state == "docked";
            if let Err(e) = peek.set_ignore_cursor_events(!docked) {
                tracing::warn!(target: "tum.window", "peek set_ignore_cursor_events failed: {e}");
                if docked {
                    let _ = dash.emit("peek-reveal", ());
                }
            } else if let Err(e) = dash.set_ignore_cursor_events(docked) {
                tracing::warn!(target: "tum.window", "dashboard set_ignore_cursor_events failed: {e}");
            }
            // 两次 set_ignore_cursor_events 合并记成一步「鼠标捕获归属切换」。
            seg.mark_ignore_cursor();

            // peek-show **不由宿主发**。
            //
            // peek-show 是**前端**发起的（App.svelte 的 syncPeek()：pillDocked &&
            // !pillRevealed 时 emitPeekShow(pillSide)），把手页靠它纠正圆角朝向
            // 并复现自己。宿主在这里再补发一次是**重复事件**——语义虽然幂等
            //（announced / clearTimeout / dataset.side / 去 is-hidden），
            // 但有两个不能接受的后果：
            //
            // ① 它多一次 IPC 往返，而这次往返**就在被测量的热路径上**：
            //    用测量改被测系统，测出来的数就不干净了。
            // ② 它若不受探针开关保护，**生产构建也会多这一次往返**，
            //    违反「生产零开销」这条硬约束。
            //
            // 所以这里既不发也不计时：`emit_ms` / `emit_done` 恒为 0 / false。
            // 代价是「宿主把事件推进 peek webview 要多久」这一段量不到——那是
            // **已知缺口**，要补得另开一次性探针，不能混进稳态循环里。

            seg.log(&state, side_str);
        }
        other => return Err(format!("unknown peek state: {other}")),
    }

    Ok(side_str.to_string())
}
/// 前端在开始原生拖拽时置位、手势结束（或收到 pointerup 快路径）时清零。
/// 与 Rust 侧的「窗口停止移动」检测配合判定拖拽结束。
#[tauri::command]
pub async fn set_pill_dragging(state: State<'_, AppState>, active: bool) -> Result<(), String> {
    state.pill_drag.store(active, Ordering::SeqCst);
    if active {
        // 拖拽开始即作废在跑的主窗滑动：glide 每 8ms set_position 会和 OS 的
        // 拖拽循环抢窗口，而它的 cur 是启动时建模的位置、不重读真实位置——
        // 短拖松手后会被拽回旧模型坐标。
        GLIDE_GEN_MAIN.fetch_add(1, Ordering::SeqCst);
        state.pill_last_move_ms
            .store(crate::now_ms(), Ordering::SeqCst);
    }
    Ok(())
}

/// Open the Settings window. If it already exists, just focus it.
/// The window is created lazily so the dashboard doesn't pay the cost on
/// startup if the user never opens Settings.
#[tauri::command]
pub async fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("settings") {
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title(crate::i18n::window_title(current_lang(&app), crate::i18n::WindowPage::Settings))
    // 680 宽时内容区实得 680−148(导航)−68(左右内边距)=464px，汇率覆盖的
    // `minmax(180px, 1fr)` 只能排 2 列（6 个币种 = 3 行），通用页因此溢出约
    // 230px、要滚动。拉到 820 让内容区到 604px、同一网格排 3 列（2 行），
    // 纵向省下一行。
    //
    // 曾在 1120（右侧实时预览栏需要，见 776fe03），该预览已于 2026-10-18 移除。
    // 高度维持 780：再加高会让窗口在 1080p 上过于占屏，而溢出量已经降到可接受。
    .inner_size(820.0, 780.0)
    .resizable(true)
    .min_inner_size(540.0, 600.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .shadow(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
    #[cfg(windows)]
    {
        // Win11 在窗口尺寸变化后会重画 DWM 边框/圆角，放大后上边缘会重新
        // 出现 1px 描边；复用 dashboard 的模式，每次 Resized 后重设。
        let win_for_event = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) {
                crate::dwm_corner::disable_corner_artifacts(&win_for_event);
            }
        });
    }

    // Alt+F4 / 任务栏关闭：settings 窗靠 hide-not-destroy 复用，真被销毁的话
    // 未保存的草稿就没了。拦下默认关闭，交给前端 requestClose 决定
    // （无草稿时前端直接调 close_settings，等价于正常关闭）。
    let win_for_close = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            // 定向发给 settings 窗：emit 是全局广播（见本文件 emit_to 的注释约定）。
            let _ = win_for_close.emit_to("settings", "settings-close-requested", ());
        }
    });

    // Restore previous position/size if the window-state plugin has saved it.
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

/// Close (hide) the Settings window. We hide instead of destroy so reopening
/// is instant and preserves form state if the user reopens without saving.
#[tauri::command]
pub async fn close_settings(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Open the standalone, resizable trend window (C7). It loads `trend.html`,
/// which mounts a full-window stacked-bar trend view the user can enlarge by
/// resizing the window. If it already exists, just show + focus it.
#[tauri::command]
pub async fn open_trend_window(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("trend") {
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        &app,
        "trend",
        tauri::WebviewUrl::App("trend.html".into()),
    )
    .title(crate::i18n::window_title(current_lang(&app), crate::i18n::WindowPage::Trend))
    // 1080×660 横窗：2026-10 日历区改版（58% 日历 + 右侧统计栏）后，日历格
    // 密度靠「限宽」而非「拉高」解决（旧 720 高是为全宽日历的六个月格子准备
    // 的）。1.64 宽高比下堆叠柱与日历+统计栏同屏；最小尺寸保证 26 列日历格
    // 不小于 ~14px、统计栏不挤成竖条。
    .inner_size(1080.0, 660.0)
    .resizable(true)
    .min_inner_size(880.0, 600.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .shadow(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
    #[cfg(windows)]
    {
        // Win11 在窗口尺寸变化后会重画 DWM 边框/圆角，放大后上边缘会重新
        // 出现 1px 描边；复用 dashboard 的模式，每次 Resized 后重设。
        let win_for_event = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) {
                crate::dwm_corner::disable_corner_artifacts(&win_for_event);
            }
        });
    }
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

/// Open the standalone, resizable local-tool window (C8). It loads
/// `toolwindow.html`, which mounts a full-window stacked-bar view of local AI
/// tool usage. If it already exists, just show + focus it.
#[tauri::command]
pub async fn open_tool_window(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("tools") {
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        &app,
        "tools",
        tauri::WebviewUrl::App("toolwindow.html".into()),
    )
    .title(crate::i18n::window_title(current_lang(&app), crate::i18n::WindowPage::Tools))
    .inner_size(760.0, 480.0)
    .resizable(true)
    .min_inner_size(520.0, 360.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .shadow(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
    #[cfg(windows)]
    {
        // Win11 在窗口尺寸变化后会重画 DWM 边框/圆角，放大后上边缘会重新
        // 出现 1px 描边；复用 dashboard 的模式，每次 Resized 后重设。
        let win_for_event = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) {
                crate::dwm_corner::disable_corner_artifacts(&win_for_event);
            }
        });
    }
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

// --- Settings & credentials CRUD --------------------------------------------

/// Returns the current `Settings` (config.toml snapshot).
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.get().await)
}

/// Whether a settings save changes the effective proxy URL. Blank strings and
/// surrounding whitespace normalize to `None`, so saves that merely touch
/// other settings keep using the fast incremental registry reconcile.
fn proxy_changed(old: &Option<String>, new: &Option<String>) -> bool {
    let normalize = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    normalize(old) != normalize(new)
}

/// Persists new settings to config.toml. Also updates the in-memory cache.
/// Removes state for providers that are no longer enabled so the dashboard
/// immediately reflects the change instead of showing stale snapshots.
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    new_settings: Settings,
) -> Result<(), String> {
    // Capture before save(): saving overwrites the cached settings, after
    // which the old proxy would be unobservable.
    let old_settings = state.settings.get().await;
    let old_proxy_url = old_settings.proxy_url.clone();
    let old_autostart = old_settings.autostart;
    // TokenRouter：补发空 token 的路由 token（hub_token 先例）。必须发生在
    // delta 计算之前，让补发本身作为 router_changed 被观察到。
    let mut new_settings = new_settings;
    crate::router::config::ensure_route_tokens(&mut new_settings);
    // Classify the edit while the "before" image is still in hand - save()
    // overwrites the cache, so the diff has to happen here. Poll loops use this
    // to skip a fetch when nothing they depend on moved.
    let delta = crate::settings_delta::SettingsDelta::compute(&old_settings, &new_settings);

    state
        .settings
        .save(new_settings.clone())
        .await
        .map_err(|e| e.to_string())?;

    // 本机工具数据目录改动：立即刷新扫描器的根目录快照，并强制重扫本地工具，
    // 否则面板会继续显示旧目录（可能是停更的僵留副本）的数据。
    let roots_changed = old_settings.tool_data_roots != new_settings.tool_data_roots
        || old_settings.tool_data_dirs != new_settings.tool_data_dirs;
    if roots_changed {
        state.settings.apply_tool_roots();
        state.local.store(crate::local::LocalToolsPayload::default()).await;
    }

    // Mirror the cached window-behavior flags so the synchronous window-event
    // handlers (close-to-tray, edge snap) pick up the change immediately.
    state.close_to_tray.store(new_settings.close_to_tray, std::sync::atomic::Ordering::SeqCst);
    state.edge_snap.store(new_settings.edge_snap, std::sync::atomic::Ordering::SeqCst);

    // 语言切换：托盘菜单文案随语言重建（已开窗口的标题在重开时生效）。
    // 菜单项 id 恒定，on_menu_event 分发不受影响；重建失败不回滚落盘设置，
    // 下次保存语言时自然再试。
    if old_settings.language != new_settings.language {
        let lang = crate::i18n::resolve(&new_settings.language);
        match crate::build_tray_menu(&app, lang) {
            Ok(menu) => {
                let _ = state._tray.set_menu(Some(menu));
            }
            Err(e) => tracing::warn!(target: "tum.settings", error = %e, "rebuild tray menu after language switch failed"),
        }
        // 已开窗口的标题一并即时刷新；未开的窗口下次打开时自然取新语言。
        let pages = [
            ("peek", crate::i18n::WindowPage::PeekHandle),
            ("settings", crate::i18n::WindowPage::Settings),
            ("trend", crate::i18n::WindowPage::Trend),
            ("tools", crate::i18n::WindowPage::Tools),
        ];
        for (label, page) in pages {
            if let Some(w) = app.get_webview_window(label) {
                let _ = w.set_title(&crate::i18n::window_title(lang, page));
            }
        }
    }

    // TokenRouter：只有端口变化才需要重启监听；enabled 与路由表每请求现读，
    // 翻转快速开关即时生效，无需重启。
    if old_settings.router.port != new_settings.router.port {
        state.router.restart().await;
    }

    // 开机自启对账：设置里开关变化时立即同步注册表项；注册表写入失败则
    // 让本次保存返回错误（设置已落盘，与 proxy 分支的行为一致）。
    if old_autostart != new_settings.autostart {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        let sync = if new_settings.autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        sync.map_err(|e| e.to_string())?;
    }

    if proxy_changed(&old_proxy_url, &new_settings.proxy_url) {
        // Build the replacement first. On failure, settings are already
        // persisted, so leave the live client/registry untouched and let the
        // user correct the URL and save again.
        let client = crate::build_http_client(new_settings.proxy_url.as_deref())?;
        // 路由器的上游转发 client 同步换新（同口径代理，但无总超时）。
        let router_client = crate::router::build_router_http_client(new_settings.proxy_url.as_deref())
            .unwrap_or_else(|_| client.clone());
        state.router.set_http_client(router_client).await;
        // Rebuild every provider against the new client in one shot.
        let fresh = crate::build_registry(
            &new_settings.accounts,
            client.clone(),
            state.storage.clone(),
        );
        *state.http.write().await = client;
        let mut registry = state.registry.write().await;
        *registry = fresh;
        // Spawn a polling task for every rebuilt instance. The previous tasks
        // see the replaced Arc via the identity check at their next loop
        // iteration and retire themselves; a request already in flight at the
        // moment of the swap may briefly keep using the old client, but the
        // per-id poll lock makes it mutually exclusive with the new task's
        // first poll, so balance-delta accounting cannot double-count.
        for account in &new_settings.accounts {
            if let Some(provider) = registry.get(&account.instance_id) {
                crate::scheduler::spawn_one(
                    app.clone(),
                    provider,
                    account.instance_id.clone(),
                );
            }
        }
        drop(registry);
    } else {
        // Reconcile the live registry with the saved accounts incrementally:
        // register newly added accounts (and spawn a polling task), remove
        // accounts that disappeared. Interval/enable edits are picked up by
        // the existing poll loops via the settings_wake ping below.
        let account_ids: std::collections::HashSet<String> = new_settings
            .accounts
            .iter()
            .map(|a| a.instance_id.clone())
            .collect();
        let client = state.http.read().await.clone();
        let mut registry = state.registry.write().await;
        for account in &new_settings.accounts {
            if registry.get(&account.instance_id).is_none() {
                if let Some(provider) =
                    build_account_provider(account, client.clone(), state.storage.clone())
                {
                    registry.insert(provider.clone());
                    let app = app.clone();
                    let id = account.instance_id.clone();
                    crate::scheduler::spawn_one(app, provider, id);
                }
            }
        }
        for id in registry.list() {
            if !account_ids.contains(&id.id()) {
                registry.remove(&id.id());
            }
        }
        drop(registry);
    }

    // Drop snapshots for accounts the user just removed or disabled. Otherwise
    // the dashboard keeps showing them until the next restart, which feels
    // broken.
    let enabled_ids: std::collections::HashSet<String> = new_settings
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .map(|a| a.instance_id.clone())
        .collect();
    let mut guard = state.state.write().await;
    guard.retain(|id, _| enabled_ids.contains(id));
    drop(guard);

    // Tell every window (the dashboard listens for this) that settings
    // changed, so it can re-pull usage and drop cards for disabled providers
    // immediately instead of waiting for a poll that no longer happens.
    let _ = app.emit("settings-changed", &new_settings);
    // Wake polling loops immediately so interval/enable edits apply now
    // instead of after the current period. The delta tells each loop whether
    // a fetch is warranted; a display-only save still wakes them so they can
    // re-read enabled state, but costs no request.
    let _ = state.settings_wake.send(delta);
    Ok(())
}

/// TokenRouter 运行状态：服务健康 + 每路由/每候选实时状态。设置页「路由」
/// pane 与主界面快速开关的数据源。
#[tauri::command]
pub async fn get_router_status(
    state: State<'_, AppState>,
) -> Result<crate::router::RouterStatusPayload, String> {
    Ok(crate::router::status_payload(&state.router).await)
}

/// 拉取某候选上游的模型列表（GET {base}/v1/models），供设置页「模型名」
/// 下拉选择。凭据按账户从 keyring 取。
#[tauri::command]
pub async fn fetch_upstream_models(
    state: State<'_, AppState>,
    protocol: crate::router::config::RouterProtocol,
    account: String,
    base_url: String,
) -> Result<Vec<String>, String> {
    state
        .router
        .fetch_upstream_models(protocol, &account, &base_url)
        .await
}

/// 直接切换开机自启：先写注册表项，成功后再落盘设置；注册表写入失败时
/// 不改动存储，保证复选框与系统实际状态不出现分歧。
#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        if enabled {
            autolaunch.enable().map_err(|e| e.to_string())?;
        } else {
            autolaunch.disable().map_err(|e| e.to_string())?;
        }
    }

    let mut current = state.settings.get().await;
    if current.autostart != enabled {
        current.autostart = enabled;
        state.settings.save(current).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Add a new account (or update an existing one by `instance_id`) to Settings
/// and start polling it. Credentials are saved separately by
/// [`save_credentials`] once the caller knows the `instance_id`.
///
/// A brand-new account is passed with an empty `instance_id`; the backend
/// assigns one and returns it. Updating an existing account keeps its id.
#[tauri::command]
pub async fn upsert_account(
    app: AppHandle,
    state: State<'_, AppState>,
    mut account: AccountMeta,
) -> Result<String, String> {
    // Assign an instance_id for brand-new accounts.
    if account.instance_id.is_empty() {
        account = AccountMeta::new(&account.provider_kind, account.label.clone());
    }

    let mut settings = state.settings.get().await;
    let old_settings = settings.clone();
    if let Some(existing) = settings
        .accounts
        .iter_mut()
        .find(|a| a.instance_id == account.instance_id)
    {
        // Preserve the instance_id; `account` carries the same one here.
        *existing = account.clone();
    } else {
        settings.accounts.push(account.clone());
    }
    // A brand-new account has no snapshot, so this is poll-relevant; editing
    // only the label or colour of an existing one is not. Computed before
    // save() because that call consumes the "after" image.
    let delta = crate::settings_delta::SettingsDelta::compute(&old_settings, &settings);
    state
        .settings
        .save(settings)
        .await
        .map_err(|e| e.to_string())?;

    // Register a live provider instance and spawn its polling task.
    let instance_id = account.instance_id.clone();
    let mut registry = state.registry.write().await;
    if registry.get(&instance_id).is_none() {
        let client = state.http.read().await.clone();
        if let Some(provider) = build_account_provider(&account, client, state.storage.clone()) {
            registry.insert(provider.clone());
            crate::scheduler::spawn_one(app, provider, instance_id.clone());
        }
    }
    drop(registry);
    let _ = state.settings_wake.send(delta);
    Ok(instance_id)
}

/// Remove an account and its credentials. Polling for it stops.
#[tauri::command]
pub async fn remove_account(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<(), String> {
    let mut settings = state.settings.get().await;
    let old_settings = settings.clone();
    settings.accounts.retain(|a| a.instance_id != instance_id);
    // A removal is a membership change: the loop for this id retires at its
    // next still_registered check, and the other accounts must not refetch.
    let delta = crate::settings_delta::SettingsDelta::compute(&old_settings, &settings);
    state
        .settings
        .save(settings)
        .await
        .map_err(|e| e.to_string())?;

    // Remove the live instance, its keyring credentials and its snapshot state.
    let _ = state.settings.delete_credentials(&instance_id);
    state.credentials.write().await.remove(&instance_id);
    state.registry.write().await.remove(&instance_id);
    state.state.write().await.remove(&instance_id);
    let _ = state.settings_wake.send(delta);
    Ok(())
}

/// Reads the stored credentials for a provider so the Settings UI can prefill
/// the form when the user clicks 「修改」. Secrets never leave this machine
/// (OS credential store → local UI, masked inputs); without this round-trip
/// the write-only fields would force users to re-enter every value just to
/// change one (e.g. adding TokenRouter's optional routing API key).
#[tauri::command]
pub async fn get_credentials(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Option<Credentials>, String> {
    Ok(state.settings.load_credentials(&provider_id))
}

/// 路由通道探测：用表单里的凭据对上游推理端点发一次免费探测（GET /v1/models）。
/// 与 test_provider 的监控通道互补——火山等监控/推理分离的 Provider 上，
/// 监控通过不代表推理接口可达。无路由 Key 时返回 ok=false + 原因（不算错误）。
#[tauri::command]
pub async fn test_routing_channel(
    state: State<'_, AppState>,
    protocol: crate::router::config::RouterProtocol,
    base_url: String,
    creds: Credentials,
) -> Result<crate::router::RoutingProbeResult, String> {
    let Some(api_key) = crate::router::routing_api_key_of(Some(&creds)) else {
        return Ok(crate::router::RoutingProbeResult {
            ok: false,
            status: None,
            url: crate::router::forward::upstream_url(&base_url, "/v1/models"),
            error: Some("该凭据没有可用于路由的推理 API Key".to_string()),
            models: None,
        });
    };
    Ok(state
        .router
        .probe_routing_channel(protocol, &base_url, &api_key)
        .await)
}

/// Saves credentials for a provider to the OS credential store and refreshes
/// the in-memory credential cache so the next poll picks them up.
///
/// Accepts the provider id + a `Credentials` payload tagged with `kind`.
#[tauri::command]
pub async fn save_credentials(
    state: State<'_, AppState>,
    provider_id: String,
    creds: Credentials,
) -> Result<(), String> {
    state
        .settings
        .save_credentials(&provider_id, &creds)
        .map_err(|e| e.to_string())?;
    state
        .credentials
        .write()
        .await
        .insert(provider_id, creds);
    Ok(())
}

/// Removes credentials for a provider from both the OS store and the
/// in-memory cache. The provider will return `NotConfigured` on next poll.
#[tauri::command]
pub async fn delete_credentials(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<(), String> {
    state
        .settings
        .delete_credentials(&provider_id)
        .map_err(|e| e.to_string())?;
    state.credentials.write().await.remove(&provider_id);
    Ok(())
}

/// Tests connectivity for a provider *kind* by performing a one-shot fetch with
/// the supplied credentials (without persisting them). Returns either
/// `Ok(snapshot)` for the frontend to preview, or `Err(message)`.
///
/// This lets the Settings UI show a "test connection" status before saving -
/// it works whether or not the account is registered yet.
#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    provider_kind: String,
    creds: Credentials,
) -> Result<UsageSnapshot, String> {
    let account = AccountMeta {
        instance_id: format!("test-{provider_kind}"),
        provider_kind: provider_kind.clone(),
        label: provider_kind.clone(),
        accent_color: "#000000".to_string(),
        enabled: true,
        note: None,
    };
    let client = state.http.read().await.clone();
    let provider = build_account_provider(&account, client, state.storage.clone())
        .ok_or_else(|| format!("unknown provider kind: {provider_kind}"))?;
    provider
        .fetch_usage(&creds)
        .await
        .map_err(|e| e.to_string())
}

/// Result of a proxy connectivity probe. Serialized to the frontend as
/// `{ ok, status, error }`; a failed probe is a normal result, not a command
/// error, so the UI can render inline.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// Local Codex credentials detected from `~/.codex/auth.json`, using the
/// spec's wire key names.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedCodexToken {
    pub token: String,
    pub account_id: String,
    pub last_refresh: String,
}

/// Pure mapping from a probe response (HTTP status) or transport error to the
/// wire result. A non-2xx status is reported as not-ok with a message.
fn proxy_probe_outcome(probe: Result<u16, String>) -> ProxyTestResult {
    match probe {
        Ok(status) if (200..300).contains(&status) => ProxyTestResult {
            ok: true,
            status: Some(status),
            error: None,
        },
        Ok(status) => ProxyTestResult {
            ok: false,
            status: Some(status),
            error: Some(format!("代理已连通，但目标返回 HTTP {status}")),
        },
        Err(error) => ProxyTestResult {
            ok: false,
            status: None,
            error: Some(error),
        },
    }
}

/// Connectivity probes tried in order until one answers.
///
/// The first entry is the historical `google.com/generate_204`, kept for
/// users behind a filtered route where it is the cheapest signal. It is *not*
/// the only entry: on a mainland-China network google.com is unreachable with
/// or without a working proxy, so a single hard-coded probe reports "proxy
/// broken" for a perfectly good proxy. The rest are hosts reachable from most
/// networks, tried in order of how cheap the response is.
const PROXY_PROBES: &[&str] = &[
    "http://www.msftconnecttest.com/connecttest.txt",
    "http://captive.apple.com/hotspot-detect.html",
    "http://www.gstatic.com/generate_204",
    "https://www.google.com/generate_204",
];

/// Fold the ordered per-probe results into one verdict.
///
/// Split out from the network loop so the decision is testable without a live
/// proxy. Rules:
/// - the first 2xx wins and stops the sequence;
/// - the first *response* of any status also stops it, because receiving an
///   HTTP status at all proves the proxy completed the request - a captive
///   portal 403 is a different diagnosis than "no route", and is far more
///   actionable than "everything failed";
/// - if every probe failed at the transport layer, the last error is reported.
fn proxy_probe_verdict(attempts: Vec<Result<u16, String>>) -> ProxyTestResult {
    let mut last_err: Option<String> = None;
    for attempt in attempts {
        match attempt {
            Ok(status) => return proxy_probe_outcome(Ok(status)),
            Err(e) => last_err = Some(e),
        }
    }
    proxy_probe_outcome(Err(last_err.unwrap_or_else(|| {
        "代理连接失败：所有探测地址均无响应".to_string()
    })))
}

/// Build a temporary client through the supplied proxy and probe each
/// endpoint in [`PROXY_PROBES`] until one answers. Nothing is persisted; the
/// saved client is untouched.
#[tauri::command]
pub async fn test_proxy(url: String) -> Result<ProxyTestResult, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(ProxyTestResult {
            ok: false,
            status: None,
            error: Some("请输入代理地址".to_string()),
        });
    }
    let client = crate::build_http_client(Some(&url))?;

    let mut attempts = Vec::with_capacity(PROXY_PROBES.len());
    for probe in PROXY_PROBES {
        let status = client
            .get(*probe)
            .send()
            .await
            .map(|r| r.status().as_u16())
            .map_err(|e| format!("代理连接失败：{e}"));
        let answered = status.is_ok();
        attempts.push(status);
        if answered {
            break;
        }
    }
    Ok(proxy_probe_verdict(attempts))
}


/// What `export_data` wrote. `rows` counts records, not file lines: the CSV
/// header is not a row, and for JSON it is the number of entities included.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub rows: usize,
    pub bytes: usize,
    pub path: String,
}

/// Write the current data out to a file the user owns.
///
/// `scope` picks the dataset (`tools` / `providers` / `all`), `format` the
/// shape (`json` / `csv`). Read-only with respect to app state: this never
/// mutates the cache or triggers a rescan beyond the one a fresh panel would
/// do anyway.
#[tauri::command]
pub async fn export_data(
    state: State<'_, AppState>,
    format: String,
    scope: String,
    dest: String,
) -> Result<ExportSummary, String> {
    let dest = dest.trim().to_string();
    if dest.is_empty() {
        return Err("请选择导出路径".to_string());
    }
    if !matches!(format.as_str(), "json" | "csv") {
        return Err(format!("不支持的导出格式：{format}"));
    }
    if !matches!(scope.as_str(), "tools" | "providers" | "all") {
        return Err(format!("不支持的导出范围：{scope}"));
    }
    if format == "csv" && scope == "all" {
        return Err(
            "CSV 放不下用量窗口和本地工具这两套互不相干的列，请分开导出或改用 JSON".to_string(),
        );
    }

    let want_providers = scope == "providers" || scope == "all";
    let want_tools = scope == "tools" || scope == "all";

    let providers: Vec<UsageSnapshot> = if want_providers {
        let guard = state.state.read().await;
        guard
            .values()
            .filter_map(|ps: &ProviderState| ps.snapshot.clone())
            .collect()
    } else {
        Vec::new()
    };

    // Prefer the warm cache: an export triggered from the tools panel should
    // not re-parse every session log just because the short-lived cache
    // expired. Fall back to the same scan the panel itself would do.
    let tools: LocalToolsPayload = if want_tools {
        match state.local.cached().await {
            Some(p) => p,
            None => {
                let scanned = crate::local::scan_all(&state.storage);
                let sessions_parsed: u64 = scanned.iter().map(|t| t.session_count).sum();
                LocalToolsPayload { tools: scanned, sessions_parsed }
            }
        }
    } else {
        LocalToolsPayload::default()
    };

    let (body, rows) = match (format.as_str(), scope.as_str()) {
        ("csv", "providers") => (
            crate::export::render_providers_csv(&providers),
            crate::export::provider_row_count(&providers),
        ),
        ("csv", "tools") => (
            crate::export::render_tools_csv(&tools),
            crate::export::tool_row_count(&tools),
        ),
        ("json", "providers") => (
            serde_json::to_string_pretty(&providers)
                .map_err(|e| format!("序列化失败：{e}"))?,
            providers.len(),
        ),
        ("json", "tools") => (
            serde_json::to_string_pretty(&tools).map_err(|e| format!("序列化失败：{e}"))?,
            tools.tools.len(),
        ),
        ("json", "all") => {
            let doc = crate::export::ExportAllJson {
                providers: &providers,
                tools: &tools,
            };
            (
                serde_json::to_string_pretty(&doc).map_err(|e| format!("序列化失败：{e}"))?,
                providers.len() + tools.tools.len(),
            )
        }
        _ => return Err("导出参数组合不受支持".to_string()),
    };

    // tokio::fs::write yields (), not a byte count, so report the length we
    // handed it rather than trusting a second stat of the file.
    let bytes = body.as_bytes().len();
    tokio::fs::write(&dest, body.as_bytes())
        .await
        .map_err(|e| format!("写入 {dest} 失败：{e}"))?;

    Ok(ExportSummary {
        rows,
        bytes,
        path: dest,
    })
}
/// Read the local Codex CLI credentials (`~/.codex/auth.json`) so the
/// Settings form can pre-fill them. `None` when the file is absent or the
/// token is blank.
#[tauri::command]
pub async fn detect_codex_token() -> Result<Option<DetectedCodexToken>, String> {
    Ok(crate::providers::codex::detect_auth().map(|auth| DetectedCodexToken {
        token: auth.access_token,
        account_id: auth.account_id,
        last_refresh: auth.last_refresh,
    }))
}

#[cfg(test)]
mod proxy_tests {
    use super::{proxy_changed, proxy_probe_outcome, proxy_probe_verdict, PROXY_PROBES};

    #[test]
    fn detects_real_change() {
        assert!(proxy_changed(&None, &Some("http://127.0.0.1:7890".to_string())));
        assert!(proxy_changed(
            &Some("http://127.0.0.1:7890".to_string()),
            &Some("socks5://127.0.0.1:1080".to_string())
        ));
    }

    #[test]
    fn treats_blank_as_none() {
        assert!(!proxy_changed(&None, &Some("   ".to_string())));
        assert!(!proxy_changed(&Some(" ".to_string()), &None));
    }

    #[test]
    fn ignores_surrounding_whitespace() {
        assert!(!proxy_changed(
            &Some("http://127.0.0.1:7890".to_string()),
            &Some("  http://127.0.0.1:7890 ".to_string())
        ));
    }

    #[test]
    fn probe_outcome_reports_success_status() {
        let r = proxy_probe_outcome(Ok(204u16));
        assert!(r.ok);
        assert_eq!(r.status, Some(204));
        assert!(r.error.is_none());
    }

    #[test]
    fn probe_outcome_marks_non_success_status() {
        let r = proxy_probe_outcome(Ok(500u16));
        assert!(!r.ok);
        assert_eq!(r.status, Some(500));
        assert!(r.error.is_some());
    }

    #[test]
    fn probe_outcome_reports_transport_error() {
        let r = proxy_probe_outcome(Err("connection refused".to_string()));
        assert!(!r.ok);
        assert!(r.status.is_none());
        assert_eq!(r.error.as_deref(), Some("connection refused"));
    }

    #[test]
    fn verdict_stops_at_the_first_success() {
        // Later probes are never reached: a 204 already proves the proxy works.
        let r = proxy_probe_verdict(vec![
            Err("first unreachable".to_string()),
            Ok(204u16),
            Err("never tried".to_string()),
        ]);
        assert!(r.ok);
        assert_eq!(r.status, Some(204));
    }

    #[test]
    fn verdict_stops_at_the_first_http_response_even_when_not_2xx() {
        // An HTTP status at all means the proxy completed the request. A
        // captive-portal 403 is a better diagnosis than "everything failed",
        // so the sequence must not keep probing past it.
        let r = proxy_probe_verdict(vec![Err("blocked".to_string()), Ok(403u16)]);
        assert!(!r.ok);
        assert_eq!(r.status, Some(403));
        assert!(r.error.as_deref().unwrap().contains("403"));
    }

    #[test]
    fn verdict_reports_the_last_error_when_all_probes_fail() {
        let r = proxy_probe_verdict(vec![
            Err("first".to_string()),
            Err("second".to_string()),
            Err("last".to_string()),
        ]);
        assert!(!r.ok);
        assert!(r.status.is_none());
        assert_eq!(r.error.as_deref(), Some("last"));
    }

    #[test]
    fn verdict_on_an_empty_probe_list_is_a_failure_not_a_success() {
        let r = proxy_probe_verdict(vec![]);
        assert!(!r.ok);
        assert!(r.error.is_some());
    }

    #[test]
    fn probe_list_leads_with_endpoints_reachable_without_google() {
        // Regression guard for the original bug: a single google.com probe
        // reports a broken proxy on a mainland-China network. google must not
        // be the only, nor the first, entry.
        assert!(PROXY_PROBES.len() >= 2);
        assert!(!PROXY_PROBES[0].contains("google.com"));
        assert!(PROXY_PROBES.iter().any(|p| p.contains("google.com")));
        for p in PROXY_PROBES {
            assert!(p.starts_with("http://") || p.starts_with("https://"));
        }
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::clamp_rect;

    #[test]
    fn keeps_inside_when_already_visible() {
        let (x, y) = clamp_rect(100.0, 100.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (100.0, 100.0));
    }

    #[test]
    fn pulls_off_screen_window_back_into_view() {
        let (x, y) = clamp_rect(1800.0, 950.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (1552.0, 432.0));
    }

    #[test]
    fn keeps_margin_on_every_edge() {
        let (x, y) = clamp_rect(0.0, 0.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (8.0, 8.0));
    }

    #[test]
    fn pins_when_window_wider_than_work_area() {
        let (x, y) = clamp_rect(500.0, 0.0, 2000.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (8.0, 8.0));
    }

    #[test]
    fn respects_work_area_origin() {
        let (x, y) = clamp_rect(-1920.0, -50.0, 360.0, 600.0, -1920.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (-1912.0, 8.0));
    }

    #[test]
    fn pins_when_window_taller_than_work_area() {
        let (x, y) = clamp_rect(0.0, 500.0, 360.0, 1200.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (8.0, 8.0));
    }
}
