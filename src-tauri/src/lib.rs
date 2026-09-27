//! TokenUsageMonitor application root.
//!
//! Wires together:
//! - The provider registry (real providers registered based on user Settings)
//! - Shared per-provider state cache (snapshots + last error)
//! - Credential map (loaded from keyring at startup; updated via Settings UI)
//! - SQLite storage (for heatmap daily snapshots)
//! - Settings store (config.toml + keyring)
//! - Tauri plugins + IPC command handlers
//! - Periodic scheduler that emits `usage-updated` events

pub mod exchange;
pub mod ipc;
pub mod hub;
pub mod local;
pub mod notify;
pub mod pricing;
pub mod providers;
pub mod scheduler;
pub mod settings;
pub mod signing;
pub mod storage;

use providers::{
    AccountMeta, Credentials, ProviderRegistry, SharedProviderState,
};
use notify::SharedNotifyState;
use reqwest::Client;
use scheduler::SharedBurnTracker;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tokio::sync::{watch, RwLock};

/// Global application state shared across IPC commands and the scheduler.
pub struct AppState {
    /// Live provider instances, keyed by `AccountMeta::instance_id`. Wrapped
    /// in an `RwLock` so accounts can be added/removed at runtime from Settings.
    pub registry: Arc<RwLock<ProviderRegistry>>,
    /// Latest snapshot + last error per account instance.
    pub state: SharedProviderState,
    /// User-configured credentials per account instance id. Populated at
    /// startup from `SettingsStore::load_credentials` and kept in sync by the
    /// Settings UI via `save_credentials` / `delete_credentials`.
    pub credentials: Arc<RwLock<HashMap<String, Credentials>>>,
    /// SQLite store for daily snapshots (heatmap data).
    pub storage: Arc<storage::Storage>,
    /// TOML + keyring settings.
    pub settings: Arc<settings::SettingsStore>,
    /// Burn-diff baselines shared by the scheduler and force_refresh so a
    /// manual refresh diffs against the same previous snapshot.
    pub burn: SharedBurnTracker,
    /// Threshold notification dedupe state (one warn + one crit per window).
    pub notify: SharedNotifyState,
    /// Pause flag broadcast. `toggle_polling` flips it; polling tasks
    /// subscribe and skip ticks while the latest value is true.
    pub pause_tx: Arc<watch::Sender<bool>>,
    /// A ping on this channel wakes every polling task so interval edits and
    /// enable/disable changes apply immediately instead of after one period.
    pub settings_wake: Arc<watch::Sender<()>>,
    /// Swappable shared HTTP client. A proxy change replaces the client under
    /// the write lock and rebuilds the whole registry; access sites take a
    /// read lock and clone the cheap inner Arc.
    pub http: Arc<RwLock<Client>>,
    /// Per-account-id async mutexes serializing whole polls (fetch +
    /// balance-delta accounting). Shared across the old and new loops of a
    /// rebuilt registry so the two can never interleave their non-idempotent
    /// accounting. Entries are created lazily and live for the app lifetime.
    pub poll_locks: crate::scheduler::PollLocks,
    /// Cached `Settings::close_to_tray`, mirrored so the synchronous window
    /// close interceptor can read it without an async lock. Kept in sync by
    /// `ipc::save_settings`.
    pub close_to_tray: Arc<AtomicBool>,
    /// Cached `Settings::edge_snap`, mirrored for the synchronous `Moved`
    /// handler. Kept in sync by `ipc::save_settings`.
    pub edge_snap: Arc<AtomicBool>,
    /// 缓存「Dashboard 处于 compact 胶囊态」。供同步的 `Moved` 处理器选择
    /// 停靠策略（贴边 vs 四向吸附），以及托盘恢复时重新拉起把手。
    pub compact_mode: Arc<AtomicBool>,
    /// 用户正在用原生拖拽移动窗口：前端调用 set_pill_dragging(true) 置位，
    /// 窗口停止移动 400ms 后由 watcher 清零并发事件通知前端结算。
    pub pill_drag: Arc<AtomicBool>,
    /// 窗口最近一次 Moved 的时间戳（epoch ms），供 watcher 判断是否停稳。
    pub pill_last_move_ms: Arc<AtomicU64>,
    /// Holds the tray icon alive for the app lifetime. Never referenced after
    /// construction.
    #[allow(dead_code)]
    pub _tray: TrayIcon,
    /// Cache for aggregated local-tool usage (Claude Code logs). Short TTL so
    /// the tool view isn't re-parsing hundreds of session files on every poll.
    pub local: local::SharedLocalCache,
}

