//! xAI (Grok) usage provider.
//!
//! Endpoint:
//!   - `GET https://api.x.ai/v1/usage?start_date=...&end_date=...&granularity=daily`
//!     Returns per-day usage records; each record carries a cost in USD.
//!     We aggregate the current calendar month's cost into `windows.monthly`
//!     and persist every day via `record_daily_on` so the heatmap keeps
//!     history across polls.
//!
//! xAI publishes no subscription/quota endpoint for API credits, so the
//! monthly window has `quota = 0` (aggregate view) and `plan_tier = None`.
//!
//! NOTE: the response schema is not fully documented. `data[].cost` is
//! confirmed, the per-item timestamp field name is not — we parse
//! defensively (skip records without a usable timestamp) and pin the
//! expected shape with the fixture tests below.
//!
//! Auth: `Authorization: Bearer <API Key>`
//!
//! Reference: <https://docs.x.ai/docs/api-reference#usage>

use crate::providers::{
    AuthKind, CostSource, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const USAGE_URL: &str = "https://api.x.ai/v1/usage";

/// How many days of history we request in one call.
const HISTORY_DAYS: i64 = 99;

pub struct XaiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl XaiProvider {
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
struct UsageRecord {
    /// Unix timestamp (UTC seconds). Field name not fully confirmed —
    /// records without a usable timestamp are skipped defensively.
    #[serde(default)]
    timestamp: i64,
    /// The record's cost in USD (`cost` confirmed; alias for safety).
    #[serde(default, alias = "cost_usd")]
    cost: f64,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    data: Vec<UsageRecord>,
}

/// Aggregate heatmap cells into the monthly `WindowUsage`: current calendar
/// month only, USD, provider-reported, no fixed quota (prepaid credits).
fn month_usage(cells: &[HeatmapCell], today: NaiveDate) -> WindowUsage {
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let used: f64 = cells
        .iter()
        .filter(|c| c.date >= month_start)
        .map(|c| c.value)
        .sum();
    let next_month_start = if today.month() == 12 {
        NaiveDate::from_ymd_opt(today.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1).unwrap()
    };
    WindowUsage {
        used,
        quota: 0.0,
        unit: UsageUnit::Usd,
        reset_at: Some(next_month_start.and_hms_opt(0, 0, 0).unwrap().and_utc()),
        over_quota: false,
        cost_source: CostSource::ProviderReported,
        tokens: None,
    }
}

#[async_trait]
impl Provider for XaiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "xai"
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
                    message: "xAI requires BearerKey credentials".into(),
                })
            }
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let today = Local::now().date_naive();
        let start = today - Duration::days(HISTORY_DAYS);
        let usage_url = format!(
            "{USAGE_URL}?start_date={start}T00:00:00Z&end_date={today}T00:00:00Z&granularity=daily&group_by=model",
            start = start.format("%Y-%m-%d"),
            today = today.format("%Y-%m-%d"),
        );
        let resp = self.http.get(&usage_url).bearer_auth(&api_key).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "xAI rejected the API key".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("xAI usage HTTP {}", resp.status()),
            });
        }
        let usage: UsageResponse = resp.json().await?;

        // Build heatmap cells (local calendar date -> USD) and persist them
        // so history survives beyond the API's rolling window.
        let mut cells: Vec<HeatmapCell> = Vec::new();
        for record in &usage.data {
            if record.timestamp <= 0 {
                continue;
            }
            if let Some(date) = DateTime::from_timestamp(record.timestamp, 0)
                .map(|dt| dt.with_timezone(&Local).date_naive())
            {
                if date > today {
                    continue;
                }
                let value = record.cost.max(0.0);
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

        let monthly = Some(month_usage(&cells, today));

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 200)
            .ok()
            .filter(|v| !v.is_empty());

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                monthly,
                ..Default::default()
            },
            heatmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_data_with_defensive_fields() {
        let raw = r#"{"data":[{"timestamp":1758969600,"cost":1.25},{"cost_usd":0.75}]}"#;
        let parsed: UsageResponse = serde_json::from_str(raw).expect("parse usage fixture");
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].timestamp, 1758969600);
        assert!((parsed.data[0].cost - 1.25).abs() < 1e-9);
        assert_eq!(parsed.data[1].timestamp, 0);
        assert!((parsed.data[1].cost - 0.75).abs() < 1e-9);
    }

    #[test]
    fn month_usage_sums_only_current_calendar_month() {
        let cells = vec![
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 8, 20).unwrap(),
                value: 10.0,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 3).unwrap(),
                value: 1.25,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
                value: 0.75,
                unit: UsageUnit::Usd,
            },
        ];
        let today = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let window = month_usage(&cells, today);
        assert!((window.used - 2.0).abs() < 1e-9);
        assert_eq!(window.quota, 0.0);
        assert_eq!(window.unit, UsageUnit::Usd);
        assert!(matches!(window.cost_source, CostSource::ProviderReported));
        assert!(!window.over_quota);
        assert!(window.reset_at.is_some());
    }
}
