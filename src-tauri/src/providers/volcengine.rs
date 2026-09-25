//! Volcano Engine (Volcengine / 火山引擎) Ark AgentPlan provider.
//!
//! Endpoints (POST to base `https://ark.cn-beijing.volcengineapi.com/`):
//!   - `?Action=GetAFPUsage&Version=2024-01-01`
//!     Returns 5h / daily / weekly / monthly AFP windows (Quota/Used are
//!     JSON strings, e.g. "50.0" / "12.5").
//!   - `?Action=GetUsageDetails&Version=2024-01-01`
//!     Body `{"QueryInterval":"Day","Filter":{"StartTime":"YYYY-MM-DD",
//!     "EndTime":"YYYY-MM-DD"}}` returns per-day rows
//!     `Result.Details[] = {Time, ObjectName, Usage, Unit, BillingType}`.
//!
//! Auth: Volcengine V4 HMAC-SHA256 signature using Access Key + Secret Key.
//! See `crate::signing` for the signing algorithm.
//!
//! Reference:
//!   - <https://www.volcengine.com/docs/82379/2479847>  (GetAFPUsage)
//!   - <https://www.volcengine.com/docs/82379/2479849>  (GetUsageDetails)

use crate::providers::{
    AuthKind, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::signing;
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) const HOST: &str = "ark.cn-beijing.volcengineapi.com";
pub(crate) const BASE_URL: &str = "https://ark.cn-beijing.volcengineapi.com/";

/// Numbers in Volcengine responses are sometimes JSON numbers and sometimes
/// strings ("50.0"); accept both. `pub(crate)` so the pay-as-you-go variant
/// (`volcengine_api`) can reuse it to parse usage fields.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum FlexibleNum {
    Number(f64),
    Text(String),
}