/// Build a live provider instance for one account. Each account gets its own
/// instance, so multiple accounts on the same kind are fully isolated in
/// storage (keyed by `instance_id`) and in the frontend.
pub fn build_account_provider(
    account: &AccountMeta,
    http: Client,
    storage: Arc<crate::storage::Storage>,
) -> Option<Arc<dyn providers::Provider>> {
    providers::build_provider(
        &account.provider_kind,
        http,
        storage,
        account.instance_id.clone(),
        account.label.clone(),
    )
}

/// Build the provider registry from the user's configured accounts. Only known
/// kinds are instantiated; unknown kinds are skipped.
pub(crate) fn build_registry(
    accounts: &[AccountMeta],
    http: Client,
    storage: Arc<storage::Storage>,
) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    for account in accounts {
        if let Some(p) = build_account_provider(account, http.clone(), storage.clone()) {
            registry.insert(p);
        }
    }
    registry
}

/// Build the shared HTTP client. `proxy_url` comes from `Settings::proxy_url`
/// and may be empty (no proxy configured). A non-empty value must parse as an
/// `http(s)`, `socks5` or `socks5h` (remote DNS) proxy URL (`user:pass@host:port`
/// supported). An explicit proxy replaces env-var proxy detection so the
/// setting is deterministic; with no proxy the previous env-aware default is
/// kept. Errors are returned as display strings for `test_proxy`.
pub fn build_http_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder().timeout(std::time::Duration::from_secs(15));
    if let Some(url) = proxy_url.map(str::trim).filter(|u| !u.is_empty()) {
        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("invalid proxy url {url:?}: {e}"))?;
        builder = builder.no_proxy().proxy(proxy);
    }
    builder.build().map_err(|e| format!("building reqwest client failed: {e}"))
}

/// Epoch 毫秒时间戳。拖拽结束检测用它判断窗口是否已停止移动。
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 左键当前是否按下。原生拖拽是系统模态循环，前端拿不到 pointerup，只有按键
/// 状态能权威判定「拖拽已经结束」；仅凭窗口停稳会在用户按住不动的中途误判。
#[cfg(windows)]
pub(crate) fn lbutton_down() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    // 最高位为 1 表示当前按下。
    unsafe { GetAsyncKeyState(0x01) < 0 }
}

#[cfg(not(windows))]
pub(crate) fn lbutton_down() -> bool {
    false
}

/// Load any saved credentials for the configured accounts into the in-memory
/// cache. Failures (no creds saved, keyring unavailable) are silently skipped
/// - the cache just stays empty and the provider returns NotConfigured.
fn load_credentials_into_cache(
    settings_store: &settings::SettingsStore,
    accounts: &[AccountMeta],
) -> HashMap<String, Credentials> {
    let mut map = HashMap::new();
    for account in accounts {
        if let Some(creds) = settings_store.load_credentials(&account.instance_id) {
            map.insert(account.instance_id.clone(), creds);
        }
    }
    map
}

/// Total local-tool tokens from the in-memory scan cache (0 if not yet cached).
async fn cached_tool_tokens(local: &local::SharedLocalCache) -> f64 {
    if let Some(p) = local.cached().await {
        p.tools.iter().flat_map(|t| t.daily.iter()).map(|d| d.total).sum()
    } else {
        0.0
    }
}

/// Number of local tools from the in-memory scan cache.
async fn cached_tool_count(local: &local::SharedLocalCache) -> u64 {
    if let Some(p) = local.cached().await {
        p.tools.len() as u64
    } else {
        0
    }
}

