//! Threshold notification state machine.
//!
//! In-memory dedupe keyed by (provider_id, window_key). Each window can
//! deliver at most one warn and one crit notification; once usage falls back
//! below the warn threshold (window reset) the key is cleared and re-armed.

use crate::providers::{BurnInfo, UsageSnapshot};
use std::collections::HashMap;
use std::time::{Duration, Instant};
/// Rate-limits repeated `provider-error` events for the same failing pair.
///
/// Threshold notifications already dedupe per window ([`NotifyState`]), but the
/// error path did not: a provider that stays rate-limited or offline re-emitted
/// on every single poll, and with a 300s period that is one frontend event per
/// five minutes per account, forever, even when the message is identical.
///
/// The frontend only needs to know that the card is broken and why. Repeating
/// the same reason adds no information, so we suppress repeats inside a cool
/// down window and let a *changed* message through immediately - a genuine
/// state change (auth error -> rate limit) should never be delayed.
#[derive(Debug)]
pub struct ErrorDeduper {
    /// (provider_id, error text) -> when we last emitted this exact pair.
    last_emitted: HashMap<(String, String), Instant>,
    /// Re-emit the same message at most once per this duration.
    pub cooldown: Duration,
}

/// Five minutes: long enough to swallow a poll storm, short enough that a user
/// watching the card still sees the error keep "breathing" without a restart.
pub const ERROR_COOLDOWN: Duration = Duration::from_secs(300);

impl Default for ErrorDeduper {
    fn default() -> Self {
        Self {
            last_emitted: HashMap::new(),
            cooldown: ERROR_COOLDOWN,
        }
    }
}

impl ErrorDeduper {
    /// Record a failure and report whether it should reach the frontend.
    /// First sighting always fires; an identical repeat inside the cool down
    /// is suppressed; a different message for the same provider fires at once.
    pub fn should_emit(&mut self, provider_id: &str, error: &str) -> bool {
        let key = (provider_id.to_string(), error.to_string());
        let now = Instant::now();
        match self.last_emitted.get(&key) {
            Some(&last) if now.duration_since(last) < self.cooldown => {
                tracing::trace!(
                    provider = provider_id,
                    error,
                    "suppressed duplicate provider-error"
                );
                false
            }
            _ => {
                self.last_emitted.insert(key, now);
                self.prune(now);
                true
            }
        }
    }

    /// Drop bookkeeping for a provider that is polling again, so a later
    /// regression of the same error is reported as new rather than as a
    /// continuation of a failure from before the recovery.
    pub fn clear(&mut self, provider_id: &str) {
        self.last_emitted.retain(|(id, _), _| id != provider_id);
    }

    /// Forget entries that have aged past the cool down. Without this the map
    /// would retain one dead string per (provider, message) pair for the whole
    /// app lifetime. Only called on the emitting path, and only once the map
    /// is larger than a handful of entries, so it stays O(n) with a tiny n.
    fn prune(&mut self, now: Instant) {
        if self.last_emitted.len() <= 32 {
            return;
        }
        // Keep the entries that are still within twice the cool down; drop the
        // rest. A just-emitted entry always survives, because its age is 0.
        let horizon = self.cooldown * 2;
        self.last_emitted
            .retain(|_, seen| now.duration_since(*seen) < horizon);
    }
}

pub type SharedErrorDeduper = std::sync::Arc<tokio::sync::Mutex<ErrorDeduper>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Warn,
    Crit,
}

#[derive(Default)]
pub struct NotifyState {
    sent: HashMap<(String, String), Level>,
}

/// Decision produced by the state machine for one snapshot evaluation.
#[derive(Debug)]
pub enum NotifyAction {
    None,
    Fire {
        provider_id: String,
        level: Level,
        title: String,
        body: String,
    },
}

fn eta_text(burn: Option<&BurnInfo>) -> String {
    match burn.and_then(|b| b.eta_seconds) {
        Some(secs) => {
            let mins = secs / 60;
            if mins >= 60 {
                format!("约 {} 小时 {} 分钟后耗尽", mins / 60, mins % 60)
            } else if mins >= 1 {
                format!("约 {} 分钟后耗尽", mins)
            } else {
                "约 1 分钟内耗尽".to_string()
            }
        }
        None => String::new(),
    }
}

