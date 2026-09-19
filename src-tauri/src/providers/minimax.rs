//! MiniMax Token Plan provider.
//!
//! Endpoint:  `GET https://www.minimaxi.com/v1/token_plan/remains`
//! Auth:      `Authorization: Bearer <Token Plan 订阅 Key>`
//!
//! Wire shape (captured against the live endpoint; MiniMax does not publish a
//! formal schema, every field here is one observed on the wire):
//!
//! ```json
//! {
//!   "model_remains": [
//!     {
//!       "model_name": "general",
//!       "start_time": 1785164400000,           // epoch ms
//!       "end_time": 1785182400000,             // epoch ms
//!       "current_interval_remaining_percent": 99, // 0..=100, REMAINING not used
//!       "weekly_start_time": 1785110400000,
//!       "weekly_end_time": 1785715200000,
//!       "current_weekly_remaining_percent": 99
//!     },
//!     { "model_name": "video", ... }
//!   ],
//!   "base_resp": {
//!     "status_code": 0,    // 0 = success, 1004 = no key, 2049 = invalid key
//!     "status_msg": "success"
//!   }
//! }
//! ```
//!
//! Four properties of this API drive the code below:
//! 1. **HTTP 200 always.** Auth failures come back `200` with the real status
//!    in `base_resp.status_code`. The HTTP status must never be read as success.
//! 2. **The percentages are what REMAINS**, not what was consumed. They are
//!    inverted here so the rest of the app keeps its consumed-% convention.
//! 3. **The interval length is not fixed** — `general` rolls every 5h but
//!    `video` rolls every 24h, so the window duration comes from each row's
//!    own `start_time`/`end_time` instead of a constant.
//! 4. **All timestamps are epoch milliseconds**, not seconds.

use crate::providers::{
    AuthKind, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const ENDPOINT: &str = "https://www.minimaxi.com/v1/token_plan/remains";

/// Bucket name of the text/coding pool — the one that drives the bars.
const BUCKET_GENERAL: &str = "general";
/// Bucket name of the video-generation pool, rendered as a secondary window.
const BUCKET_VIDEO: &str = "video";

/// MiniMax's standard envelope. Present on every response, including the ones
/// that carry no data because authentication failed.
#[derive(Debug, Clone, Deserialize)]
struct BaseResp {
    status_code: i64,
    #[serde(default)]
    #[allow(dead_code)]
    status_msg: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RemainsEnvelope {
    /// Absent (not merely empty) on failure responses.
    #[serde(default)]
    model_remains: Vec<ModelRemains>,
    base_resp: BaseResp,
}

impl RemainsEnvelope {
    /// Reject the in-band failure shape before any field is read as a quota.
    /// `status_code == 0` is the documented success value; everything else —
    /// including the auth failures that arrive as HTTP 200 — is an error.
    fn check_ok(&self) -> Result<(), ProviderError> {
        if self.base_resp.status_code == 0 {
            return Ok(());
        }
        // Map known auth-failure codes to Auth so the UI reports an auth
        // problem rather than a generic schema error.
        if matches!(self.base_resp.status_code, 1004 | 2049) {
            return Err(ProviderError::Auth {
                message: format!("MiniMax rejected the API key (status {})",
                    self.base_resp.status_code),
            });
        }
        Err(ProviderError::Parse {
            message: format!("MiniMax API failure (status_code {})",
                self.base_resp.status_code),
        })
    }
}

/// One model bucket's quota.
#[derive(Debug, Clone, Deserialize)]
struct ModelRemains {
    model_name: String,
    /// Rolling interval window, epoch **milliseconds**.
    start_time: i64,
    end_time: i64,
    /// Percentage of the interval quota still available (0..=100).
    current_interval_remaining_percent: i64,
    /// Weekly window, epoch **milliseconds**.
    weekly_start_time: i64,
    weekly_end_time: i64,
    /// Percentage of the weekly quota still available (0..=100).
    current_weekly_remaining_percent: i64,
}

pub struct MiniMaxProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl MiniMaxProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }

    fn self_id(&self) -> &str {
        &self.instance_id
    }
}

/// Consumed percent from MiniMax's remaining percent, clamped to 0..=100.
fn consumed_pct(remaining: i64) -> f64 {
    (100 - remaining.clamp(0, 100)) as f64
}

/// Epoch milliseconds to a UTC instant. Non-positive values mean "unreported"
/// rather than 1970, so they become `None`.
fn at_millis(ms: i64) -> Option<DateTime<Utc>> {
    if ms <= 0 {
        return None;
    }
    DateTime::from_timestamp_millis(ms)
}