/// 从托盘恢复主面板：显示 + 聚焦；若此前处于 compact 胶囊态，把贴边把手
/// 一并拉起并对齐（close-to-tray 会把把手一起隐藏，否则胶囊无法被唤醒）。
fn show_dashboard(app: &AppHandle) {
    let Some(window) = app.get_webview_window("dashboard") else { return; };
    let _ = window.show();
    let _ = window.set_focus();
    if app.state::<AppState>().compact_mode.load(Ordering::SeqCst) {
        // compact 主窗被恢复时仍是「停靠 + 穿透 + 内容滑出窗外」的不可见态，
        // 因此同时请前端把胶囊唤出来（把手只是备选入口）。
        let _ = window.emit("peek-reveal", ());
        ipc::reposition_peek(&window);
        if let Some(peek) = app.get_webview_window("peek") {
            let _ = peek.show();
        }
    }
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=warn")),
        )
        .try_init();

    tauri::Builder::default()
        // 单例插件必须注册在最前：第二实例启动时由它唤起本实例的 dashboard
        // 后自动退出，其余插件只在唯一实例内初始化。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二实例启动时本回调在第一实例内执行，第二实例进程随后由插件
            // 自动退出；面板此前收进托盘也能被唤起。
            if let Some(dash) = app.get_webview_window("dashboard") {
                let _ = dash.unminimize();
                let _ = dash.show();
                let _ = dash.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Restore window POSITION only. Restoring SIZE poisons startup: the
        // saved size can come from an older build or the other UI mode, while
        // the frontend always boots in dashboard mode, so the startup size
        // must come from tauri.conf.json (mode switches go through
        // set_window_mode in ipc.rs).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(tauri_plugin_window_state::StateFlags::POSITION)
                .build(),
        )
        .setup(|app| {
            // Resolve OS app-data dir and open the SQLite store there.
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app_data_dir should be resolvable");
            let db_path = data_dir.join("token_usage_monitor.db");
            let store = Arc::new(
                storage::Storage::open(&db_path).expect("opening SQLite storage"),
            );
            let settings_store = settings::SettingsStore::new(data_dir)
                .expect("loading settings store");

            // 启动自愈：自启开启时注册表项可能被用户或安全软件清除，启动时
            // 按设置对账一次；失败不阻断启动。
            {
                use tauri_plugin_autostart::ManagerExt;
                let s = settings_store.read_blocking();
                if s.autostart {
                    let autolaunch = app.autolaunch();
                    if !autolaunch.is_enabled().unwrap_or(false) {
                        let _ = autolaunch.enable();
                    }
                }
            }

            // Shared, swappable HTTP client. A saved proxy URL that fails to
            // build falls back to a direct client instead of aborting launch.
            let initial_proxy = settings_store.read_blocking().proxy_url.clone();
            let initial_client =
                crate::build_http_client(initial_proxy.as_deref()).unwrap_or_else(|_| {
                    Client::builder()
                        .timeout(std::time::Duration::from_secs(15))
                        .build()
                        .expect("building reqwest client")
                });

            let registry = {
                // First-load the settings to learn the configured accounts.
                let accounts = settings_store.read_blocking().accounts;
                let reg = build_registry(&accounts, initial_client.clone(), store.clone());
                Arc::new(RwLock::new(reg))
            };
            let http = Arc::new(RwLock::new(initial_client));
            let credentials = {
                let accounts = settings_store.read_blocking().accounts;
                load_credentials_into_cache(&settings_store, &accounts)
            };

            // burn / notify shared state.
            let burn: SharedBurnTracker =
                Arc::new(tokio::sync::Mutex::new(Default::default()));
            let notify_state: SharedNotifyState =
                Arc::new(tokio::sync::Mutex::new(Default::default()));
            // Local-tool usage cache (Claude Code logs).
            let local: local::SharedLocalCache =
                Arc::new(local::LocalCache::new());
            // Pre-warm the local-tool cache in the background right after startup
            // (delayed slightly so it doesn't compete with launch), so the first
            // time the user opens the tools view it's served from cache instead of
            // blocking on a multi-hundred-file disk scan.
            {
                let warm = local.clone();
                let store_for_warm = store.clone();
                tauri::async_runtime::spawn(async move {
                    // 延迟放宽：避免和启动争抢，但要尽快热好缓存以减少首开等待。
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    if warm.cached().await.is_none() {
                        let tools = local::scan_all(&store_for_warm);
                        // 与 get_local_tools 同：全量扫描后重建 jsonl 工具续读指针。
                        local::delta::record_tool_offsets(&store_for_warm, "claude-code");
                        local::delta::record_tool_offsets(&store_for_warm, "codex");
                        let sessions: u64 = tools.iter().map(|t| t.session_count).sum();
                        // 一并把 MiniMax token 用量写入 DB，供卡片热力图快速读取。
                        local::persist_minimax(&store_for_warm, &tools);
                        let payload = local::LocalToolsPayload {
                            tools,
                            sessions_parsed: sessions,
                        };
                        warm.store(payload.clone()).await;
                        // 持久化到二级缓存，应用重启后首个工具页也能秒开（无源变化时）。
                        let _ = store_for_warm.save_local_scan_cache(
                            &local::cache::src_fingerprint(),
                            &serde_json::to_string(&payload).unwrap_or_default(),
                        );
                    }
                });
            }

            // 多端同步 hub（B8）：hub 模式在本机端口提供 ingest/devices 服务；
            // agent 模式定期把本机用量上报到远端 hub。
            {
                let start = settings_store.read_blocking();
                if start.hub_mode == "hub" {
                    hub::spawn_hub_server(store.clone(), start.hub_port, start.hub_token.clone());
                }
                if start.hub_mode == "agent" && start.report_on {
                    let store_reporter = store.clone();
                    let local_reporter = local.clone();
                    let settings_reporter = settings_store.clone();
                    let http_reporter = http.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        loop {
                            let s = settings_reporter.read_blocking();
                            if s.report_on && s.hub_mode == "agent" && !s.hub_base.is_empty() {
                                let base = s.hub_base.clone();
                                let token = s.hub_token.clone();
                                let tool_tokens = cached_tool_tokens(&local_reporter).await;
                                let tool_count = cached_tool_count(&local_reporter).await;
                                let daily = hub::tool_daily_from_cache(&local_reporter).await;
                                let provider_count = s.accounts.len() as u64;
                                let (id, host, os, arch, ver) = hub::machine_info();
                                let device = hub::build_device_usage(
                                    &id, &host, &os, &arch, &ver,
                                    tool_tokens, provider_count, tool_count, daily,
                                );
                                let client = http_reporter.read().await.clone();
                                let _ = hub::report_to_hub(
                                    &client,
                                    &base,
                                    &device,
                                    &token,
                                )
                                .await;
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        }
                    });
                }
            }

            // 工具日志 watch + 增量扫描：3s 指纹轮询，只重扫变化的工具并广播
            // tools-updated，让工具/模型/设备页近实时反映新增用量。
            local::watch::spawn_tool_watcher(
                app.handle().clone(),
                local.clone(),
                store.clone(),
            );

            // 汇率定时拉取（B.6）：启动后稍作延迟抓一次，之后每 6 小时检查一次
            // 陈旧状态（缓存超过 24h 才真正打网络），失败静默回落缓存/默认值。
            {
                let store_fx = store.clone();
                let settings_fx = settings_store.clone();
                let http_fx = http.clone();
                let app_fx = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    loop {
                        let s = settings_fx.read_blocking();
                        let client = http_fx.read().await.clone();
                        let snap = exchange::refresh_rates(&store_fx, &s, &client, false).await;
                        match snap.warning {
                            // 数据有效（新拉取或缓存仍新鲜）→ 广播，让所有窗口同步生效表
                            None => {
                                let _ = app_fx.emit("rates-updated", &snap);
                            }
                            Some(w) => tracing::warn!("exchange rate refresh: {w}"),
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
                    }
                });
            }

            // Pause broadcast (initial value false = polling normally). The
            // initial receiver is dropped: tasks subscribe later, and a
            // watch channel accepts new subscribers while the sender lives.
            let (pause_tx, _pause_rx) = watch::channel(false);
            let pause_tx = Arc::new(pause_tx);

            // Settings-change wake ping, same pattern.
            let (settings_wake, _wake_rx) = watch::channel(());
            let settings_wake = Arc::new(settings_wake);

            // Mirror close-to-tray / edge-snap into atomics so the synchronous
            // window-event handlers below can read them without an async lock.
            // ipc::save_settings keeps them in sync when the user edits settings.
            let close_to_tray = Arc::new(AtomicBool::new(settings_store.close_to_tray_now()));
            let edge_snap = Arc::new(AtomicBool::new(settings_store.edge_snap_now()));

            // System tray: lets the app stay resident when the dashboard window
            // is hidden (close-to-tray) and gives a show/quit menu.
            let tray_mgr = app.handle().clone();
            let show_item = MenuItem::with_id(&tray_mgr, "show", "显示面板", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(&tray_mgr, "quit", "退出", true, None::<&str>)?;
            let tray_menu = Menu::with_items(&tray_mgr, &[&show_item, &quit_item])?;
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("default window icon").clone())
                .tooltip("TokenUsageMonitor")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_dashboard(tray.app_handle());
                    }
                })
                .on_menu_event(|app_handle, event| match event.id().as_ref() {
                    "show" => show_dashboard(app_handle),
                    "quit" => app_handle.exit(0),
                    _ => {}
                })
                .build(&tray_mgr)?;

            let state = AppState {
                registry,
                state: Arc::new(RwLock::new(HashMap::new())),
                credentials: Arc::new(RwLock::new(credentials)),
                storage: store,
                settings: Arc::new(settings_store),
                burn,
                notify: notify_state,
                pause_tx,
                settings_wake,
                http,
                poll_locks: Arc::new(RwLock::new(HashMap::new())),
                close_to_tray,
                edge_snap,
                compact_mode: Arc::new(AtomicBool::new(false)),
                pill_drag: Arc::new(AtomicBool::new(false)),
                pill_last_move_ms: Arc::new(AtomicU64::new(0)),
                _tray: tray,
                local,
            };

            app.manage(state);

            // One-shot startup clamp: the first time the dashboard gains focus,
            // pull it back inside the monitor's work area. on_window_event has no
            // unlisten handle, so guard with an AtomicBool swap.
            let dash = app
                .get_webview_window("dashboard")
                .expect("dashboard window must exist");
            // 显示形态是持久的：上次停在 compact 胶囊态就先按胶囊尺寸定好大小，
            // 再显示窗口（配置里 visible=false）—— 否则启动会先闪一个 400x680 的全窗。
            // 恢复后是常态浮动（只把窗口拉回屏内，不贴边），只有用户把它拖到屏幕边缘
            // 松手才由前端判定贴边收起。前端挂载后会按窗宽自行对齐视图，故无需额外通知。
            if app.state::<AppState>().settings.compact_mode_now() {
                let _ = dash.set_size(tauri::LogicalSize::new(168u32, 56u32));
                ipc::clamp_window_to_work_area(&dash, 8.0);
                app.state::<AppState>()
                    .compact_mode
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let _ = dash.show();
            let startup_clamped = std::sync::atomic::AtomicBool::new(false);
            let dash_for_clamp = dash.clone();
            dash.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Focused(true))
                    && !startup_clamped.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    ipc::clamp_window_to_work_area(&dash_for_clamp, 8.0);
                }
            });

            // Close-to-tray: when the user closes the dashboard, hide it to the
            // tray instead of quitting. The tray's "退出" item calls app.exit
            // which bypasses this. Reads the cached flag synchronously.
            let close_flag = dash.clone();
            let close_flag_inner = close_flag.clone();
            let close_to_tray_flag = app.state::<AppState>().close_to_tray.clone();
            close_flag.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if close_to_tray_flag.load(Ordering::SeqCst) {
                        api.prevent_close();
                        // 把手一起收起：胶囊被隐藏后把手无法唤醒任何东西。
                        if let Some(peek) =
                            close_flag_inner.app_handle().get_webview_window("peek")
                        {
                            let _ = peek.hide();
                        }
                        let _ = close_flag_inner.hide();
                    }
                }
            });

            // 拖动时的贴边策略：dashboard 形态沿用普通边缘吸附；胶囊态不做实时吸附，
            // 是否贴边收起只在拖拽停稳后由前端按落点判定，拖拽期间把手也不跟随。
            let snap_dash = dash.clone();
            let snap_dash_inner = snap_dash.clone();
            let edge_snap_flag = app.state::<AppState>().edge_snap.clone();
            let compact_flag = app.state::<AppState>().compact_mode.clone();
            let pill_drag_flag = app.state::<AppState>().pill_drag.clone();
            let last_move = app.state::<AppState>().pill_last_move_ms.clone();
            snap_dash.on_window_event(move |event| {
                if !matches!(event, tauri::WindowEvent::Moved(_)) {
                    return;
                }
                last_move.store(crate::now_ms(), Ordering::SeqCst);
                if compact_flag.load(Ordering::SeqCst) {
                    // 胶囊态不做实时吸附：吸附会和用户拖拽互相拉扯（拖不离边缘），
                    // 是否贴边收起只在松手停稳后由前端按落点判定。
                    // 拖拽期间也不跟随把手，否则把手会沿屏幕边缘乱跳。
                    if !pill_drag_flag.load(Ordering::SeqCst) {
                        ipc::reposition_peek(&snap_dash_inner);
                    }
                } else if edge_snap_flag.load(Ordering::SeqCst) {
                    ipc::snap_to_edges(&snap_dash_inner, 14.0);
                }
            });

            // Windows 11 会在窗口尺寸变化时重画 DWM 边框与圆角：启动时对 400x680
            // 设过一次的「不圆角 + 无边框」属性在缩到 168x56（以及明细展开/收起）后
            // 会失效，四角重新变成可见的矩形。故每次 resize 后重设一次。
            #[cfg(windows)]
            let corner_dash = dash.clone();
            #[cfg(windows)]
            let corner_dash_inner = corner_dash.clone();
            #[cfg(windows)]
            corner_dash.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Resized(_)) {
                    dwm_corner::disable_corner_artifacts(&corner_dash_inner);
                }
            });

            // Windows 11: kill the DWM rounded-corner mask + 1px window
            // border so the four corners of the transparent window stay
            // fully invisible behind the rounded UI content.
            #[cfg(windows)]
            dwm_corner::disable_corner_artifacts(&dash);

            // Spawn the periodic scheduler. It will only poll providers that
            // appear in `Settings::enabled_providers`; unconfigured providers
            // emit `NotConfigured` and keep waiting for credentials.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                scheduler::spawn_all(&handle).await;
            });

            // 拖拽结束检测：原生 startDragging 的模态拖拽会把 mouseup 交给系统，
            // WebView 通常收不到 pointerup，因此由 Rust 观察「窗口停止移动」来判定
            // 手势结束，再通知前端做贴边/浮动结算。
            let settle_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    let Some(state) = settle_handle.try_state::<AppState>() else { continue; };
                    if !state.pill_drag.load(Ordering::SeqCst) {
                        continue;
                    }
                    let last = state.pill_last_move_ms.load(Ordering::SeqCst);
                    // 必须「左键已松开」才算手势结束：用户按住不动超过阈值时，
                    // 只凭停稳会在拖拽中途误判，随后的贴边结算会把窗口拽回边缘。
                    if last == 0
                        || crate::now_ms().saturating_sub(last) < 150
                        || crate::lbutton_down()
                    {
                        continue;
                    }
                    state.pill_drag.store(false, Ordering::SeqCst);
                    if let Some(w) = settle_handle.get_webview_window("dashboard") {
                        let _ = w.emit("pill-drag-settled", ());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_providers,
            ipc::get_usage,
            ipc::get_provider_states,
            ipc::get_heatmap,
            ipc::force_refresh,
            ipc::toggle_polling,
            ipc::set_window_mode,
            ipc::sync_peek_window,
            ipc::set_pill_dragging,
            ipc::open_settings,
            ipc::close_settings,
            ipc::open_trend_window,
            ipc::open_tool_window,
            ipc::get_settings,
            ipc::save_settings,
            ipc::save_credentials,
            ipc::delete_credentials,
            ipc::upsert_account,
            ipc::remove_account,
            ipc::test_provider,
            ipc::test_proxy,
            ipc::detect_codex_token,
            ipc::get_local_tools,
            ipc::get_device_report,
            ipc::get_hub_devices,
            ipc::get_exchange_rates,
            ipc::refresh_exchange_rates,
            ipc::set_autostart,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Windows 11 rounds the corners of top-level windows and draws a 1px
/// border around the window rect. On this transparent, undecorated
/// widget window both render as four faintly visible corners around the
/// rounded content. Disable both via DWM window attributes.
///
/// Raw FFI on purpose: pulling in the `windows` crate just for two
/// attribute calls would recompile half the dependency tree, and this
/// project's windows-gnu toolchain rebuilds are expensive (dlltool/as).
/// dwmapi.dll is resolved at runtime so no import library is needed.
#[cfg(windows)]
pub(crate) mod dwm_corner {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWCP_DONOTROUND: u32 = 1;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    type DwmSetWindowAttributeFn =
        unsafe extern "system" fn(isize, u32, *const c_void, u32) -> i32;

    fn dwm_set_window_attribute() -> Option<DwmSetWindowAttributeFn> {
        static DWM_FN: OnceLock<Option<DwmSetWindowAttributeFn>> = OnceLock::new();
        *DWM_FN.get_or_init(|| unsafe {
            let module_name: Vec<u16> = "dwmapi.dll\0".encode_utf16().collect();
            let module = LoadLibraryW(module_name.as_ptr());
            if module.is_null() {
                return None;
            }
            let proc_name = b"DwmSetWindowAttribute\0";
            let proc = GetProcAddress(module, proc_name.as_ptr());
            if proc.is_null() {
                return None;
            }
            Some(std::mem::transmute::<*mut c_void, DwmSetWindowAttributeFn>(proc))
        })
    }

    /// Best-effort: failures are ignored (unsupported/older DWM simply
    /// keeps the default appearance).
    pub fn disable_corner_artifacts(window: &tauri::WebviewWindow) {
        let Some(dwm_set_window_attribute) = dwm_set_window_attribute() else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let hwnd = hwnd.0 as isize;
        unsafe {
            let preference = DWMWCP_DONOTROUND;
            dwm_set_window_attribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            let border = DWMWA_COLOR_NONE;
            dwm_set_window_attribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &border as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

#[cfg(all(test, windows, target_env = "gnu"))]
mod windows_test_manifest {
    #[link(name = "cargo_test_manifest_res", kind = "static", modifiers = "+whole-archive")]
    extern "C" {}
}

#[cfg(test)]
mod http_client_tests {
    use super::*;

    #[test]
    fn none_and_blank_build_without_proxy() {
        assert!(build_http_client(None).is_ok());
        assert!(build_http_client(Some("")).is_ok());
        assert!(build_http_client(Some("   ")).is_ok());
    }

    #[test]
    fn valid_http_and_socks_urls_are_accepted() {
        assert!(build_http_client(Some("http://127.0.0.1:7890")).is_ok());
        assert!(build_http_client(Some("http://user:pass@127.0.0.1:7890")).is_ok());
        assert!(build_http_client(Some("socks5://127.0.0.1:1080")).is_ok());
    }

    #[test]
    fn invalid_proxy_string_is_rejected() {
        assert!(build_http_client(Some("not a proxy")).is_err());
    }
}
