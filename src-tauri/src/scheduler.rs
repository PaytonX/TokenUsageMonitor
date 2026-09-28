//! Periodic usage fetcher.
//!
//! Spawns one `tokio::task` per provider. Each task ticks on its provider's
//! effective interval (a positive global `poll_interval_seconds` in Settings
//! overrides the per-provider default) and pushes a `UsageUpdate` payload
//! (snapshot plus burn info) via the `usage-updated` Tauri event. Every poll
//! also feeds the burn tracker and evaluates threshold notifications.
//!
//! The loop reacts live to a `settings_wake` watch (re-reads enabled state and
//! rebuilds the interval on period change) and skips ticking while the global
//! `pause_tx` watch is set. Failures don't kill the task - they emit
//! `provider-error` (keeping the previous snapshot) and the next tick tries
//! again. [`poll_one`] is shared by both the loop and the `force_refresh`
//! command.

use crate::notify::{self, NotifyAction};
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
use tokio::time::{interval_at, Instant, MissedTickBehavior};

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

/// Effective polling period: a positive global override in Settings wins;
/// 0 means "use the per-provider default".
fn effective_interval(provider_id: &str, global_seconds: u32) -> Duration {
    let secs = if global_seconds > 0 {
        global_seconds as u64
    } else {
        default_interval_for(provider_id)
    };
    Duration::from_secs(secs)
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
        // round to nearest second: deriving seconds back through the f64
        // pipeline (delta/elapsed*60, then remaining/rate*60) can yield
        // 479.9999... for a true 480s ETA, and plain `as u64` truncation would
        // display 479. ETA is a display value, so nearest-second rounding is
        // also more correct than truncation.
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

/// Per-account-id async mutexes serializing whole polls (fetch + balance-delta
/// accounting).
pub type PollLocks =
    Arc<tokio::sync::RwLock<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>;

/// Get (creating if absent) the mutex entry for one account id.
fn lock_entry(
    map: &mut HashMap<String, Arc<Mutex<()>>>,
    id: &str,
) -> Arc<Mutex<()>> {
    map.entry(id.to_string()).or_default().clone()
}

/// Get (creating if absent) the mutex serializing polls for one account
/// id. Keyed by id (not by provider Arc) on purpose: when a proxy change
/// rebuilds the registry, the old loop's in-flight fetch and the new
/// loop's first fetch contend on the same mutex, so balance-delta
/// accounting (read baseline -> accumulate -> write baseline) can never
/// interleave and double-count.
pub async fn poll_lock_for(state: &AppState, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = state.poll_locks.write().await;
    lock_entry(&mut map, id)
}

/// Spawn a polling task per registered provider. Each task is fire-and-forget
/// for the lifetime of the app. The loop inside re-reads settings on every
/// tick and on every settings-changed ping, so toggling a provider, changing
/// the interval, or pausing takes effect without a restart.
pub async fn spawn_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let providers = state.registry.read().await.list();
    for provider in providers {
        let id = provider.id();
        let app = app.clone();
        let provider = provider.clone();
        tauri::async_runtime::spawn(async move {
            poll_loop(app, provider, id).await;
        });
    }
}

/// Spawn a single polling task for a newly-added account at runtime (used by
/// `ipc::upsert_account`). Idempotent per account id.
pub fn spawn_one(app: AppHandle, provider: Arc<dyn Provider>, id: String) {
    tauri::async_runtime::spawn(async move {
        poll_loop(app, provider, id.clone()).await;
    });
}

/// Pure core of `still_registered`: a loop is live only while `provider` is
/// still the instance registered under `id`. A removed account (no entry) or
/// a rebuilt/replaced instance (different `Arc`, e.g. after a proxy change
/// or an account edit) makes this loop stale.
fn is_current(
    registry: &crate::providers::ProviderRegistry,
    provider: &Arc<dyn Provider>,
    id: &str,
) -> bool {
    registry
        .get(id)
        .map(|current| Arc::ptr_eq(&current, provider))
        .unwrap_or(false)
}

/// `AppHandle` flavour of [`is_current`] used by `poll_loop`.
async fn still_registered(app: &AppHandle, provider: &Arc<dyn Provider>, id: &str) -> bool {
    let state = app.state::<AppState>();
    let registry = state.registry.read().await;
    is_current(&registry, provider, id)
}