/// Build a `WindowUsage` from a single model row's interval window.
fn interval_window(row: &ModelRemains) -> WindowUsage {
    let used = consumed_pct(row.current_interval_remaining_percent);
    WindowUsage {
        used,
        quota: 100.0,
        unit: UsageUnit::Percent,
        reset_at: at_millis(row.end_time),
        over_quota: used > 100.0,
    }
}

/// Build a `WindowUsage` from a single model row's weekly window.
fn weekly_window(row: &ModelRemains) -> WindowUsage {
    let used = consumed_pct(row.current_weekly_remaining_percent);
    WindowUsage {
        used,
        quota: 100.0,
        unit: UsageUnit::Percent,
        reset_at: at_millis(row.weekly_end_time),
        over_quota: used > 100.0,
    }
}

#[async_trait]
impl Provider for MiniMaxProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "minimax"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let api_key = match creds {
            Credentials::BearerKey { api_key } => api_key.clone(),
            _ => return Err(ProviderError::Auth {
                message: "MiniMax requires BearerKey credentials".into(),
            }),
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let resp = self
            .http
            .get(ENDPOINT)
            .bearer_auth(&api_key)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        // MiniMax always returns HTTP 200, even on auth failure. Read the
        // body and inspect base_resp.status_code to know if it really worked.
        let body: RemainsEnvelope = resp.json().await?;
        body.check_ok()?;

        // Pick the `general` bucket; fall back to the first non-`video` row if
        // the plan ever names its text bucket something else.
        let rows = &body.model_remains;
        if rows.is_empty() {
            return Err(ProviderError::Parse {
                message: "MiniMax response carried no model buckets".into(),
            });
        }
        let general = rows
            .iter()
            .find(|r| r.model_name == BUCKET_GENERAL)
            .or_else(|| rows.iter().find(|r| r.model_name != BUCKET_VIDEO))
            .ok_or_else(|| ProviderError::Parse {
                message: "MiniMax response carried no usable (non-video) bucket".into(),
            })?;

        let now = Utc::now();

        let windows = UsageWindows {
            five_hour: Some(interval_window(general)),
            weekly: Some(weekly_window(general)),
            monthly: None, // MiniMax subscription tiers don't expose monthly quota.
            balance: None,
            daily: None,
        };

        // Persist a heatmap cell from the 5h window's consumed percent. MiniMax
        // exposes only the *current* window's remaining %, so a window's usage
        // is attributed to the calendar day its end_time falls on — never to the
        // day the poll happened (which caused the previous day to "borrow" a
        // window that actually ended but was still being re-recorded).
        //
        // Rule: remember the last observed window in kv; write a cell only when
        // a *new* window appears (i.e. the previous one rolled), attributing the
        // stored value to the stored end_date. Same-window polls just update the
        // tracked value (只更新不写) so we never mis-date or double-count.
        let end_ms = general.end_time;
        let end_date = DateTime::<Utc>::from_timestamp_millis(end_ms)
            .map(|d| d.with_timezone(&Local).date_naive())
            .unwrap_or_else(|| Local::now().date_naive());
        let consumed = consumed_pct(general.current_interval_remaining_percent);

        let prev_end = self
            .storage
            .kv_get(self.self_id(), "mm_win_end_ms")
            .ok()
            .flatten();
        let prev_consumed = self
            .storage
            .kv_get(self.self_id(), "mm_win_consumed_pct")
            .ok()
            .flatten();
        let prev_date = self
            .storage
            .kv_get(self.self_id(), "mm_win_end_date")
            .ok()
            .flatten();

        let rolled = matches!(&prev_end, Some(p) if p.parse::<i64>().ok() != Some(end_ms));
        if rolled {
            if let (Some(c_s), Some(d_s)) = (&prev_consumed, &prev_date) {
                if let (Ok(prev_c), Ok(prev_d)) = (
                    c_s.parse::<f64>(),
                    NaiveDate::parse_from_str(d_s, "%Y-%m-%d"),
                ) {
                    let _ = self
                        .storage
                        .record_daily_on(self.self_id(), prev_d, prev_c, UsageUnit::Percent);
                }
            }
        }
        // Track the current window for the next poll's roll detection.
        let _ = self.storage.kv_set(self.self_id(), "mm_win_end_ms", &end_ms.to_string());
        let _ = self.storage.kv_set(self.self_id(), "mm_win_consumed_pct", &consumed.to_string());
        let _ = self.storage.kv_set(self.self_id(), "mm_win_end_date", &end_date.to_string());

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 90)
            .ok()
            .filter(|v| !v.is_empty());

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: now,
            windows,
            heatmap,
        })
    }
}
