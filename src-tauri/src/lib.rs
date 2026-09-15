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
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

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

            let state = AppState {
                registry,
                state: Arc::new(RwLock::new(HashMap::new())),
                credentials: Arc::new(RwLock::new(credentials)),
                storage: store,
                settings: Arc::new(settings_store),
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