impl NotifyState {
    /// Evaluate one snapshot against warn/crit percent thresholds.
    /// Percent values are integers 0..=100 from settings.
    pub fn evaluate(
        &mut self,
        snap: &UsageSnapshot,
        burn: Option<&BurnInfo>,
        warn_percent: u8,
        crit_percent: u8,
    ) -> NotifyAction {
        let Some((key, used_pct)) = Self::critical_pct(snap) else {
            return NotifyAction::None;
        };
        let map_key = (snap.provider_id.clone(), key.to_string());
        let pct_int = (used_pct * 100.0).round() as u8;

        let level = if used_pct >= (crit_percent as f64 / 100.0) {
            Level::Crit
        } else if used_pct >= (warn_percent as f64 / 100.0) {
            Level::Warn
        } else {
            // Below warn line: reset window clears the dedupe key, re-arming.
            self.sent.remove(&map_key);
            return NotifyAction::None;
        };

        if self.sent.get(&map_key).copied().unwrap_or(Level::Warn) >= level
            && self.sent.contains_key(&map_key)
        {
            return NotifyAction::None;
        }
        self.sent.insert(map_key, level);

        let title = match level {
            Level::Warn => format!("{} 用量提醒（{}%）", snap.provider_display_name, pct_int),
            Level::Crit => format!("{} 用量告急（{}%）", snap.provider_display_name, pct_int),
        };
        let mut body = format!("{} 本窗口已用 {}%", snap.provider_display_name, pct_int);
        let eta = eta_text(burn);
        if !eta.is_empty() {
            body.push('，');
            body.push_str(&eta);
        }

        NotifyAction::Fire {
            provider_id: snap.provider_id.clone(),
            level,
            title,
            body,
        }
    }

    /// (stable window key, used-percent 0..1) of the most critical window
    /// that has a known quota.
    fn critical_pct(snap: &UsageSnapshot) -> Option<(&'static str, f64)> {
        [
            ("five_hour", snap.windows.five_hour.as_ref()),
            ("daily", snap.windows.daily.as_ref()),
            ("weekly", snap.windows.weekly.as_ref()),
            ("monthly", snap.windows.monthly.as_ref()),
        ]
        .into_iter()
        .filter_map(|(k, w)| w.map(|w| (k, w)))
        .filter(|(_, w)| w.quota > 0.0)
        .map(|(k, w)| (k, w.percent()))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }
}

/// Deliver a notification through tauri-plugin-notification. Best effort:
/// failures are logged and never break the polling pipeline. Desktop
/// NotificationBuilder exposes no click/action callback, so we intentionally
/// do not register a click handler (clicking is a no-op on desktop).
pub fn deliver(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "failed to show notification");
    }
}

pub type SharedNotifyState = std::sync::Arc<tokio::sync::Mutex<NotifyState>>;

#[cfg(test)]
mod notify_state_tests {
    use super::*;
    use crate::providers::{UsageUnit, UsageWindows, WindowUsage};

    fn window(pct: f64) -> WindowUsage {
        WindowUsage {
            used: pct * 100.0,
            quota: 100.0,
            unit: UsageUnit::Tokens,
            reset_at: None,
            over_quota: false,
            cost_source: Default::default(),
            tokens: None,
        }
    }

    fn snap(pct: f64) -> UsageSnapshot {
        UsageSnapshot {
            provider_id: "p".to_string(),
            provider_display_name: "方舟".to_string(),
            plan_tier: None,
            timestamp: chrono::Utc::now(),
            windows: UsageWindows {
                monthly: Some(window(pct)),
                ..Default::default()
            },
            heatmap: None,
        }
    }

    #[test]
    fn warn_then_crit_fire_once_each_then_go_silent() {
        let mut st = NotifyState::default();
        let a1 = st.evaluate(&snap(0.85), None, 80, 95);
        assert!(matches!(a1, NotifyAction::Fire { level: Level::Warn, .. }));
        let a2 = st.evaluate(&snap(0.90), None, 80, 95);
        assert!(matches!(a2, NotifyAction::None));
        let a3 = st.evaluate(&snap(0.96), None, 80, 95);
        assert!(matches!(a3, NotifyAction::Fire { level: Level::Crit, .. }));
        let a4 = st.evaluate(&snap(0.99), None, 80, 95);
        assert!(matches!(a4, NotifyAction::None));
    }

    #[test]
    fn dropping_below_warn_rearms() {
        let mut st = NotifyState::default();
        assert!(matches!(
            st.evaluate(&snap(0.85), None, 80, 95),
            NotifyAction::Fire { level: Level::Warn, .. }
        ));
        assert!(matches!(
            st.evaluate(&snap(0.40), None, 80, 95),
            NotifyAction::None
        ));
        assert!(matches!(
            st.evaluate(&snap(0.82), None, 80, 95),
            NotifyAction::Fire { level: Level::Warn, .. }
        ));
    }

