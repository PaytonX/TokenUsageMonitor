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
use chrono::{DateTime, Utc};
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

/// 增量记账的核心：根据上次观察到的 `(窗口 end_ms, consumed%)` 与本次的
/// `(end_ms, consumed%)`，计算应计入「今天」的用量增量。
///
/// - 首次观察（prev 为 None）→ 0（只建基线，不把历史用量算进今天）；
/// - 同一窗口（end_ms 相同）→ consumed 的增量（抖动回退按 0 处理）；
/// - 窗口滚动（end_ms 变化）→ 新窗口当前已消耗的全部（旧窗口最后一次
///   轮询之后的尾巴丢失，轮询间隔为分钟级，损失可忽略）。
///
/// 一天多个 5h 窗口的增量会累加，日值可能 >100%：含义是「当日消耗的
/// 窗口额度当量」（如 140% = 1.4 个窗口额度），这是百分比制额度下
/// 最诚实的日用量口径。
fn window_delta(prev: Option<(i64, f64)>, cur: (i64, f64)) -> f64 {
    match prev {
        None => 0.0,
        Some((pe, pc)) if pe == cur.0 => (cur.1 - pc).max(0.0),
        Some(_) => cur.1.max(0.0),
    }
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

        // 热力图：MiniMax 没有历史用量查询接口，只有当前窗口的剩余 %。
        // 采用与 DeepSeek 余额差值一致的【增量记账】：每次轮询把
        // `window_delta` 计算出的增量累加到「今天」（accumulate_daily）。
        // 归属日 = 轮询当天（增量只可能发生在当下），彻底避免旧逻辑按
        // 窗口 end_time 归属导致的跨天错位；窗口滚动时新窗口已消耗的
        // 部分计入今天，多个窗口的消耗自然累加（见 window_delta 文档）。
        let end_ms = general.end_time;
        let consumed = consumed_pct(general.current_interval_remaining_percent);

        let prev_end = self
            .storage
            .kv_get(self.self_id(), "mm_win_end_ms")
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<i64>().ok());
        let prev_consumed = self
            .storage
            .kv_get(self.self_id(), "mm_win_consumed_pct")
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok());
        let prev = match (prev_end, prev_consumed) {
            (Some(e), Some(c)) => Some((e, c)),
            _ => None,
        };

        let delta = window_delta(prev, (end_ms, consumed));
        if delta > 0.0 {
            let _ = self
                .storage
                .accumulate_daily(self.self_id(), delta, UsageUnit::Percent);
        }
        // 更新基线，供下次轮询 diff。
        let _ = self
            .storage
            .kv_set(self.self_id(), "mm_win_end_ms", &end_ms.to_string());
        let _ = self
            .storage
            .kv_set(self.self_id(), "mm_win_consumed_pct", &consumed.to_string());

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 200)
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

#[cfg(test)]
mod minimax_delta_tests {
    use super::window_delta;

    #[test]
    fn first_observation_only_baselines() {
        // 首次轮询：只建基线，不把历史用量算进今天。
        assert_eq!(window_delta(None, (1000, 60.0)), 0.0);
    }

    #[test]
    fn same_window_accumulates_increase_only() {
        // 同一窗口内：增量 = 本次 - 上次。
        assert_eq!(window_delta(Some((1000, 30.0)), (1000, 45.0)), 15.0);
        // API 抖动导致回落：不计负数。
        assert_eq!(window_delta(Some((1000, 45.0)), (1000, 44.0)), 0.0);
        // 无变化：0。
        assert_eq!(window_delta(Some((1000, 45.0)), (1000, 45.0)), 0.0);
    }

    #[test]
    fn window_rollover_counts_new_window_usage() {
        // 窗口滚动：新窗口当前已消耗的部分计入今天（旧窗口尾巴丢失）。
        assert_eq!(window_delta(Some((1000, 80.0)), (2000, 12.0)), 12.0);
        // 滚动且新窗口还没消耗：0。
        assert_eq!(window_delta(Some((1000, 80.0)), (2000, 0.0)), 0.0);
    }

    #[test]
    fn multiple_windows_a_day_can_exceed_one_window() {
        // 一天多个窗口累加的口径验证：窗口 A 用了 60%，滚动后窗口 B 已用 80%。
        // B 滚动前（当天内）：60 + (0→80 增量) = 140% = 1.4 个窗口额度。
        let a = window_delta(None, (1000, 60.0)); // 基线，不写
        assert_eq!(a, 0.0);
        let b = window_delta(Some((1000, 60.0)), (2000, 80.0)); // B 滚动进来
        let c = window_delta(Some((2000, 80.0)), (2000, 80.0)); // B 内无变化
        assert_eq!(b + c, 80.0); // B 的当量全部计入
    }
}
