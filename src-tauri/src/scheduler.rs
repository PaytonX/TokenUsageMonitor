//! Periodic usage fetcher.
//!
//! Spawns one `tokio::task` per provider. Each task ticks on its provider's
//! configured interval and pushes updated `UsageSnapshot`s via the
//! `usage-updated` Tauri event. Failures don't kill the task - they emit
//! `provider-error` and the next tick tries again.

use crate::providers::{
    BurnInfo, Credentials, Provider, ProviderState, UsageSnapshot, UsageUpdate, WindowUsage,
};
use crate::AppState;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};
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

/// Max gap between two snapshots that still allows a burn diff (seconds).
const MAX_BURN_GAP_SECS: i64 = 1800;
/// Sanity ceiling for burn rate (units/minute). Above this, treat the diff
/// as a reset/corruption rather than real consumption.
const MAX_BURN_RATE_PER_MIN: f64 = 10_000_000.0;

/// Ordered (stable key, window) pairs for reset detection and totals.
fn diff_windows(
    snap: &UsageSnapshot,
) -> [(&'static str, Option<&WindowUsage>); 4] {
    [
        ("five_hour", snap.windows.five_hour.as_ref()),
        ("daily", snap.windows.daily.as_ref()),
        ("weekly", snap.windows.weekly.as_ref()),
        ("monthly", snap.windows.monthly.as_ref()),
    ]
}

/// Pick the window with a known quota and the highest used percentage.
/// Returns a stable key plus the window, used both for burn diff and for
/// detecting a window reset (key changes between two samples).
fn most_critical_window(snap: &UsageSnapshot) -> Option<(&'static str, &WindowUsage)> {
    diff_windows(snap)
        .into_iter()
        .filter_map(|(key, w)| w.map(|w| (key, w)))
        .filter(|(_, w)| w.quota > 0.0)
        .max_by(|(_, a), (_, b)| {
            a.percent()
                .partial_cmp(&b.percent())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Pure burn diff between two consecutive snapshots of the same provider.
/// Returns None on reset, missing window, oversized gap, non-positive or
/// absurd rate.
pub fn compute_burn(prev: &UsageSnapshot, cur: &UsageSnapshot) -> Option<BurnInfo> {
    let (prev_key, prev_w) = most_critical_window(prev)?;
    let (cur_key, cur_w) = most_critical_window(cur)?;
    if prev_key != cur_key {
        return None;
    }
    let elapsed = (cur.timestamp - prev.timestamp).num_seconds();
    if elapsed <= 0 || elapsed > MAX_BURN_GAP_SECS {
        return None;
    }
    let delta = cur_w.used - prev_w.used;
    let rate = delta / (elapsed as f64) * 60.0;
    if !rate.is_finite() || rate <= 0.0 || rate > MAX_BURN_RATE_PER_MIN {
        return None;
    }
    let eta_seconds = if cur_w.quota > cur_w.used {
        // round to nearest second: the f64 pipeline (rate already rounded
        // once) can yield 479.9999... for a true 480s ETA, and plain `as u64`
        // truncation would display 479. ETA is a display value, so nearest-
        // second rounding is also more correct than truncation.
        Some((((cur_w.quota - cur_w.used) / rate) * 60.0).round() as u64)
    } else {
        None
    };
    Some(BurnInfo {
        rate_per_min: rate,
        unit: cur_w.unit,
        eta_seconds,
    })
}

/// Per-provider previous-snapshot memory for burn diffing.
#[derive(Default)]
pub struct BurnTracker {
    prev: HashMap<String, UsageSnapshot>,
}

impl BurnTracker {
    /// Feed a fresh snapshot; returns the burn info (if any) and whether the
    /// provider is actively generating (used grew since the previous sample).
    pub fn observe(&mut self, id: &str, snap: &UsageSnapshot) -> (Option<BurnInfo>, bool) {
        let burn = self
            .prev
            .get(id)
            .and_then(|prev| compute_burn(prev, snap));
        let active = match self.prev.get(id) {
            Some(prev) => total_used(snap) > total_used(prev),
            None => false,
        };
        self.prev.insert(id.to_string(), snap.clone());
        (burn, active)
    }
}

fn total_used(snap: &UsageSnapshot) -> f64 {
    diff_windows(snap)
        .into_iter()
        .filter_map(|(_, w)| w)
        .map(|w| if w.quota > 0.0 { w.used } else { 0.0 })
        .sum()
}

/// Shared handle stored in AppState so both scheduler and force_refresh diff
/// against the same baseline.
pub type SharedBurnTracker = Arc<Mutex<BurnTracker>>;

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

#[cfg(test)]
mod burn_tests {
    use super::*;
    use crate::providers::{UsageUnit, UsageWindows, WindowUsage};

    fn window(used: f64, quota: f64) -> WindowUsage {
        WindowUsage {
            used,
            quota,
            unit: UsageUnit::Tokens,
            reset_at: None,
            over_quota: false,
        }
    }

    fn snap_with_monthly(used: f64, quota: f64, ts: chrono::DateTime<chrono::Utc>) -> crate::providers::UsageSnapshot {
        crate::providers::UsageSnapshot {
            provider_id: "p".to_string(),
            provider_display_name: "P".to_string(),
            plan_tier: None,
            timestamp: ts,
            windows: UsageWindows {
                monthly: Some(window(used, quota)),
                ..Default::default()
            },
            heatmap: None,
        }
    }

    #[test]
    fn normal_growth_produces_rate_and_eta() {
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(60);
        let t1 = chrono::Utc::now();
        let prev = snap_with_monthly(1000.0, 10_000.0, t0);
        let cur = snap_with_monthly(2000.0, 10_000.0, t1);
        let burn = compute_burn(&prev, &cur).expect("burn expected");
        assert!((burn.rate_per_min - 1000.0).abs() < 1.0);
        // remaining 8000 at 1000/min -> 8 min -> 480s
        assert_eq!(burn.eta_seconds, Some(480));
    }

    #[test]
    fn window_reset_or_missing_window_resets_baseline() {
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(60);
        let t1 = chrono::Utc::now();
        let prev = snap_with_monthly(9000.0, 10_000.0, t0);
        // current snapshot has no monthly window -> cannot diff same window
        let cur = crate::providers::UsageSnapshot {
            provider_id: "p".to_string(),
            provider_display_name: "P".to_string(),
            plan_tier: None,
            timestamp: t1,
            windows: UsageWindows::default(),
            heatmap: None,
        };
        assert!(compute_burn(&prev, &cur).is_none());
    }

    #[test]
    fn gap_over_1800_seconds_is_none() {
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(2000);
        let t1 = chrono::Utc::now();
        let prev = snap_with_monthly(1000.0, 10_000.0, t0);
        let cur = snap_with_monthly(5000.0, 10_000.0, t1);
        assert!(compute_burn(&prev, &cur).is_none());
    }

    #[test]
    fn non_positive_rate_is_none() {
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(60);
        let t1 = chrono::Utc::now();
        let prev = snap_with_monthly(5000.0, 10_000.0, t0);
        let cur = snap_with_monthly(4000.0, 10_000.0, t1);
        assert!(compute_burn(&prev, &cur).is_none());
    }

    #[test]
    fn absurdly_large_rate_is_none() {
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(60);
        let t1 = chrono::Utc::now();
        let prev = snap_with_monthly(0.0, 1_000_000_000.0, t0);
        let cur = snap_with_monthly(500_000_000.0, 1_000_000_000.0, t1);
        assert!(compute_burn(&prev, &cur).is_none());
    }
}
