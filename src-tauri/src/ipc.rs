//! Tauri IPC commands exposed to the frontend.
//!
//! Naming convention: snake_case in Rust, frontend calls via `invoke('get_usage')`.

use crate::build_account_provider;
use crate::hub::HubDevice;
use crate::local::LocalToolsPayload;
use crate::providers::{
    AccountMeta, Credentials, HeatmapCell, PRESETS, Preset, ProviderState, UsageSnapshot,
};
use crate::settings::Settings;
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
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
    // MiniMax 账户卡片：优先读取持久化到 DB 的本机 MiniMax Code token 热力图
    // （由 get_local_tools 写入 minimax-code 键），避免重新扫描大库；无数据时
    // 回退到该账户自身（百分比）热力图。
    if provider_id.starts_with("minimax") {
        let cells = state
            .storage
            .load_heatmap("minimax-code", days)
            .unwrap_or_default();
        if !cells.is_empty() {
            return Ok(cells);
        }
    }
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

    // 把本地 MiniMax token 用量持久化到 DB（键 minimax-code），供 MiniMax 账户
    // 卡片日历热力图快速读取，无需每次重新扫描 ~/.minimax 的大 SQLite。
    crate::local::persist_minimax(&state.storage, &tools);

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
    let daily = crate::hub::tool_daily_from_cache(&state.local).await;
    let self_device = crate::hub::build_device_usage(
        &id, &host, &os, &arch, &ver, tool_tokens, provider_count, tool_count, daily,
    );

    // 候选设备：agent 模式下优先拉远端 hub 列表，其次本地存储。
    let s = state.settings.get().await;
    let mut warning: Option<String> = None;
    let mut sources: Vec<Vec<HubDevice>> = Vec::new();
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
            tracing::warn!(provider = %provider.id(), error = %e,
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

/// 把手窗与胶囊主行的逻辑尺寸（DIP）。主行固定取收起态的 56，这样明细
/// 展开时把手不会跟着下移，收起后也无需重新对齐。
const PEEK_W: f64 = 7.0;
const PEEK_H: f64 = 58.0;
const PILL_ROW_H: f64 = 56.0;

/// 胶囊应贴靠的水平边。比较窗口中心与工作区中心；正中时归右侧（默认边）。
fn dock_side(x: f64, w: f64, area_x: f64, area_w: f64) -> bool {
    x + w / 2.0 >= area_x + area_w / 2.0
}

/// 把窗口横向贴死到最近的水平边缘，纵向保留原位置但夹在工作区内。
/// 把手画在屏幕边缘，胶囊必须紧贴边缘二者才能对齐；纵向是用户自由选择的位置。
fn dock_rect(
    x: f64, y: f64, w: f64, h: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
) -> (f64, f64) {
    let left = area_x;
    let right = (area_x + area_w - w).max(area_x);
    let nx = if dock_side(x, w, area_x, area_w) { right } else { left };
    let max_y = (area_y + area_h - h).max(area_y);
    (nx, y.clamp(area_y, max_y))
}

/// 把手的物理位置：贴死所在边缘，纵向中心对齐胶囊主行（`row_h` 为物理像素）。
fn peek_rect(
    dash_y: f64, row_h: f64, is_right: bool,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
    peek_w: f64, peek_h: f64,
) -> (f64, f64) {
    let x = if is_right { area_x + area_w - peek_w } else { area_x };
    let y = dash_y + row_h / 2.0 - peek_h / 2.0;
    let max_y = (area_y + area_h - peek_h).max(area_y);
    (x, y.clamp(area_y, max_y))
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

/// 把窗口横向贴死到最近的左/右边缘，纵向夹在工作区内。compact 胶囊专用。
pub fn dock_window(window: &WebviewWindow) -> bool {
    let Ok(pos) = window.outer_position() else { return false; };
    let Ok(size) = window.outer_size() else { return false; };
    let Ok(Some(monitor)) = window.current_monitor() else { return false; };
    let area = monitor.work_area();
    let (nx, ny) = dock_rect(
        f64::from(pos.x),
        f64::from(pos.y),
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
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

/// 贴边把手的物理位置 + 所在边（true = 右侧）。窗口无监视器时返回 None。
fn peek_placement(window: &WebviewWindow) -> Option<(bool, PhysicalPosition<i32>)> {
    let Ok(pos) = window.outer_position() else { return None; };
    let Ok(size) = window.outer_size() else { return None; };
    let Ok(Some(monitor)) = window.current_monitor() else { return None; };
    let area = monitor.work_area();
    let scale = f64::from(monitor.scale_factor());
    let is_right = dock_side(
        f64::from(pos.x),
        f64::from(size.width),
        f64::from(area.position.x),
        f64::from(area.size.width),
    );
    let (px, py) = peek_rect(
        f64::from(pos.y),
        PILL_ROW_H * scale,
        is_right,
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
        PEEK_W * scale,
        PEEK_H * scale,
    );
    Some((is_right, PhysicalPosition::new(px.round() as i32, py.round() as i32)))
}

/// 让把手跟随 Dashboard 胶囊的纵向位置。把手尚未创建时为空操作。
pub fn reposition_peek(window: &WebviewWindow) {
    let Some(peek) = window.app_handle().get_webview_window("peek") else { return; };
    let Some((_, pos)) = peek_placement(window) else { return; };
    if peek.outer_position().map(|cur| cur == pos).unwrap_or(false) {
        return;
    }
    let _ = peek.set_position(pos);
}

/// Snap the window to the nearest screen edge when it is dragged within
/// `margin_logical` of one. Called from the dashboard's `Moved` window event.
pub fn snap_to_edges(window: &WebviewWindow, margin_logical: f64) -> bool {
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

/// Switch the dashboard window between full and compact modes.
#[tauri::command]
pub async fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    use tauri::LogicalSize;

    let window = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    let compact = app.state::<AppState>().compact_mode.clone();

    match mode.as_str() {
        "dashboard" => {
            window
                .set_size(LogicalSize::new(400u32, 680u32))
                .map_err(|e| e.to_string())?;
            // 全窗态是普通窗口：恢复鼠标交互，贴边把手在此模式下没有意义。
            let _ = window.set_ignore_cursor_events(false);
            if let Some(peek) = app.get_webview_window("peek") {
                let _ = peek.close();
            }
            compact.store(false, Ordering::SeqCst);
            clamp_window_to_work_area(&window, 8.0);
        }
        "compact" => {
            // 迷你胶囊 168x56：主行 = 圆环 + 百分比 + 分隔线 + 品牌芯片。
            window
                .set_size(LogicalSize::new(168u32, 56u32))
                .map_err(|e| e.to_string())?;
            compact.store(true, Ordering::SeqCst);
            // 胶囊必须紧贴左/右边缘，7px 把手才能与之对齐；纵向夹在屏内。
            dock_window(&window);
        }
        other => return Err(format!("unknown window mode: {other}")),
    }

    Ok(())
}

/// 创建/定位贴边把手，并把「谁捕获鼠标」交给本命令统一托管：
/// `docked = true`（胶囊收起）→ 把手捕获鼠标、主窗穿透；
/// `docked = false`（胶囊已滑入）→ 把手穿透（被胶囊盖住）、主窗捕获鼠标。
/// 返回胶囊贴靠的水平边，供前端决定滑入方向。
#[tauri::command]
pub async fn sync_peek_window(app: AppHandle, docked: bool) -> Result<String, String> {
    let dash = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;
    let Some((is_right, pos)) = peek_placement(&dash) else {
        return Err("dashboard window has no monitor".to_string());
    };

    // 只在 compact 胶囊态才有把手；dashboard 态（可能是模式切换竞态）直接
    // 返回所在边，不重建刚被 set_window_mode 关掉的窗口。
    if !app.state::<AppState>().compact_mode.load(Ordering::SeqCst) {
        return Ok(if is_right { "right" } else { "left" }.to_string());
    }

    // 每个状态边界都强制贴边一次：胶囊必须紧贴左/右边缘，7px 把手才能与之对齐。
    // 启动恢复时窗口刚被改过尺寸，那一瞬间贴边可能读到旧尺寸而偏移，这里是
    // 窗口真正显示后的二次校正；收起草时也由此保证「收起即贴边」。
    dock_window(&dash);

    let peek = match app.get_webview_window("peek") {
        Some(existing) => existing,
        None => {
            // 把手页通过初始化脚本拿到所在边，决定 4px 圆角朝向。
            let side = if is_right { "\"right\"" } else { "\"left\"" };
            let built = WebviewWindowBuilder::new(
                &app,
                "peek",
                tauri::WebviewUrl::App("peek.html".into()),
            )
            .title("TokenUsageMonitor · 贴边把手")
            .inner_size(PEEK_W, PEEK_H)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .focused(false)
            .visible(false)
            .initialization_script(format!("window.__PEEK_SIDE__ = {side};"))
            .build()
            .map_err(|e| e.to_string())?;
            #[cfg(windows)]
            crate::dwm_corner::disable_corner_artifacts(&built);
            built
        }
    };

    let _ = peek.set_position(pos);
    let _ = peek.show();
    // 鼠标捕获：两个窗口必须一致切换。走 best-effort 但带兜底 —— 停靠态主窗是
    // 穿透的，把手一旦不能捕获鼠标就再也唤不醒胶囊，此时直接让主窗把胶囊唤出来
    // （peek-reveal），宁可少一层交互也不能让应用不可达。
    if let Err(e) = peek.set_ignore_cursor_events(!docked) {
        tracing::warn!("peek set_ignore_cursor_events failed: {e}");
        if docked {
            let _ = dash.emit("peek-reveal", ());
        }
    } else if let Err(e) = dash.set_ignore_cursor_events(docked) {
        tracing::warn!("dashboard set_ignore_cursor_events failed: {e}");
    }

    Ok(if is_right { "right" } else { "left" }.to_string())
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
    .title("TokenUsageMonitor · 设置")
    .inner_size(680.0, 780.0)
    .resizable(true)
    .min_inner_size(540.0, 600.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);

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
    .title("TokenUsageMonitor · 用量趋势")
    .inner_size(760.0, 480.0)
    .resizable(true)
    .min_inner_size(520.0, 360.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
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
    .title("TokenUsageMonitor · 工具用量")
    .inner_size(760.0, 480.0)
    .resizable(true)
    .min_inner_size(520.0, 360.0)
    .visible(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
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
    let old_proxy_url = state.settings.get().await.proxy_url.clone();

    state
        .settings
        .save(new_settings.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Mirror the cached window-behavior flags so the synchronous window-event
    // handlers (close-to-tray, edge snap) pick up the change immediately.
    state.close_to_tray.store(new_settings.close_to_tray, std::sync::atomic::Ordering::SeqCst);
    state.edge_snap.store(new_settings.edge_snap, std::sync::atomic::Ordering::SeqCst);

    if proxy_changed(&old_proxy_url, &new_settings.proxy_url) {
        // Build the replacement first. On failure, settings are already
        // persisted, so leave the live client/registry untouched and let the
        // user correct the URL and save again.
        let client = crate::build_http_client(new_settings.proxy_url.as_deref())?;
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
    // instead of after the current period.
    let _ = state.settings_wake.send(());
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
    let _ = state.settings_wake.send(());
    Ok(instance_id)
}

/// Remove an account and its credentials. Polling for it stops.
#[tauri::command]
pub async fn remove_account(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<(), String> {
    let mut settings = state.settings.get().await;
    settings.accounts.retain(|a| a.instance_id != instance_id);
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
    let _ = state.settings_wake.send(());
    Ok(())
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

/// Build a temporary client through the supplied proxy and probe a lightweight
/// 204 endpoint. Nothing is persisted; the saved client is untouched.
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
    let probe = async {
        let resp = client
            .get("https://www.google.com/generate_204")
            .send()
            .await
            .map_err(|e| format!("代理连接失败：{e}"))?;
        Ok::<u16, String>(resp.status().as_u16())
    }
    .await;
    Ok(proxy_probe_outcome(probe))
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
    use super::{proxy_changed, proxy_probe_outcome};

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

#[cfg(test)]
mod dock_tests {
    use super::{dock_rect, dock_side, peek_rect};

    const AREA_X: f64 = 0.0;
    const AREA_Y: f64 = 0.0;
    const AREA_W: f64 = 1920.0;
    const AREA_H: f64 = 1040.0;

    #[test]
    fn dock_rect_flushes_to_the_left_edge() {
        let (x, y) = dock_rect(10.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (0.0, 300.0));
    }

    #[test]
    fn dock_rect_flushes_to_the_right_edge() {
        let (x, y) = dock_rect(1000.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (1752.0, 300.0));
    }

    #[test]
    fn dock_rect_exact_middle_docks_right() {
        let (x, _) = dock_rect(876.0, 0.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(x, 1752.0);
    }

    #[test]
    fn dock_rect_keeps_vertical_but_clamps_inside() {
        let (_, top) = dock_rect(0.0, -40.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(top, 0.0);
        let (_, bottom) = dock_rect(0.0, 1030.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(bottom, 984.0);
    }

    #[test]
    fn dock_side_mirrors_for_a_negative_origin_monitor() {
        assert!(!dock_side(-1900.0, 168.0, -1920.0, 1920.0));
        assert!(dock_side(-60.0, 168.0, -1920.0, 1920.0));
    }

    #[test]
    fn peek_rect_sits_flush_and_centres_on_the_pill_row() {
        let (x, y) = peek_rect(100.0, 56.0, true, AREA_X, AREA_Y, AREA_W, AREA_H, 7.0, 58.0);
        assert_eq!((x, y), (1913.0, 99.0));
        let (lx, _) = peek_rect(100.0, 56.0, false, AREA_X, AREA_Y, AREA_W, AREA_H, 7.0, 58.0);
        assert_eq!(lx, 0.0);
    }

    #[test]
    fn peek_rect_clamps_to_the_work_area() {
        let (_, top) = peek_rect(0.0, 56.0, true, AREA_X, AREA_Y, AREA_W, AREA_H, 7.0, 58.0);
        assert_eq!(top, 0.0);
        let (_, bottom) = peek_rect(1020.0, 56.0, true, AREA_X, AREA_Y, AREA_W, AREA_H, 7.0, 58.0);
        assert_eq!(bottom, 982.0);
    }
}