    #[test]
    fn below_threshold_stays_silent_and_body_omits_eta_when_unknown() {
        let mut st = NotifyState::default();
        assert!(matches!(
            st.evaluate(&snap(0.50), None, 80, 95),
            NotifyAction::None
        ));
        match st.evaluate(&snap(0.85), None, 80, 95) {
            NotifyAction::Fire { body, .. } => assert!(!body.contains("耗尽")),
            other => panic!("expected fire, got other variant: {other:?}"),
        }
    }
}

#[cfg(test)]
mod error_deduper_tests {
    use super::*;

    fn deduper(cooldown_secs: u64) -> ErrorDeduper {
        ErrorDeduper {
            last_emitted: HashMap::new(),
            cooldown: Duration::from_secs(cooldown_secs),
        }
    }

    #[test]
    fn first_failure_of_a_provider_always_emits() {
        let mut d = deduper(300);
        assert!(d.should_emit("minimax-main", "network timeout"));
    }

    #[test]
    fn identical_repeat_inside_the_cooldown_is_suppressed() {
        let mut d = deduper(300);
        assert!(d.should_emit("minimax-main", "network timeout"));
        assert!(!d.should_emit("minimax-main", "network timeout"));
        assert!(!d.should_emit("minimax-main", "network timeout"));
    }

    #[test]
    fn a_changed_message_for_the_same_provider_emits_immediately() {
        let mut d = deduper(300);
        assert!(d.should_emit("minimax-main", "network timeout"));
        // Auth failure replacing a timeout is a new fact, not a repeat.
        assert!(d.should_emit("minimax-main", "401 unauthorized"));
        // ...and the original message stays suppressed, so alternating
        // between two errors cannot be used to bypass the cool down.
        assert!(!d.should_emit("minimax-main", "network timeout"));
    }

    #[test]
    fn one_failing_provider_does_not_silence_another() {
        let mut d = deduper(300);
        assert!(d.should_emit("minimax-main", "network timeout"));
        assert!(d.should_emit("deepseek-main", "network timeout"));
        assert!(!d.should_emit("deepseek-main", "network timeout"));
    }

    #[test]
    fn clear_rearms_the_error_after_a_successful_poll() {
        let mut d = deduper(300);
        assert!(d.should_emit("minimax-main", "network timeout"));
        assert!(!d.should_emit("minimax-main", "network timeout"));
        d.clear("minimax-main");
        // Recovery happened, so a later regression is news again.
        assert!(d.should_emit("minimax-main", "network timeout"));
    }

    #[test]
    fn clear_only_touches_the_named_provider() {
        let mut d = deduper(300);
        assert!(d.should_emit("a", "boom"));
        assert!(d.should_emit("b", "boom"));
        d.clear("a");
        assert!(d.should_emit("a", "boom"));
        assert!(!d.should_emit("b", "boom"));
    }

    #[test]
    fn a_zero_cooldown_lets_every_repeat_through() {
        // Guards the arithmetic in should_emit: with no cool down the elapsed
        // check must never be true, or the dedupe would silently die.
        let mut d = deduper(0);
        assert!(d.should_emit("p", "boom"));
        assert!(d.should_emit("p", "boom"));
        assert!(d.should_emit("p", "boom"));
    }

    #[test]
    fn prune_drops_entries_older_than_twice_the_cooldown() {
        let mut d = deduper(300);
        let stale = Instant::now() - Duration::from_secs(3600);
        for i in 0..40 {
            d.last_emitted
                .insert((format!("p{i}"), "boom".to_string()), stale);
        }
        assert!(d.should_emit("fresh", "boom"));
        // 40 back-dated entries are evicted; only the new one survives.
        assert_eq!(d.last_emitted.len(), 1);
        assert!(d
            .last_emitted
            .contains_key(&("fresh".to_string(), "boom".to_string())));
    }

    #[test]
    fn prune_is_skipped_while_the_map_is_small() {
        let mut d = deduper(300);
        // 31 entries, so the one inserted by should_emit brings the map to 32 -
        // exactly the floor, and prune bails out before scanning.
        for i in 0..31 {
            d.last_emitted
                .insert((format!("p{i}"), "boom".to_string()), Instant::now());
        }
        assert!(d.should_emit("fresh", "boom"));
        // Below the size floor nothing is evicted, so a normal-size map never
        // pays for a full scan on the hot path.
        assert_eq!(d.last_emitted.len(), 32);
    }
}