impl FlexibleNum {
    pub(crate) fn value(&self) -> Option<f64> {
        match self {
            FlexibleNum::Number(n) => Some(*n),
            FlexibleNum::Text(s) => s.trim().parse().ok(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct ErrorEnvelope {
    #[serde(default, rename = "ResponseMetadata", alias = "response_metadata")]
    response_metadata: Option<MetadataRaw>,
}

#[derive(Debug, Deserialize, Default)]
struct MetadataRaw {
    #[serde(default, rename = "Error", alias = "error")]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize, Default)]
struct ApiError {
    #[serde(default, rename = "Code", alias = "code")]
    code: String,
    #[serde(default, rename = "Message", alias = "message")]
    message: String,
}

/// Map an HTTP status / response body carrying `ResponseMetadata.Error` to a
/// typed provider error with a human-readable message.
fn map_api_error(status: reqwest::StatusCode, body: &str) -> ProviderError {
    let envelope: ErrorEnvelope = serde_json::from_str(body).unwrap_or_default();
    let err = envelope.response_metadata.and_then(|m| m.error);
    let code = err.as_ref().map(|e| e.code.clone()).unwrap_or_default();
    let message = err.as_ref().map(|e| e.message.clone()).unwrap_or_default();
    let combined = format!("{code} {message}").to_lowercase();
    let status_code = status.as_u16();

    let auth_error = combined.contains("signature")
        || combined.contains("auth")
        || combined.contains("credential")
        || combined.contains("accesskey")
        || combined.contains("token")
        || status_code == 401
        || status_code == 403;
    let rate_limited = combined.contains("throttl")
        || combined.contains("flowlimit")
        || combined.contains("ratelimit")
        || status_code == 429;

    let detail = if code.is_empty() {
        format!("HTTP {status_code}")
    } else {
        format!("{code}: {message}")
    };
    if auth_error {
        ProviderError::Auth {
            message: format!("Volcano rejected the request ({detail})"),
        }
    } else if rate_limited {
        ProviderError::RateLimited
    } else {
        ProviderError::Network {
            message: format!("Volcano API error ({detail})"),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct AfpUsageResponse {
    /// Result body. Volcengine wraps responses in `Result`.
    #[serde(default, alias = "Result")]
    result: Option<AfpUsageBody>,
}

#[derive(Debug, Deserialize, Default)]
struct AfpUsageBody {
    #[serde(default, alias = "AFPFiveHour")]
    afp_five_hour: Option<WindowRaw>,
    #[serde(default, alias = "AFPDaily")]
    afp_daily: Option<WindowRaw>,
    #[serde(default, alias = "AFPWeekly")]
    afp_weekly: Option<WindowRaw>,
    #[serde(default, alias = "AFPMonthly")]
    afp_monthly: Option<WindowRaw>,
    /// Plan tier name: "Small" | "Medium" | "Large" | "Max".
    #[serde(default, alias = "PlanType")]
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WindowRaw {
    /// AFP value; the API returns strings ("50.0") but accept numbers too.
    #[serde(default, alias = "Quota")]
    quota: Option<FlexibleNum>,
    #[serde(default, alias = "Used")]
    used: Option<FlexibleNum>,
    /// Unix milliseconds.
    #[serde(default, alias = "ResetTime")]
    reset_time: Option<i64>,
}

impl WindowRaw {
    fn into_window(self, unit: UsageUnit) -> Option<WindowUsage> {
        let quota = self.quota.as_ref().and_then(FlexibleNum::value)?;
        let used = self.used.as_ref().and_then(FlexibleNum::value)?;
        let reset_at = self.reset_time.and_then(|ms| {
            chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.with_timezone(&Utc))
        });
        Some(WindowUsage {
            used,
            quota,
            unit,
            reset_at,
            over_quota: used > quota,
            cost_source: crate::providers::CostSource::ProviderReported,
            tokens: None,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
struct UsageDetailsResponse {
    #[serde(default, alias = "Result")]
    result: Option<UsageDetailsBody>,
}

#[derive(Debug, Deserialize, Default)]
struct UsageDetailsBody {
    #[serde(default, alias = "Details")]
    details: Vec<DetailRaw>,
}

#[derive(Debug, Deserialize)]
struct DetailRaw {
    /// Unix milliseconds.
    #[serde(default, alias = "Time")]
    time: Option<i64>,
    #[serde(default, alias = "Usage")]
    usage: Option<FlexibleNum>,
    /// "Tokens" | "Images"
    #[serde(default, alias = "Unit")]
    unit: Option<String>,
    /// "WithinPlan" | "OutsideOfPlan"
    #[serde(default, alias = "BillingType")]
    billing_type: Option<String>,
}

/// Aggregate raw detail rows into one heatmap cell per Beijing-calendar day.
/// Prefers token-billed rows; when the window has no token rows at all
/// (e.g. AgentPlan windows billed in AFP points), the dominant non-token
/// unit group is used instead of returning an empty heatmap.
fn aggregate_daily(details: Vec<DetailRaw>, beijing: FixedOffset) -> Vec<HeatmapCell> {
    let mut groups: BTreeMap<String, BTreeMap<NaiveDate, f64>> = BTreeMap::new();
    for d in details {
        let key = d
            .unit
            .as_deref()
            .map(|u| u.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "tokens".to_string());
        let Some(ts) = d.time else { continue };
        let Some(usage) = d.usage.as_ref().and_then(FlexibleNum::value) else {
            continue;
        };
        let Some(dt) = chrono::DateTime::from_timestamp_millis(ts) else {
            continue;
        };
        let date = dt.with_timezone(&beijing).date_naive();
        *groups.entry(key).or_default().entry(date).or_default() += usage;
    }
    let chosen = groups
        .iter()
        .find(|(key, _)| key.as_str() == "tokens")
        .or_else(|| {
            groups
                .iter()
                .max_by(|(_, a), (_, b)| {
                    a.values()
                        .sum::<f64>()
                        .partial_cmp(&b.values().sum::<f64>())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
    let Some((key, days)) = chosen else {
        return Vec::new();
    };
    let unit = unit_from_key(key);
    days.iter()
        .map(|(date, value)| HeatmapCell {
            date: *date,
            value: *value,
            unit,
        })
        .collect()
}

fn unit_from_key(key: &str) -> UsageUnit {
    match key {
        "afp" => UsageUnit::Afp,
        "cny" => UsageUnit::Cny,
        "credits" | "images" => UsageUnit::Credits,
        _ => UsageUnit::Tokens,
    }
}

/// Parse a GetUsageDetails body into heatmap cells. Any failure (bad JSON or
/// nothing aggregatable) yields None instead of aborting the whole snapshot,
/// so the AFP windows above stay intact.
fn parse_heatmap(text: &str, beijing: FixedOffset) -> Option<Vec<HeatmapCell>> {
    let parsed: UsageDetailsResponse = match serde_json::from_str(text) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, body_chars = text.len(), "GetUsageDetails: parse failed");
            return None;
        }
    };
    let cells = aggregate_daily(parsed.result.map(|b| b.details).unwrap_or_default(), beijing);
    if cells.is_empty() {
        None
    } else {
        tracing::debug!(cells = cells.len(), "GetUsageDetails: heatmap cells built");
        Some(cells)
    }
}

/// Date range for the GetUsageDetails Filter, as Beijing calendar days.
/// The API rejects spans longer than 31 days (HTTP 400
/// InvalidParameter.StartTime/EndTime), so we clamp to a 30-day span
/// (31 calendar days inclusive).
fn details_window(now: DateTime<Utc>, beijing: FixedOffset) -> (NaiveDate, NaiveDate) {
    let today_bj = now.with_timezone(&beijing).date_naive();
    (today_bj - chrono::Duration::days(30), today_bj)
}
pub struct VolcengineProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl VolcengineProvider {
    pub fn new(
        http: Client,
        storage: Arc<Storage>,
        instance_id: String,
        label: String,
    ) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }

    fn afp_query() -> String {
        "Action=GetAFPUsage&Version=2024-01-01".to_string()
    }

    /// Sign and send a POST request to Volcengine Ark. Returns the response
    /// body text after verifying that no `ResponseMetadata.Error` is present
    /// (Volcengine sometimes returns HTTP 200 with an embedded error).
    async fn signed_post(
        &self,
        access_key: &str,
        secret_key: &str,
        query: &str,
        body: serde_json::Value,
    ) -> Result<String, ProviderError> {
        signed_post(&self.http, access_key, secret_key, query, body).await
    }
}

/// Sign and send a POST request to Volcengine Ark, shared by the AFP
/// (AgentPlan) provider and the pay-as-you-go `volcengine_api` variant.
pub(crate) async fn signed_post(
    client: &Client,
    access_key: &str,
    secret_key: &str,
    query: &str,
    body: serde_json::Value,
) -> Result<String, ProviderError> {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| Vec::new());
    let signed = signing::sign(
        access_key,
        secret_key,
        "POST",
        HOST,
        "/",
        query,
        "application/json",
        &body_bytes,
        Utc::now(),
    );
    let resp = client
        .post(format!("{BASE_URL}?{query}"))
        .header("Host", &signed.host)
        .header("X-Date", &signed.x_date)
        .header("X-Content-Sha256", &signed.x_content_sha256)
        .header("Authorization", &signed.authorization)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await?;
    let envelope: ErrorEnvelope = serde_json::from_str(&text).unwrap_or_default();
    let has_error = envelope
        .response_metadata
        .as_ref()
        .and_then(|m| m.error.as_ref())
        .is_some();
    if !status.is_success() || has_error {
        return Err(map_api_error(status, &text));
    }
    Ok(text)
}

#[async_trait]
impl Provider for VolcengineProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "volcengine"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::AccessKeySecret
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let (access_key, secret_key) = match creds {
            Credentials::AccessKeySecret {
                access_key,
                secret_key,
            } => (access_key.clone(), secret_key.clone()),
            _ => {
                return Err(ProviderError::Auth {
                    message: "Volcano requires AccessKeySecret credentials".into(),
                })
            }
        };
        if access_key.trim().is_empty() || access_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        // Beijing calendar days drive both the API date filter and the
        // heatmap bucketing (the account is billed on Beijing time).
        let beijing = FixedOffset::east_opt(8 * 3600).expect("UTC+8 is a valid offset");
        let now = Utc::now();
        let (start_date, end_date) = details_window(now, beijing);
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let end_str = end_date.format("%Y-%m-%d").to_string();

        // 1. AFP usage (4 windows + plan tier).
        let afp_text = self
            .signed_post(&access_key, &secret_key, &Self::afp_query(), json!({}))
            .await?;
        let afp_resp: AfpUsageResponse = serde_json::from_str(&afp_text)?;
        let afp = afp_resp.result.unwrap_or_default();
        let unit = UsageUnit::Afp;

        let windows = UsageWindows {
            five_hour: afp.afp_five_hour.and_then(|w| w.into_window(unit)),
            daily: afp.afp_daily.and_then(|w| w.into_window(unit)),
            weekly: afp.afp_weekly.and_then(|w| w.into_window(unit)),
            monthly: afp.afp_monthly.and_then(|w| w.into_window(unit)),
            balance: None,
        };

        // 2. Usage details for the last 31 days, daily granularity. The
        //    heatmap is best-effort: a details failure must not drop the AFP
        //    windows above.
        let body = json!({
            "QueryInterval": "Day",
            "Filter": {
                "StartTime": start_str,
                "EndTime": end_str,
            }
        });
        let details_resp = self
            .signed_post(
                &access_key,
                &secret_key,
                "Action=GetUsageDetails&Version=2024-01-01",
                body,
            )
            .await;

        let api_heatmap: Option<Vec<HeatmapCell>> = match details_resp {
            Ok(text) => parse_heatmap(&text, beijing),
            Err(e) => {
                tracing::warn!(error = %e, "GetUsageDetails: request failed, skipping heatmap");
                None
            }
        };

        // 火山 API 只回最近 31 天明细，且每次轮询都是全量重拉。把每次
        // 拉到的日用量落入本地存储（record_daily_on 峰值语义，幂等），
        // 日历热力图才能逐月累积出 >31 天的历史。嵌入快照的热力图改用
        // 本地合并视图（含刚写入的最新 31 天）。
        if let Some(cells) = &api_heatmap {
            for c in cells {
                let _ = self
                    .storage
                    .record_daily_on(self.instance_id.as_str(), c.date, c.value, c.unit);
            }
        }
        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.instance_id.as_str(), 200)
            .ok()
            .filter(|v| !v.is_empty())
            .or(api_heatmap);

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: afp.plan_type,
            timestamp: now,
            windows,
            heatmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_heatmap_returns_none_on_garbage() {
        assert!(parse_heatmap("not json", beijing()).is_none());
    }

    #[test]
    fn parse_heatmap_routes_rows_through_aggregate_daily() {
        let body = r#"{"result":{"details":[{"time":1789313400000,"usage":2.5,"unit":"Tokens"}]}}"#;
        let cells = parse_heatmap(body, beijing()).expect("cells");
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].value, 2.5);
        assert_eq!(cells[0].unit, UsageUnit::Tokens);
    }

    #[test]
    fn details_window_spans_at_most_31_days() {
        let now = Utc::now();
        let (start, end) = details_window(now, beijing());
        assert_eq!(end, now.with_timezone(&beijing()).date_naive());
        let span = (end - start).num_days();
        assert!(span <= 30, "span {} days exceeds the 31-day API limit", span);
    }
    const SEP13_BJ_2330_MS: i64 = 1_789_313_400_000;
    const SEP14_BJ_1200_MS: i64 = 1_789_358_400_000;

    fn row(ts_ms: i64, usage: FlexibleNum, unit: Option<&str>) -> DetailRaw {
        DetailRaw {
            time: Some(ts_ms),
            usage: Some(usage),
            unit: unit.map(str::to_string),
            billing_type: None,
        }
    }

    fn num(v: f64) -> FlexibleNum {
        FlexibleNum::Number(v)
    }

    fn beijing() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    #[test]
    fn token_rows_aggregate_per_beijing_day() {
        let cells = aggregate_daily(
            vec![
                row(SEP13_BJ_2330_MS, num(3.0), Some("Tokens")),
                row(
                    SEP13_BJ_2330_MS,
                    FlexibleNum::Text("2.0".to_string()),
                    Some("tokens"),
                ),
                row(SEP14_BJ_1200_MS, num(7.0), Some("Tokens")),
            ],
            beijing(),
        );
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].date.to_string(), "2026-09-13");
        assert_eq!(cells[0].value, 5.0);
        assert_eq!(cells[0].unit, UsageUnit::Tokens);
        assert_eq!(cells[1].date.to_string(), "2026-09-14");
        assert_eq!(cells[1].value, 7.0);
    }

    #[test]
    fn rows_without_unit_count_as_tokens() {
        let cells = aggregate_daily(vec![row(SEP14_BJ_1200_MS, num(4.0), None)], beijing());
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].value, 4.0);
        assert_eq!(cells[0].unit, UsageUnit::Tokens);
    }

    #[test]
    fn rows_missing_time_or_usage_are_skipped() {
        let mut no_time = row(SEP14_BJ_1200_MS, num(1.0), Some("Tokens"));
        no_time.time = None;
        let mut no_usage = row(SEP14_BJ_1200_MS, num(1.0), Some("Tokens"));
        no_usage.usage = None;
        let cells = aggregate_daily(vec![no_time, no_usage], beijing());
        assert!(cells.is_empty());
    }

    #[test]
    fn non_token_rows_fall_back_instead_of_empty() {
        let cells = aggregate_daily(
            vec![
                row(SEP13_BJ_2330_MS, num(8.0), Some("AFP")),
                row(SEP14_BJ_1200_MS, num(2.0), Some("AFP")),
            ],
            beijing(),
        );
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].date.to_string(), "2026-09-13");
        assert_eq!(cells[0].value, 8.0);
        assert_eq!(cells[0].unit, UsageUnit::Afp);
        assert_eq!(cells[1].value, 2.0);
    }

    #[test]
    fn dominant_non_token_group_is_chosen() {
        let cells = aggregate_daily(
            vec![
                row(SEP13_BJ_2330_MS, num(3.0), Some("AFP")),
                row(SEP14_BJ_1200_MS, num(100.0), Some("Images")),
            ],
            beijing(),
        );
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].date.to_string(), "2026-09-14");
        assert_eq!(cells[0].value, 100.0);
    }

    #[test]
    fn token_rows_take_priority_over_other_units() {
        let cells = aggregate_daily(
            vec![
                row(SEP14_BJ_1200_MS, num(5.0), Some("Tokens")),
                row(SEP14_BJ_1200_MS, num(999.0), Some("Images")),
            ],
            beijing(),
        );
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].value, 5.0);
        assert_eq!(cells[0].unit, UsageUnit::Tokens);
    }
}
