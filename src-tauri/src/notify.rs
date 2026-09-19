//! Threshold notification state machine.
//!
//! In-memory dedupe keyed by (provider_id, window_key). Each window can
//! deliver at most one warn and one crit notification; once usage falls back
//! below the warn threshold (window reset) the key is cleared and re-armed.

use crate::providers::{BurnInfo, UsageSnapshot};
use std::collections::HashMap;

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
