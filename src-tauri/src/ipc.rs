//! Tauri IPC commands exposed to the frontend.
//!
//! Naming convention: snake_case in Rust, frontend calls via `invoke('get_usage')`.

use crate::build_account_provider;
use crate::providers::{
    AccountMeta, Credentials, HeatmapCell, PRESETS, Preset, ProviderState, UsageSnapshot,
};
use crate::settings::Settings;
use crate::AppState;
use std::collections::HashMap;
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
    .inner_size(680.0, 780.0)
    .resizable(true)
    .min_inner_size(540.0, 600.0)
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

    // Mirror the cached window-behavior flags so the synchronous window-event
    // handlers (close-to-tray, edge snap) pick up the change immediately.
    state.close_to_tray.store(new_settings.close_to_tray, std::sync::atomic::Ordering::SeqCst);
    state.edge_snap.store(new_settings.edge_snap, std::sync::atomic::Ordering::SeqCst);

    // Reconcile the live registry with the saved accounts: register newly added
    // accounts (and spawn a polling task), drop stale snapshots for accounts
    // that were removed or disabled. Enabled/interval edits are picked up by
    // the existing poll loops via the settings_wake ping below.
    let account_ids: std::collections::HashSet<String> =
        new_settings.accounts.iter().map(|a| a.instance_id.clone()).collect();
    let mut registry = state.registry.write().await;
    for account in &new_settings.accounts {
        if registry.get(&account.instance_id).is_none() {
            if let Some(provider) =
                build_account_provider(account, state.http.clone(), state.storage.clone())
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
        if let Some(provider) =
            build_account_provider(&account, state.http.clone(), state.storage.clone())
        {
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
    let provider = build_account_provider(&account, state.http.clone(), state.storage.clone())
        .ok_or_else(|| format!("unknown provider kind: {provider_kind}"))?;
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