/// Per-provider polling loop.
///
/// - `tick`: fires on the effective interval (Settings override > per-provider
///   default); skipped while paused or while the provider is disabled.
/// - `settings_rx.changed()`: wakes immediately on save_settings, rebuilds the
///   interval if the period changed, and does one immediate fetch if enabled.
async fn poll_loop(app: AppHandle, provider: Arc<dyn Provider>, id: String) {
    let initial_period = {
        let s = app.state::<AppState>().settings.get().await;
        effective_interval(provider.kind(), s.poll_interval_seconds)
    };
    let mut period = initial_period;
    // First tick ~1s after launch so users see data immediately.
    let start = Instant::now() + Duration::from_secs(1);
    let mut tick = interval_at(start, period);
    // Delay (not Burst): after a long pause we must not instantly replay all
    // missed ticks.
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut settings_rx = app.state::<AppState>().settings_wake.subscribe();
    let pause_rx = app.state::<AppState>().pause_tx.subscribe();

    loop {
        tokio::select! {
            _ = tick.tick() => {
                if *pause_rx.borrow() {
                    continue;
                }
                if !still_registered(&app, &provider, &id).await {
                    break;
                }
                let (enabled, _) = app
                    .state::<AppState>()
                    .settings
                    .poll_view(&id)
                    .await;
                if !enabled {
                    continue;
                }
                if let Err(e) = poll_one(&app, &provider).await {
                    tracing::warn!(provider = %id, error = %e, "poll failed");
                }
            }
            changed = settings_rx.changed() => {
                // Sender lives in AppState for the whole app lifetime; if it
                // is gone the app is tearing down, so end this task.
                if changed.is_err() {
                    break;
                }
                if !still_registered(&app, &provider, &id).await {
                    break;
                }
                // The payload says whether this save can change when or whether
                // we poll. A display-only edit (currency, ring window, edge
                // snap) still re-reads enabled state and the interval below,
                // but must not cost a request.
                let poll_relevant = settings_rx.borrow_and_update().poll_relevant;
                let (enabled, global_seconds) = app
                    .state::<AppState>()
                    .settings
                    .poll_view(&id)
                    .await;
                let new_period = effective_interval(provider.kind(), global_seconds);
                if new_period != period {
                    period = new_period;
                    let mut rebuilt = interval_at(Instant::now() + period, period);
                    rebuilt.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    tick = rebuilt;
                }
                // A watch channel keeps only the newest value, so a
                // poll-relevant save followed within one select iteration by a
                // display-only one coalesces into "nothing changed". The only
                // case where that loses data is an account that has no
                // snapshot at all, so probe for it before skipping the fetch.
                let needs_first_fetch = !poll_relevant
                    && !app.state::<AppState>().state.read().await.contains_key(&id);
                // Enable/disable or interval edits refresh immediately.
                if (poll_relevant || needs_first_fetch) && enabled && !*pause_rx.borrow() {
                    if let Err(e) = poll_one(&app, &provider).await {
                        tracing::warn!(provider = %id, error = %e,
                            "poll after settings change failed");
                    }
                }
            }
        }
    }
}

