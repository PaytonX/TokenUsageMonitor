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
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

/// How many days of synthesized heatmap history to keep. The endpoint reports
/// only rolling windows, never a per-day series, so the calendar heatmap is
/// built from the deltas we observe across polls: each poll's consumption is
/// **accumulated** onto the current day (the 30-second slice it represents).
/// Only the monthly window feeds the series — it is the one additive window
/// (see `fetch_usage`).
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

/// 跨轮询的窗口增量（consumed 百分点）。
///
/// 首次观察只建基线（返回 0，不猜历史）；同一窗口取 consumed 增量，回退（服务端
/// 重算/时钟偏差）按 0 截断；窗口滚动（resetsAt 变化）则把当前 consumed 全部计入
/// 今天——那是新周期开张以来烧掉的部分，此前没有被任何一天记过。
///
/// 语义是"这一个窗口里、这一段时间内的消耗"，**不能跨窗口相加**：调用方只把它
/// 用在可加的那一个窗口上（见 `fetch_usage` 的月度差分说明）。
fn window_delta(prev: Option<(i64, f64)>, cur: (i64, f64)) -> f64 {
    match prev {
        None => 0.0,
        Some((pe, pc)) if pe == cur.0 => (cur.1 - pc).max(0.0),
        Some(_) => cur.1.max(0.0),
    }
}

/// 月度窗口的上次观察值。只有它被持久化：按日序列由月度差分累加而成，滚动/周
/// 窗口不参与记账，留它们的基线只会让人以为它们在记。旧版本写下的
/// `oc_go_rolling` / `oc_go_weekly` 键留在库里无害（不再被读取）。
const KV_MONTHLY: &str = "oc_go_monthly";

/// 今天的记账增量：只取**月度窗口**的差分。
///
/// 单独抽出来是为了把"哪些窗口可以相加"这条判断钉在测试里。三个窗口各有自己
/// 的额度分母，同一笔消费会在每个窗口里各涨一次；只有月度在计费周期内单调，
/// 逐日差分之和才等于"本月已用%"。月度缺失时不记账（也不拿别的窗口顶替）。
fn daily_increment(
    monthly_prev: Option<(i64, f64)>,
    monthly_cur: Option<(i64, f64)>,
) -> f64 {
    match monthly_cur {
        Some(cur) => window_delta(monthly_prev, cur),
        None => 0.0,
    }
}

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

        // ── synthesized heatmap: today's consumption, attributed by the monthly delta
        //
        // 三个窗口的分母是三份**不同的额度**（近 5 小时 / 周 / 月各有一条额度线），
        // 把它们的百分点增量相加是跨分母相加：同一笔消费会在三个窗口里各记一次，
        // 日值轻易超过 100%，既不可解释也不可比。这里只用月度窗口——
        // 它在计费周期内单调不减，逐日差分之和恰好等于"本月已用百分比"，
        // 是唯一自洽的可加口径；周窗口只用于展示。
        //
        // 写入必须是**累加**而不是取峰值：每次轮询算出来的是这 30 秒的增量，
        // 峰值合并会把当天压成"最大的一次增量"，也就是十几分钟里最热的 30 秒。
        let monthly_obs = monthly
            .as_ref()
            .map(|w| (w.reset_at.map(|d| d.timestamp()).unwrap_or(0), w.used));
        let mut today_delta = 0.0;
        if let Some(cur) = monthly_obs {
            let prev = self
                .storage
                .kv_get(self.self_id(), KV_MONTHLY)
                .ok()
                .flatten()
                .and_then(|raw| parse_baseline(&raw));
            today_delta = daily_increment(prev, Some(cur));
            let _ = self
                .storage
                .kv_set(self.self_id(), KV_MONTHLY, &format!("{}:{}", cur.0, cur.1));
        }
        if today_delta > 0.0 {
            let _ = self
                .storage
                .accumulate_daily(self.self_id(), today_delta, UsageUnit::Percent);
        }
        let heatmap: Vec<HeatmapCell> = self
            .storage
            .load_heatmap(self.self_id(), KEEP_DAYS as u32)
            .unwrap_or_default();

        // ── 统一账本双写：百分比日值也进 `usage_daily`
        //
        // 差分类 Provider 过去只写 `daily_snapshots`，而趋势/统计视图只读
        // `usage_daily`，于是 OpenCode 在统计页**根本没有入口**。这里按
        // volcengine / openai / xai 的双写模式把同一串日值也写进账本
        // （kind=provider、unit=percent）：token 日历只吸收 tokens 行，
        // percent 行自成一条额度轨道，两者永不相加。整段 replace 幂等，
        // 每次轮询重放整段历史。
        let ledger: Vec<crate::storage::UsageDailyRow> = heatmap
            .iter()
            .map(|cell| crate::storage::UsageDailyRow {
                source: self.instance_id.clone(),
                kind: "provider".to_string(),
                date: cell.date.to_string(),
                model: String::new(),
                provider: String::new(),
                input: 0.0,
                cache_read: 0.0,
                output: 0.0,
                total: cell.value,
                unit: cell.unit,
                cost: None,
                currency: None,
                cost_estimated: false,
            })
            .collect();
        if !ledger.is_empty() {
            let _ = self.storage.replace_usage_daily(&ledger);
        }

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
    fn daily_increment_counts_the_monthly_window_only() {
        // 同一笔消费让 rolling/weekly/monthly 各涨 1 个百分点。旧实现把三个增量
        // 相加得到 3——那是跨分母相加（5h/周/月是三份不同的额度），会把一天记成
        // 三倍。可加的只有月度：日值必须是 1。
        let prev = Some((1_000, 12.0));
        assert_eq!(daily_increment(prev, Some((1_000, 13.0))), 1.0);
        // 月度窗口滚动 → 当前 consumed 全部计入今天（新周期开张以来烧掉的）。
        assert_eq!(daily_increment(prev, Some((2_000, 5.0))), 5.0);
        // 首次观察只建基线，不猜历史。
        assert_eq!(daily_increment(None, Some((1_000, 13.0))), 0.0);
        // 月度缺失 → 不记账；绝不拿周/滚动窗口顶替。
        assert_eq!(daily_increment(prev, None), 0.0);
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
