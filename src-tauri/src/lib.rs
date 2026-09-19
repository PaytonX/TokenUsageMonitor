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

pub mod ipc;
pub mod notify;
pub mod providers;
pub mod scheduler;
pub mod settings;
pub mod signing;
pub mod storage;

use providers::{
    Credentials, ProviderRegistry, SharedProviderState,
    deepseek::DeepSeekProvider, minimax::MiniMaxProvider,
    volcengine::VolcengineProvider,
};
use notify::SharedNotifyState;
use reqwest::Client;
use scheduler::SharedBurnTracker;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tokio::sync::{watch, RwLock};

/// Global application state shared across IPC commands and the scheduler.
pub struct AppState {
    /// All known providers, keyed by `Provider::id()`.
    pub registry: ProviderRegistry,
    /// Latest snapshot + last error per provider.
    pub state: SharedProviderState,
    /// User-configured credentials per provider id. Populated at startup
    /// from `SettingsStore::load_credentials` and kept in sync by Settings UI
    /// via `save_credentials` / `delete_credentials`.
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
    /// Cached `Settings::close_to_tray`, mirrored so the synchronous window
    /// close interceptor can read it without an async lock. Kept in sync by
    /// `ipc::save_settings`.
    pub close_to_tray: Arc<AtomicBool>,
    /// Cached `Settings::edge_snap`, mirrored for the synchronous `Moved`
    /// handler. Kept in sync by `ipc::save_settings`.
    pub edge_snap: Arc<AtomicBool>,
    /// Holds the tray icon alive for the app lifetime. Never referenced after
    /// construction.
    #[allow(dead_code)]
    pub _tray: TrayIcon,
}

/// The set of provider ids the app knows how to register. Anything in the
/// user's `enabled_providers` list (or any unknown id) is ignored unless it
/// appears here, so users can't accidentally enable a non-existent provider.
const KNOWN_PROVIDER_IDS: &[&str] = &["minimax", "deepseek", "volcengine"];

/// Build the provider registry. We always register every known provider so
/// `test_provider` and `get_providers` can list them in the Settings UI -
/// the scheduler only polls providers whose id is in
/// `Settings::enabled_providers`.
fn build_registry(http: Client, storage: Arc<storage::Storage>) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(MiniMaxProvider::new(http.clone(), storage.clone()));
    registry.register(DeepSeekProvider::new(http.clone(), storage.clone()));
    registry.register(VolcengineProvider::new(http));
    registry
}

/// Load any saved credentials for the known providers into the in-memory
/// cache. Failures (no creds saved, keyring unavailable) are silently skipped
/// - the cache just stays empty and the provider returns NotConfigured.
fn load_credentials_into_cache(
    settings_store: &settings::SettingsStore,
) -> HashMap<String, Credentials> {
    let mut map = HashMap::new();
    for id in KNOWN_PROVIDER_IDS {
        if let Some(creds) = settings_store.load_credentials(id) {
            map.insert((*id).to_string(), creds);
        }
    }
    map
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=warn")),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
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

            // Single shared HTTP client with sensible defaults.
            let http = Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("building reqwest client");

            let registry = build_registry(http, store.clone());
            let credentials = load_credentials_into_cache(&settings_store);

            // burn / notify shared state.
            let burn: SharedBurnTracker =
                Arc::new(tokio::sync::Mutex::new(Default::default()));
            let notify_state: SharedNotifyState =
                Arc::new(tokio::sync::Mutex::new(Default::default()));

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
                        if let Some(w) = tray.app_handle().get_webview_window("dashboard") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .on_menu_event(|app_handle, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app_handle.get_webview_window("dashboard") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
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
                close_to_tray,
                edge_snap,
                _tray: tray,
            };

            app.manage(state);

            // One-shot startup clamp: the first time the dashboard gains focus,
            // pull it back inside the monitor's work area. on_window_event has no
            // unlisten handle, so guard with an AtomicBool swap.
            let dash = app
                .get_webview_window("dashboard")
                .expect("dashboard window must exist");
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
                        let _ = close_flag_inner.hide();
                    }
                }
            });

            // Edge snap: while the user drags the dashboard, when it comes near
            // a screen edge snap to that edge. Reads the cached flag synchronously.
            let snap_dash = dash.clone();
            let snap_dash_inner = snap_dash.clone();
            let edge_snap_flag = app.state::<AppState>().edge_snap.clone();
            snap_dash.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Moved(_))
                    && edge_snap_flag.load(Ordering::SeqCst)
                {
                    ipc::snap_to_edges(&snap_dash_inner, 14.0);
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
            ipc::open_settings,
            ipc::close_settings,
            ipc::get_settings,
            ipc::save_settings,
            ipc::save_credentials,
            ipc::delete_credentials,
            ipc::test_provider,
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
mod dwm_corner {
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