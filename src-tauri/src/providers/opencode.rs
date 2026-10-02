//! OpenCode Zen "Go" subscription usage provider.
//!
//! Endpoint:
//!   `GET https://opencode.ai/zen/go/v1/usage` →
//!   ```json
//!   { "usage": {
//!       "rolling": { "status": "ok",           "percent": 12, "resetsAt": "…Z" },
//!       "weekly":  { "status": "ok",           "percent": 34, "resetsAt": "…Z" },
//!       "monthly": { "status": "rate-limited", "percent": 100,"resetsAt": "…Z" } } }
//!   ```
//!
//! Three quota windows on a 100-point scale, computed server-side from
//! micro-cents of spend. `percent` is *consumed* (clamped 0..=100 by the
//! server), and `status` flips to `rate-limited` at the cap.
//!
//! **No absolute dollar figures and no per-model split are exposed** — the
//! limits live in a server-side deployment resource we cannot read, so the
//! windows stay on the `Percent` unit (same presentation as MiniMax Token
//! Plan) rather than inventing a dollar quota.
//!
//! Auth: `Authorization: Bearer <api key>` (the same key used for chat; created
//! at <https://opencode.ai/auth>). A key **without** a Go subscription is a
//! normal case, not a bug: the server answers `403` with
//! `{"type":"error","error":{"type":"EntitlementError",…}}`, surfaced here as a
//! clear "needs a Go subscription" message rather than a generic error.
//!
//! References:
//!   - <https://opencode.ai/docs/go/> (plan + quota model)
//!   - <https://github.com/anomalyco/opencode/blob/dev/packages/console/app/src/routes/zen/go/v1/usage.ts>
//!   - feature request that added it: <https://github.com/anomalyco/opencode/issues/16017>

