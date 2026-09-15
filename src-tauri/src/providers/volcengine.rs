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
use async_trait::async_trait;
use chrono::{FixedOffset, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

const HOST: &str = "ark.cn-beijing.volcengineapi.com";
const BASE_URL: &str = "https://ark.cn-beijing.volcengineapi.com/";

/// Numbers in Volcengine responses are sometimes JSON numbers and sometimes
/// strings ("50.0"); accept both.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FlexibleNum {
    Number(f64),
    Text(String),
}

impl FlexibleNum {
    fn value(&self) -> Option<f64> {
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
/// Only token-billed rows are counted (image rows have a different unit and
/// must not be mixed into the token heatmap).
fn aggregate_daily(details: Vec<DetailRaw>, beijing: FixedOffset) -> Vec<HeatmapCell> {
    let mut per_day: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for d in details {
        if let Some(unit) = d.unit.as_deref() {
            if !unit.eq_ignore_ascii_case("tokens") {
                continue;
            }
        }
        let Some(ts) = d.time else { continue };
        let Some(usage) = d.usage.as_ref().and_then(FlexibleNum::value) else {
            continue;
        };
        let Some(dt) = chrono::DateTime::from_timestamp_millis(ts) else {
            continue;
        };
        let date = dt.with_timezone(&beijing).date_naive();
        *per_day.entry(date).or_default() += usage;
    }
    per_day
        .into_iter()
        .map(|(date, value)| HeatmapCell {
            date,
            value,
            unit: UsageUnit::Tokens,
        })
        .collect()
}

pub struct VolcengineProvider {
    http: Client,
}

impl VolcengineProvider {
    pub fn new(http: Client) -> Self {
        Self { http }
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
        let resp = self
            .http
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
}

#[async_trait]
impl Provider for VolcengineProvider {
    fn id(&self) -> &'static str {
        "volcengine"
    }

    fn display_name(&self) -> &'static str {
        "Volcano AgentPlan"
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
        let today_bj = now.with_timezone(&beijing);
        let start_bj = today_bj - chrono::Duration::days(90);
        let start_str = start_bj.format("%Y-%m-%d").to_string();
        let end_str = today_bj.format("%Y-%m-%d").to_string();

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

        // 2. Usage details for the last 90 days, daily granularity. The
        // heatmap is best-effort: a details failure must not drop the AFP
        // windows above.
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

        let heatmap: Option<Vec<HeatmapCell>> = match details_resp {
            Ok(text) => {
                let parsed: UsageDetailsResponse = serde_json::from_str(&text)?;
                let cells = aggregate_daily(parsed.result.map(|b| b.details).unwrap_or_default(), beijing);
                if cells.is_empty() {
                    None
                } else {
                    Some(cells)
                }
            }
            Err(_) => None,
        };

        Ok(UsageSnapshot {
            provider_id: self.id().to_string(),
            provider_display_name: self.display_name().to_string(),
            plan_tier: afp.plan_type,
            timestamp: now,
            windows,
            heatmap,
        })
    }
}
