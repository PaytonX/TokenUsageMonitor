//! Periodic usage fetcher.
//!
//! Spawns one `tokio::task` per provider. Each task ticks on its provider's
//! configured interval and pushes updated `UsageSnapshot`s via the
//! `usage-updated` Tauri event. Failures don't kill the task - they emit
//! `provider-error` and the next tick tries again.

use crate::providers::{Credentials, Provider, ProviderState};
use crate::AppState;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use tokio::time::{interval_at, Instant};

/// Default per-provider polling intervals (seconds). Tunable via Settings later.
pub const DEFAULT_INTERVAL_MINIMAX: u64 = 300; // 5 min
pub const DEFAULT_INTERVAL_DEEPSEEK: u64 = 300; // 5 min
pub const DEFAULT_INTERVAL_VOLCENGINE: u64 = 300; // 5 min

pub fn default_interval_for(provider_id: &str) -> u64 {
    match provider_id {
        "minimax" => DEFAULT_INTERVAL_MINIMAX,
        "deepseek" => DEFAULT_INTERVAL_DEEPSEEK,
        "volcengine" => DEFAULT_INTERVAL_VOLCENGINE,
        _ => 300,
    }
}

/// Spawn a polling task per registered provider. Each task is fire-and-forget
/// for the lifetime of the app and re-reads the enabled set on every tick, so
/// toggling a provider in Settings takes effect without a restart: disabled
/// providers stop polling entirely (an unconditionally-polling task would
/// keep emitting `usage-updated` and re-inserting snapshots, resurrecting the
/// dashboard card the user just disabled), and newly enabled providers pick
/// up polling on their next tick.
pub async fn spawn_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let providers = state.registry.list();
    for provider in providers {
        let id = provider.id().to_string();
        let app = app.clone();
        let provider = provider.clone();
        // First tick happens immediately so users see data on launch without
        // waiting the full interval. Enabled gating happens inside the loop.
        let start = Instant::now() + Duration::from_secs(1);
        let mut tick = interval_at(start, Duration::from_secs(default_interval_for(&id)));
        tauri::async_runtime::spawn(async move {
            loop {
                tick.tick().await;
                let enabled_now = app
                    .state::<AppState>()
                    .settings
                    .get()
                    .await
                    .enabled_providers
                    .contains(&id);
                if !enabled_now {
                    continue;
                }
                if let Err(e) = poll_one(&app, &provider).await {
                    tracing::warn!(provider = %id, error = %e, "poll failed");
                }
            }
        });
    }
}

/// Polls a single provider and updates state + emits events.
pub async fn poll_one(
    app: &AppHandle,
    provider: &Arc<dyn Provider>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tauri::Manager;
    let state = app.state::<AppState>();

    // Pull credentials. Missing credentials surface as NotConfigured rather
    // than a network error. We check the in-memory cache first (fast path),
    // then fall back to the OS credential store (covers the case where
    // credentials were saved in a previous app session and the cache hasn't
    // been populated yet for this provider).
    let creds = {
        let cache = state.credentials.read().await;
        cache.get(provider.id()).cloned()
    };
    let creds = creds
        .or_else(|| state.settings.load_credentials(provider.id()))
        .unwrap_or(Credentials::BearerKey {
            api_key: "mock".to_string(),
        });

    let id = provider.id().to_string();
    let result = provider.fetch_usage(&creds).await;
    let mut guard = state.state.write().await;
    match result {
        Ok(snapshot) => {
            let _ = app.emit("usage-updated", &snapshot);
            guard.insert(
                id,
                ProviderState {
                    snapshot: Some(snapshot),
                    last_error: None,
                    last_updated_at: Some(Utc::now()),
                },
            );
        }
        Err(err) => {
            let _ = app.emit(
                "provider-error",
                &serde_json::json!({ "id": id, "error": &err }),
            );
            // Don't overwrite the last good snapshot - keep showing stale data
            // with an error badge.
            let entry = guard.entry(id.clone()).or_default();
            entry.last_error = Some(err);
            entry.last_updated_at = Some(Utc::now());
        }
    }
    Ok(())
}

/// Type alias to satisfy Rust's borrow checker at call sites that need a
/// read-only view of the snapshot map.
pub type SnapshotMap = Arc<RwLock<HashMap<String, ProviderState>>>;
