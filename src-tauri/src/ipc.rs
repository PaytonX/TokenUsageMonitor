//! Tauri IPC commands exposed to the frontend.
//!
//! Naming convention: snake_case in Rust, frontend calls via `invoke('get_usage')`.

use crate::providers::{
    Credentials, HeatmapCell, ProviderState, UsageSnapshot,
};
use crate::settings::Settings;
use crate::AppState;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WebviewWindowBuilder};

/// Lightweight provider metadata for the Settings UI.
#[derive(serde::Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub auth_kind: String,
    /// Whether the user has saved credentials for this provider.
    pub has_credentials: bool,
    /// Whether the user has enabled this provider in Settings.
    pub enabled: bool,
}

/// Returns all available providers + their configuration state.
/// Used by both Dashboard (to show which providers are active) and Settings
/// (to render the credential form list).
#[tauri::command]
pub async fn get_providers(state: State<'_, AppState>) -> Result<Vec<ProviderInfo>, String> {
    let providers = state.registry.list();
    let settings = state.settings.get().await;
    Ok(providers
        .iter()
        .map(|p| {
            let id = p.id();
            let has_creds = state.settings.load_credentials(id).is_some();
            ProviderInfo {
                id: id.to_string(),
                display_name: p.display_name().to_string(),
                auth_kind: match p.auth_kind() {
                    crate::providers::AuthKind::BearerKey => "bearer_key".to_string(),
                    crate::providers::AuthKind::AccessKeySecret => "access_key_secret".to_string(),
                },
                has_credentials: has_creds,
                enabled: settings.enabled_providers.iter().any(|s| s == id),
            }
        })
        .collect())
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

/// Heatmap placeholder - real impl lands in step 8 (HeatmapGrid) backed by SQLite.
#[tauri::command]
pub async fn get_heatmap(
    _state: State<'_, AppState>,
    _provider_id: String,
    _days: u32,
) -> Result<Vec<HeatmapCell>, String> {
    // Step 8 will read from SQLite daily_snapshots.
    Ok(Vec::new())
}

/// Trigger an immediate refresh for one provider (or all if `provider_id` is None).
/// Emits `usage-updated` events as snapshots come in.
///
/// Credentials are now sourced from the OS credential store via SettingsStore.
/// If credentials are missing, the provider returns `ProviderError::NotConfigured`.
#[tauri::command]
pub async fn force_refresh(
    state: State<'_, AppState>,
    app: AppHandle,
    provider_id: Option<String>,
) -> Result<(), String> {
    // Read enabled list so we don't poll (and emit errors for) providers the
    // user has just turned off in Settings.
    let settings = state.settings.get().await;
    let enabled: std::collections::HashSet<String> =
        settings.enabled_providers.iter().cloned().collect();
    drop(settings);

    let targets: Vec<_> = match provider_id {
        Some(ref id) => state
            .registry
            .get(id)
            .map(|p| vec![p])
            .ok_or_else(|| format!("unknown provider: {id}"))?,
        None => state
            .registry
            .list()
            .into_iter()
            .filter(|p| enabled.contains(p.id()))
            .collect(),
    };

    for provider in targets {
        // Pull credentials from OS store (None if user hasn't configured them yet).
        let creds = match state.settings.load_credentials(provider.id()) {
            Some(c) => c,
            None => Credentials::BearerKey {
                api_key: "mock".to_string(),
            },
        };

        let result = provider.fetch_usage(&creds).await;
        let id = provider.id().to_string();
        let mut guard = state.state.write().await;
        match result {
            Ok(snapshot) => {
                let _ = app.emit("usage-updated", &snapshot);
                guard.insert(
                    id.clone(),
                    ProviderState {
                        snapshot: Some(snapshot),
                        last_error: None,
                        last_updated_at: Some(chrono::Utc::now()),
                    },
                );
            }
            Err(err) => {
                let _ = app.emit(
                    "provider-error",
                    &serde_json::json!({ "id": id, "error": &err }),
                );
                guard.insert(
                    id.clone(),
                    ProviderState {
                        snapshot: None,
                        last_error: Some(err),
                        last_updated_at: Some(chrono::Utc::now()),
                    },
                );
            }
        }
    }

    Ok(())
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

/// Switch the dashboard window between full and compact modes.
#[tauri::command]
pub async fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    use tauri::LogicalSize;

    let window = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    match mode.as_str() {
        "dashboard" => {
            window
                .set_size(LogicalSize::new(360u32, 600u32))
                .map_err(|e| e.to_string())?;
        }
        "compact" => {
            // Mini pill: single row 150x44 (ring + name + divider + dot + close).
            window
                .set_size(LogicalSize::new(150u32, 44u32))
                .map_err(|e| e.to_string())?;
        }
        other => return Err(format!("unknown window mode: {other}")),
    }

    clamp_window_to_work_area(&window, 8.0);
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
    .title("TokenUsageMonitor · 设置")
    .inner_size(520.0, 640.0)
    .resizable(true)
    .min_inner_size(420.0, 480.0)
    .visible(false)
    .decorations(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

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

// --- Settings & credentials CRUD --------------------------------------------

/// Returns the current `Settings` (config.toml snapshot).
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.get().await)
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
    state
        .settings
        .save(new_settings.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Drop snapshots for providers the user just disabled. Otherwise the
    // dashboard keeps showing them until the next restart, which feels broken.
    let enabled: std::collections::HashSet<String> =
        new_settings.enabled_providers.iter().cloned().collect();
    let mut guard = state.state.write().await;
    guard.retain(|id, _| enabled.contains(id));
    drop(guard);

    // Tell every window (the dashboard listens for this) that settings
    // changed, so it can re-pull usage and drop cards for disabled providers
    // immediately instead of waiting for a poll that no longer happens.
    let _ = app.emit("settings-changed", &new_settings);
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

/// Tests connectivity for a provider by performing a one-shot fetch with the
/// supplied credentials (without persisting them). Returns either
/// `Ok(snapshot)` for the frontend to preview, or `Err(message)`.
///
/// This lets the Settings UI show a "test connection" status before saving.
#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    provider_id: String,
    creds: Credentials,
) -> Result<UsageSnapshot, String> {
    let provider = state
        .registry
        .get(&provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    provider
        .fetch_usage(&creds)
        .await
        .map_err(|e| e.to_string())
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
