//! Volcengine (火山方舟) pay-as-you-go API provider.
//!
//! Queries the official **GetInferenceUsage** endpoint to surface real,
//! post-paid ("按量") token consumption for this account. This is the
//! pay-as-you-go companion to the AFP/AgentPlan `volcengine` provider.
//!
//! Endpoint (POST to `https://ark.cn-beijing.volcengineapi.com/`):
//!   - `?Action=GetInferenceUsage&Version=2024-01-01`
//!     Body `{"QueryInterval":"Day","StartTime":"YYYY-MM-DD","EndTime":"YYYY-MM-DD"}`
//!     Returns `Result.Fields` (column names) + `Result.Data` (2D row arrays),
//!     one row per interval. Columns include InputTokens / OutputTokens /
//!     TotalTokens / ImageCount / Day / Hour.
//!
//! Auth: Volcengine V4 HMAC-SHA256 signature using Access Key + Secret Key,
//! exactly like the AFP `volcengine` provider (reuses `signed_post`).
//!
//! Reference: <https://docs.volcengine.com/docs/82379/2116766>
//!
//! Note: The endpoint exposes token *usage*, not a balance, so this snapshot
//! reports month-to-date total tokens as the consumption (quota = 0 marks it
//! pay-as-you-go). A balance figure is intentionally omitted.

use crate::providers::volcengine::{signed_post, FlexibleNum};
use crate::providers::{
    AuthKind, CostSource, Credentials, Provider, ProviderError, TokenBreakdown, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{Datelike, FixedOffset, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub struct VolcengineApiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl VolcengineApiProvider {
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
}

/// JSON body of the GetInferenceUsage response.
#[derive(Debug, Deserialize, Default)]
struct InferenceUsageResp {
    #[serde(rename = "Result", default)]
    result: Option<InferenceUsageBody>,
}

#[derive(Debug, Deserialize, Default)]
struct InferenceUsageBody {
    #[serde(rename = "Fields", default)]
    fields: Vec<String>,
    #[serde(rename = "Data", default)]
    data: Vec<Vec<FlexibleNum>>,
}

/// Locate the index of a column by (case-insensitive) name. Falls back to the
/// first occurrence that matches the tail (`*Tokens`) so the endpoint's exact
/// column naming can vary without breaking aggregation.
fn col_index(fields: &[String], name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    fields
        .iter()
        .position(|f| f.to_ascii_lowercase() == lower)
        .or_else(|| {
            fields.iter().position(|f| {
                let f = f.to_ascii_lowercase();
                f.ends_with("tokens")
            })
        })
}

impl VolcengineApiProvider {
    /// Fetch month-to-date inference usage, returning `(total, input, output)`
    /// tokens aggregated across every reported day. Columns are located by name
    /// so the endpoint's exact field ordering can vary without breaking.
    async fn fetch_monthly_total(
        &self,
        creds: &Credentials,
    ) -> Result<(f64, f64, f64), ProviderError> {
        let (access_key, secret_key) = match creds {
            Credentials::AccessKeySecret {
                access_key,
                secret_key,
                ..
            } => (access_key, secret_key),
            _ => {
                return Err(ProviderError::Auth {
                    message: "Volcengine API 按量 requires AccessKey + SecretKey".into(),
                })
            }
        };

        let beijing = FixedOffset::east_opt(8 * 3600).unwrap();
        let now = Utc::now().with_timezone(&beijing);
        let start = now
            .date_naive()
            .with_day(1)
            .unwrap_or(now.date_naive());
        let end = now.date_naive();

        let body = json!({
            "QueryInterval": "Day",
            "StartTime": start.format("%Y-%m-%d").to_string(),
            "EndTime": end.format("%Y-%m-%d").to_string(),
        });

        let text = signed_post(
            &self.http,
            access_key,
            secret_key,
            "Action=GetInferenceUsage&Version=2024-01-01",
            body,
        )
        .await?;

        let parsed: InferenceUsageResp = serde_json::from_str(&text)?;
        let result = parsed.result.unwrap_or_default();
        let fields = result.fields;
        if fields.is_empty() {
            return Ok((0.0, 0.0, 0.0));
        }
        let total_idx = col_index(&fields, "TotalTokens").ok_or_else(|| ProviderError::Parse {
            message: "GetInferenceUsage response missing TotalTokens column".into(),
        })?;
        let input_idx = col_index(&fields, "InputTokens");
        let output_idx = col_index(&fields, "OutputTokens");

        let mut total = 0.0f64;
        let mut input = 0.0f64;
        let mut output = 0.0f64;
        for row in &result.data {
            if let Some(v) = row.get(total_idx).and_then(FlexibleNum::value) {
                total += v;
            }
            if let Some(idx) = input_idx {
                if let Some(v) = row.get(idx).and_then(FlexibleNum::value) {
                    input += v;
                }
            }
            if let Some(idx) = output_idx {
                if let Some(v) = row.get(idx).and_then(FlexibleNum::value) {
                    output += v;
                }
            }
        }
        Ok((total, input, output))
    }
}

#[async_trait]
impl Provider for VolcengineApiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "volcengine_api"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::AccessKeySecret
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let (total, input, output) = self.fetch_monthly_total(creds).await?;

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                monthly: Some(WindowUsage {
                    used: total,
                    quota: 0.0, // no fixed quota -> pay-as-you-go
                    unit: UsageUnit::Tokens,
                    reset_at: None,
                    over_quota: false,
                    cost_source: CostSource::ProviderReported,
                    tokens: Some(TokenBreakdown {
                        input,
                        cache_read: 0.0, // GetInferenceUsage does not split cache hits
                        output,
                        model_id: None,
                    }),
                }),
                // GetInferenceUsage has no balance field.
                balance: None,
                ..Default::default()
            },
            heatmap: None,
        })
    }
}