/// Fetch one provider and run the unified update pipeline:
/// burn diff -> threshold notify -> emit UsageUpdate -> upsert ProviderState.
/// Shared by the periodic scheduler and `force_refresh`.
pub async fn poll_one(
    app: &AppHandle,
    provider: &Arc<dyn Provider>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = app.state::<AppState>();

    let id = provider.id().to_string();
    let poll_lock = poll_lock_for(&state, &id).await;
    let _poll_guard = poll_lock.lock().await;

    // In-memory credential cache first, then OS credential store, then the
    // "mock" fallback (matches the pre-refactor behavior).
    let creds = {
        let cache = state.credentials.read().await;
        cache.get(&provider.id()).cloned()
    };
    let creds = creds
        .or_else(|| state.settings.load_credentials(&provider.id()))
        .unwrap_or(Credentials::BearerKey {
            api_key: "mock".to_string(),
        });

    let result = provider.fetch_usage(&creds).await;
    let mut guard = state.state.write().await;
    match result {
        Ok(snapshot) => {
            // A successful poll re-arms error reporting: if this provider later
            // breaks the same way, that is new information rather than a repeat
            // of a failure the user has already seen and that we have since
            // recovered from.
            state
                .error_deduper
                .lock()
                .await
                .clear(&id);

            // Burn + active share one baseline with manual refresh.
            let (burn, active) = {
                let mut tracker = state.burn.lock().await;
                tracker.observe(&id, &snapshot)
            };

            // Threshold notification state machine. Lock order is fixed:
            // snapshot map -> burn (released) -> settings -> notify. Skips OS
            // delivery entirely when the "用量告急通知" setting is off.
            let settings = state.settings.get().await;
            let action = {
                let mut notify_guard = state.notify.lock().await;
                notify_guard.evaluate(
                    &snapshot,
                    burn.as_ref(),
                    settings.notify_warn_percent,
                    settings.notify_crit_percent,
                )
            };
            if settings.notify_enabled {
                if let NotifyAction::Fire { title, body, .. } = &action {
                    notify::deliver(app, title, body);
                }
            }

            let update = UsageUpdate {
                snapshot: snapshot.clone(),
                burn,
                active,
            };
            let _ = app.emit("usage-updated", &update);
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
            // Only the first failure inside the cool down reaches the frontend.
            // Without this a provider that stays offline emits one event per
            // poll forever, and the frontend re-renders an identical error card
            // on every tick. The provider state below is still updated every
            // time: this only gates the event, not the recorded last_error.
            let message = err.to_string();
            let should_emit = state.error_deduper.lock().await.should_emit(&id, &message);
            if should_emit {
                let _ = app.emit(
                    "provider-error",
                    &serde_json::json!({ "id": id, "error": &message }),
                );
            }
            // Keep the last good snapshot: stale data + error badge beats an
            // empty card during a transient API outage.
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
            cost_source: Default::default(),
            tokens: None,
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
        // Deterministic timestamps: two adjacent Utc::now() calls plus a
        // zero-tolerance assertion would make this test flaky under scheduler
        // stalls (elapsed truncating to 59s/61s). The function under test is
        // pure and receives time by parameter, so feed it fixed instants.
        let t0 = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let t1 = t0 + chrono::Duration::seconds(60);
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

#[cfg(test)]
mod interval_tests {
    use super::*;

    #[test]
    fn positive_global_override_wins() {
        assert_eq!(
            effective_interval("minimax", 30),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn zero_falls_back_to_provider_default() {
        assert_eq!(
            effective_interval("minimax", 0),
            Duration::from_secs(DEFAULT_INTERVAL_MINIMAX)
        );
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use crate::providers::mock::MockProvider;
    use crate::providers::ProviderRegistry;

    fn provider(id: &'static str) -> Arc<dyn Provider> {
        Arc::new(MockProvider::new(id, id, 1.0))
    }

    #[test]
    fn current_instance_is_live() {
        let p = provider("acct-a");
        let mut registry = ProviderRegistry::new();
        registry.insert(p.clone());
        assert!(is_current(&registry, &p, "acct-a"));
    }

    #[test]
    fn replaced_instance_is_stale() {
        let p = provider("acct-a");
        let mut registry = ProviderRegistry::new();
        registry.insert(p.clone());
        // Same id, different instance: what a registry rebuild produces.
        registry.insert(provider("acct-a"));
        assert!(!is_current(&registry, &p, "acct-a"));
    }

    #[test]
    fn unknown_id_is_stale() {
        let p = provider("acct-a");
        let registry = ProviderRegistry::new();
        assert!(!is_current(&registry, &p, "acct-a"));
    }
}

#[cfg(test)]
mod poll_lock_tests {
    use super::*;

    #[tokio::test]
    async fn same_id_shares_one_mutex_and_excludes_reentrancy() {
        let mut map = HashMap::new();
        let a = lock_entry(&mut map, "acct-a");
        let a_again = lock_entry(&mut map, "acct-a");
        let b = lock_entry(&mut map, "acct-b");

        // Same id, including across a registry rebuild: one and the same lock.
        assert!(Arc::ptr_eq(&a, &a_again));
        // Different ids never block each other.
        assert!(!Arc::ptr_eq(&a, &b));

        // Held lock rejects a second acquisition of the same lock.
        let guard = a.lock().await;
        assert!(a.try_lock().is_err());
        assert!(a_again.try_lock().is_err());
        // The other id's lock stays free.
        assert!(b.try_lock().is_ok());
        drop(guard);

        // Once released, the same lock can be acquired again.
        assert!(a.try_lock().is_ok());
    }
}