use crate::providers::{
    AuthKind, CostSource, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

/// How many days of synthesized heatmap history to keep. The endpoint reports
/// only three rolling windows, never a per-day series, so the calendar
/// heatmap is built from the deltas we observe across polls (same approach as
/// MiniMax): each poll attributes its own consumption to the current day.
const KEEP_DAYS: usize = 200;

pub struct OpenCodeProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl OpenCodeProvider {
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

#[derive(Debug, Clone, Deserialize)]
struct Window {
    /// `"ok"` or `"rate-limited"`. Unknown values are tolerated.
    #[serde(default)]
    status: Option<String>,
    /// Consumed percent 0..=100. Missing/malformed → 0 rather than dropping
    /// the window, so a partial response still renders something truthful.
    #[serde(default)]
    percent: Option<f64>,
    /// Server-computed UTC reset instant (ISO 8601).
    #[serde(default)]
    #[serde(rename = "resetsAt")]
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct UsageBody {
    #[serde(default)]
    rolling: Option<Window>,
    #[serde(default)]
    weekly: Option<Window>,
    #[serde(default)]
    monthly: Option<Window>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    usage: Option<UsageBody>,
}

/// Error envelope shape used by the console API for both 401 and 403.
#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    error: Option<ApiErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Extract the bearer token; returns the provider-flavored auth error when the
/// account is configured without one.
fn bearer(creds: &Credentials) -> Result<String, ProviderError> {
    match creds {
        Credentials::BearerKey { api_key } if !api_key.trim().is_empty() => {
            Ok(api_key.trim().to_string())
        }
        Credentials::BearerKey { .. } => Err(ProviderError::Auth {
            message: "未配置 API Key".to_string(),
        }),
        Credentials::AccessKeySecret { .. } => Err(ProviderError::Auth {
            message: "OpenCode 需要 API Key 认证".to_string(),
        }),
        Credentials::LocalToken { .. } => Err(ProviderError::Auth {
            message: "OpenCode 需要 API Key 认证".to_string(),
        }),
    }
}

fn parse_reset(value: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    // Guard against a server sending an epoch-like or absurdly distant value.
    let now = Utc::now();
    match value {
        Some(dt) if dt > now - Duration::days(1) && dt < now + Duration::days(400) => Some(dt),
        _ => None,
    }
}

/// Build a percentage-window `WindowUsage`, or `None` when the server omitted it.
fn window(w: Option<&Window>) -> Option<WindowUsage> {
    let w = w?;
    let used = w.percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let rate_limited = w.status.as_deref() == Some("rate-limited");
    Some(WindowUsage {
        // At the cap the server reports 100 either way; `rate-limited` with a
        // lower percent would be inconsistent, so trust the flag and pin 100.
        used: if rate_limited { 100.0 } else { used },
        quota: 100.0,
        unit: UsageUnit::Percent,
        reset_at: parse_reset(w.resets_at),
        over_quota: rate_limited || used >= 100.0,
        cost_source: CostSource::ProviderReported,
        tokens: None,
    })
}

/// 跨轮询的窗口增量 → 今天的日用量。
///
/// 同形如 minimax.rs：首次观察只建基线；同一窗口取 consumed 增量（回退按 0）；
/// 窗口滚动（resetsAt 变化）则把当前 consumed 全部计入今天。
/// 百分比制额度下日值可以 >100%，含义是"当日消耗的窗口额度当量"。
fn window_delta(prev: Option<(i64, f64)>, cur: (i64, f64)) -> f64 {
    match prev {
        None => 0.0,
        Some((pe, pc)) if pe == cur.0 => (cur.1 - pc).max(0.0),
        Some(_) => cur.1.max(0.0),
    }
}

const KV_ROLLING: &str = "oc_go_rolling";
const KV_WEEKLY: &str = "oc_go_weekly";
const KV_MONTHLY: &str = "oc_go_monthly";

/// 解析 KV 里存的 `"{resetsAt_epoch_secs}:{consumed_pct}"` 基线。
fn parse_baseline(raw: &str) -> Option<(i64, f64)> {
    let (end, pct) = raw.split_once(':')?;
    Some((end.trim().parse().ok()?, pct.trim().parse().ok()?))
}

#[async_trait]
impl Provider for OpenCodeProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "opencode"
    }

    fn display_name(&self) -> String {
        if self.label.trim().is_empty() {
            "OpenCode Go".to_string()
        } else {
            self.label.clone()
        }
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let token = bearer(creds)?;
        let resp = self
            .http
            .get(USAGE_URL)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Network {
                message: e.to_string(),
            })?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ProviderError::Network {
                message: e.to_string(),
            })?;

        if !status.is_success() {
            let parsed: Option<ApiError> = serde_json::from_str(&body).ok();
            let kind = parsed
                .as_ref()
                .and_then(|p| p.error.as_ref())
                .and_then(|e| e.kind.as_deref())
                .unwrap_or("");
            let message = parsed
                .as_ref()
                .and_then(|p| p.error.as_ref())
                .and_then(|e| e.message.clone())
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(match status.as_u16() {
                401 | 403 => ProviderError::Auth {
                    message: if kind == "EntitlementError" {
                        format!("该 Key 未订阅 OpenCode Go（{message}）")
                    } else {
                        format!("认证失败（{message}）")
                    },
                },
                429 => ProviderError::Network {
                    message: "请求过于频繁，请稍后重试".to_string(),
                },
                _ => ProviderError::Network {
                    message: format!("HTTP {}: {message}", status.as_u16()),
                },
            });
        }

        let parsed: UsageResponse = serde_json::from_str(&body).map_err(|e| ProviderError::Network {
            message: format!("unexpected usage payload: {e}"),
        })?;
        let usage = parsed.usage.ok_or_else(|| ProviderError::Network {
            message: "响应缺少 usage 字段".to_string(),
        })?;

        let five_hour = window(usage.rolling.as_ref());
        let weekly = window(usage.weekly.as_ref());
        let monthly = window(usage.monthly.as_ref());

        // ── synthesized heatmap: today's consumption = the three window deltas
        let today = Local::now().date_naive();
        let mut by_day: BTreeMap<NaiveDate, f64> = BTreeMap::new();
        for (key, w) in [(KV_ROLLING, &five_hour), (KV_WEEKLY, &weekly), (KV_MONTHLY, &monthly)] {
            let Some(w) = w else { continue };
            let end = w.reset_at.map(|d| d.timestamp()).unwrap_or(0);
            let prev = self
                .storage
                .kv_get(self.self_id(), key)
                .ok()
                .flatten()
                .and_then(|raw| parse_baseline(&raw));
            let delta = window_delta(prev, (end, w.used));
            if delta > 0.0 {
                *by_day.entry(today).or_default() += delta;
            }
            let _ = self
                .storage
                .kv_set(self.self_id(), key, &format!("{}:{}", end, w.used));
        }

        // Persist each observed day (max-merge) and serve a long window.
        for (date, value) in &by_day {
            let _ = self
                .storage
                .record_daily_on(self.self_id(), *date, *value, UsageUnit::Percent);
        }
        let heatmap: Vec<HeatmapCell> = self
            .storage
            .load_heatmap(self.self_id(), KEEP_DAYS as u32)
            .unwrap_or_default();

        Ok(UsageSnapshot {
            provider_id: self.instance_id.clone(),
            provider_display_name: self.display_name(),
            plan_tier: Some("Go".to_string()),
            timestamp: Utc::now(),
            windows: UsageWindows {
                five_hour,
                daily: None,
                weekly,
                monthly,
                balance: None,
            },
            heatmap: if heatmap.is_empty() {
                None
            } else {
                Some(heatmap)
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_payload() {
        // resetsAt 接受 now-1d..now+400d 之间的值，固定日期会随时间腐坏——
        // 动态生成"未来 1 小时 / 6 天 / 21 天"三个时间戳。
        let iso = |secs: i64| {
            (chrono::Utc::now() + chrono::Duration::seconds(secs))
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        };
        let raw = format!(r#"{{"usage":{{
            "rolling":{{"status":"ok","percent":12,"resetsAt":"{}"}},
            "weekly":{{"status":"ok","percent":34,"resetsAt":"{}"}},
            "monthly":{{"status":"rate-limited","percent":100,"resetsAt":"{}"}}}}}}"#,
            iso(3600), iso(6 * 86400), iso(21 * 86400));
        let parsed: UsageResponse = serde_json::from_str(&raw).unwrap();
        let u = parsed.usage.unwrap();
        let r = window(u.rolling.as_ref()).unwrap();
        assert_eq!(r.used, 12.0);
        assert_eq!(r.quota, 100.0);
        assert_eq!(r.unit, UsageUnit::Percent);
        assert!(!r.over_quota);
        assert!(r.reset_at.is_some());

        let m = window(u.monthly.as_ref()).unwrap();
        assert_eq!(m.used, 100.0);
        assert!(m.over_quota);

        let w = window(u.weekly.as_ref()).unwrap();
        assert_eq!(w.used, 34.0);
    }

    #[test]
    fn rate_limited_status_pins_the_bar_to_full() {
        // Server said 37% but flagged rate-limited: trust the flag.
        let raw = r#"{"usage":{"monthly":{"status":"rate-limited","percent":37,"resetsAt":"2026-10-01T00:00:00.000Z"}}}"#;
        let parsed: UsageResponse = serde_json::from_str(&raw).unwrap();
        let m = window(parsed.usage.unwrap().monthly.as_ref()).unwrap();
        assert_eq!(m.used, 100.0);
        assert!(m.over_quota);
    }

    #[test]
    fn missing_windows_degrade_to_none_not_panic() {
        let raw = r#"{"usage":{"rolling":{"status":"ok","percent":5}}}"#;
        let parsed: UsageResponse = serde_json::from_str(&raw).unwrap();
        let u = parsed.usage.unwrap();
        assert!(window(u.rolling.as_ref()).is_some());
        assert!(window(u.weekly.as_ref()).is_none());
        assert!(window(u.monthly.as_ref()).is_none());
    }

    #[test]
    fn absurd_reset_timestamps_are_rejected() {
        // Epoch 0 / far-future resets are "unreported", not real windows.
        let raw = r#"{"usage":{"weekly":{"status":"ok","percent":1,"resetsAt":"1970-01-01T00:00:00.000Z"}}}"#;
        let parsed: UsageResponse = serde_json::from_str(&raw).unwrap();
        let w = window(parsed.usage.unwrap().weekly.as_ref()).unwrap();
        assert!(w.reset_at.is_none());
    }

    #[test]
    fn percent_is_clamped_into_range() {
        let raw = r#"{"usage":{"daily":{"status":"ok","percent":9999}}}"#;
        // `daily` is not part of UsageBody — serde ignores unknown fields.
        let parsed: UsageResponse = serde_json::from_str(&raw).unwrap();
        assert!(parsed.usage.unwrap().rolling.is_none());

        let raw2 = r#"{"usage":{"rolling":{"status":"ok","percent":9999}}}"#;
        let parsed2: UsageResponse = serde_json::from_str(raw2).unwrap();
        assert_eq!(
            window(parsed2.usage.unwrap().rolling.as_ref()).unwrap().used,
            100.0
        );
    }

    #[test]
    fn window_delta_seeds_then_diffs_then_resets() {
        // First sight only establishes a baseline.
        assert_eq!(window_delta(None, (100, 30.0)), 0.0);
        // Same window → the increment.
        assert_eq!(window_delta(Some((100, 30.0)), (100, 42.0)), 12.0);
        // A backwards jump inside one window is treated as no consumption.
        assert_eq!(window_delta(Some((100, 42.0)), (100, 40.0)), 0.0);
        // Window rolled → everything consumed so far lands on today.
        assert_eq!(window_delta(Some((100, 42.0)), (200, 7.0)), 7.0);
    }

    #[test]
    fn bearer_requires_a_non_empty_key() {
        assert!(matches!(
            bearer(&Credentials::BearerKey { api_key: "  ".into() }),
            Err(ProviderError::Auth { .. })
        ));
        assert!(matches!(
            bearer(&Credentials::LocalToken { token: "x".into() }),
            Err(ProviderError::Auth { .. })
        ));
        let ok = bearer(&Credentials::BearerKey { api_key: " sk-abc ".into() }).unwrap();
        assert_eq!(ok, "sk-abc");
    }

    #[test]
    fn entitlement_error_is_recognised_from_the_envelope() {
        let raw = r#"{"type":"error","error":{"type":"EntitlementError","message":"OpenCode Go subscription required."}}"#;
        let parsed: ApiError = serde_json::from_str(raw).unwrap();
        let kind = parsed.error.unwrap().kind.unwrap();
        assert_eq!(kind, "EntitlementError");
    }
}
