//! OpenAI billing provider.
//!
//! Endpoints:
//!   - `GET https://api.openai.com/v1/dashboard/billing/subscription`
//!     Returns the account's hard/soft usage limit (in USD) and plan tier.
//!   - `GET https://api.openai.com/v1/dashboard/billing/usage?start_date&end_date`
//!     Returns per-day costs (`daily_costs`) and the total for the range.
//!
//! Mapping into the unified snapshot:
//!   - `monthly`  -> this calendar month's spend (used) against `hard_limit_usd`
//!     (quota). When the account has no hard limit, `quota = 0` so the frontend
//!     falls back to the aggregate view just like DeepSeek's prepaid balance.
//!   - `heatmap`  -> per-day cost from `daily_costs`, persisted via
//!     `record_daily_on` so history survives across polls.
//!   - `plan_tier`-> the plan title from `subscription`.
//!
//! Auth: `Authorization: Bearer <API Key>`
//!
//! Reference: <https://platform.openai.com/docs/api-reference/usage>

use crate::providers::{
    AuthKind, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const SUB_URL: &str = "https://api.openai.com/v1/dashboard/billing/subscription";
const USAGE_URL: &str = "https://api.openai.com/v1/dashboard/billing/usage";

/// How many days of history we request (the heatmap grid shows 31 days, but
/// fetching a little more gives the frontend room to scroll further back).
const HISTORY_DAYS: i64 = 119;

pub struct OpenAIProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl OpenAIProvider {
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

#[derive(Debug, Deserialize)]
struct SubscriptionPlan {
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize)]
struct SubscriptionResponse {
    #[serde(default)]
    hard_limit_usd: f64,
    #[serde(default)]
    plan: Option<SubscriptionPlan>,
}

#[derive(Debug, Deserialize)]
struct DailyCost {
    /// Unix timestamp (UTC) of the start of the day.
    #[serde(default)]
    timestamp: i64,
    /// The day's total cost in USD.
    #[serde(default)]
    total: f64,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    daily_costs: Vec<DailyCost>,
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "openai"
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
            _ => {
                return Err(ProviderError::Auth {
                    message: "OpenAI requires BearerKey credentials".into(),
                })
            }
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        // --- Subscription: hard_limit_usd = quota, plan.title = tier. ---
        let sub_resp = self
            .http
            .get(SUB_URL)
            .bearer_auth(&api_key)
            .send()
            .await?;
        if sub_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "OpenAI rejected the API key".into(),
            });
        }
        if !sub_resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("OpenAI subscription HTTP {}", sub_resp.status()),
            });
        }
        let sub: SubscriptionResponse = sub_resp.json().await?;
        let quota = if sub.hard_limit_usd > 0.0 {
            sub.hard_limit_usd
        } else {
            0.0
        };
        let plan_tier = sub
            .plan
            .as_ref()
            .map(|p| p.title.trim().to_string())
            .filter(|t| !t.is_empty());

        // --- Usage: daily costs from HISTORY_DAYS ago through today. ---
        let today = Local::now().date_naive();
        let start = today - Duration::days(HISTORY_DAYS);
        let usage_url = format!(
            "{USAGE_URL}?start_date={}&end_date={}",
            start.format("%Y-%m-%d"),
            today.format("%Y-%m-%d"),
        );
        let usage_resp = self.http.get(&usage_url).bearer_auth(&api_key).send().await?;
        if !usage_resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("OpenAI usage HTTP {}", usage_resp.status()),
            });
        }
        let usage: UsageResponse = usage_resp.json().await?;

        // Build heatmap cells (local calendar date -> USD) and persist them so
        // history is preserved even when the API only serves a rolling window.
        let mut cells: Vec<HeatmapCell> = Vec::new();
        for day in &usage.daily_costs {
            if let Some(date) = DateTime::from_timestamp(day.timestamp, 0)
                .map(|dt| dt.with_timezone(&Local).date_naive())
            {
                if date > today {
                    continue;
                }
                let value = day.total.max(0.0);
                let _ = self
                    .storage
                    .record_daily_on(self.self_id(), date, value, UsageUnit::Usd);
                cells.push(HeatmapCell {
                    date,
                    value,
                    unit: UsageUnit::Usd,
                });
            }
        }
        cells.sort_by_key(|c| c.date);

        // This calendar month's spend -> the monthly window.
        let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        let month_used: f64 = cells
            .iter()
            .filter(|c| c.date >= month_start)
            .map(|c| c.value)
            .sum();
        let next_month_start = if today.month() == 12 {
            NaiveDate::from_ymd_opt(today.year() + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1).unwrap()
        };
        let monthly = Some(WindowUsage {
            used: month_used,
            quota,
            unit: UsageUnit::Usd,
            reset_at: Some(next_month_start.and_hms_opt(0, 0, 0).unwrap().and_utc()),
            over_quota: quota > 0.0 && month_used > quota,
        });

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 90)
            .ok()
            .filter(|v| !v.is_empty());

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier,
            timestamp: Utc::now(),
            windows: UsageWindows {
                monthly,
                ..Default::default()
            },
            heatmap,
        })
    }
}