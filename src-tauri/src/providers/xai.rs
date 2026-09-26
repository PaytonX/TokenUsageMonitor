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
//! defensively and pin the expected shape with the fixture tests below.
//! A `null` or wrong-typed timestamp or cost is tolerated: a record with
//! an unusable timestamp is skipped, and a missing/null cost is treated
//! as 0. Because the request groups by model, the API may return several
//! records for the same calendar day; same-day (e.g. per-model) records
//! are summed into one daily cell before persistence.
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
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
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

/// Tolerate a missing, `null`, or wrong-typed timestamp field: numbers
/// (integer or float) and numeric strings are accepted; anything else is
/// `None` so the record can be skipped.
fn deserialize_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64)),
        Some(serde_json::Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

/// Tolerate a missing, `null`, or wrong-typed cost field: numbers and
/// numeric strings are accepted; anything else is `None` (treated as 0).
fn deserialize_opt_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
struct UsageRecord {
    /// Unix timestamp (UTC seconds). Field name not fully confirmed —
    /// missing, `null`, or wrong-typed values become `None` and the
    /// record is skipped defensively.
    #[serde(default, deserialize_with = "deserialize_opt_i64")]
    timestamp: Option<i64>,
    /// The record's cost in USD (`cost` confirmed; alias for safety).
    /// Missing, `null`, or wrong-typed values become `None` (= 0.0).
    #[serde(default, alias = "cost_usd", deserialize_with = "deserialize_opt_f64")]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    data: Vec<UsageRecord>,
}

/// Aggregate raw records into one heatmap cell per local calendar day.
///
/// Records without a usable timestamp (`None` or `<= 0`) are skipped, as
/// are records whose local date is in the future. Costs are summed per
/// local date; a missing/null cost contributes `0.0`.
fn daily_cells(records: &[UsageRecord], today: NaiveDate) -> Vec<HeatmapCell> {
    let mut totals: HashMap<NaiveDate, f64> = HashMap::new();
    for record in records {
        let Some(ts) = record.timestamp else {
            continue;
        };
        if ts <= 0 {
            continue;
        }
        let Some(date) = DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.with_timezone(&Local).date_naive())
        else {
            continue;
        };
        if date > today {
            continue;
        }
        *totals.entry(date).or_insert(0.0) += record.cost.unwrap_or(0.0);
    }
    let mut cells: Vec<HeatmapCell> = totals
        .into_iter()
        .map(|(date, total)| HeatmapCell {
            date,
            value: total.max(0.0),
            unit: UsageUnit::Usd,
        })
        .collect();
    cells.sort_by_key(|c| c.date);
    cells
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

        // Aggregate per-model records into per-day totals (local calendar
        // date -> USD) and persist each day once so history survives beyond
        // the API's rolling window.
        let cells = daily_cells(&usage.data, today);
        for cell in &cells {
            let _ = self.storage.record_daily_on(
                self.self_id(),
                cell.date,
                cell.value,
                UsageUnit::Usd,
            );
        }

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
        assert_eq!(parsed.data[0].timestamp, Some(1758969600));
        assert!((parsed.data[0].cost.unwrap() - 1.25).abs() < 1e-9);
        assert_eq!(parsed.data[1].timestamp, None);
        assert!((parsed.data[1].cost.unwrap() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn parses_usage_data_tolerating_null_and_wrong_types() {
        let raw = r#"{"data":[{"timestamp":null,"cost":1.0},{"timestamp":"1758969600","cost":"2.50"},{"timestamp":1758969600},{"cost":null}]}"#;
        let parsed: UsageResponse = serde_json::from_str(raw).expect("parse usage fixture");
        assert_eq!(parsed.data.len(), 4);
        assert_eq!(parsed.data[0].timestamp, None);
        assert_eq!(parsed.data[1].timestamp, Some(1758969600));
        assert!((parsed.data[1].cost.unwrap() - 2.5).abs() < 1e-9);
        assert_eq!(parsed.data[2].cost, None);
        assert_eq!(parsed.data[3].timestamp, None);
    }

    #[test]
    fn daily_cells_merges_records_on_same_day_and_skips_bad_rows() {
        let ts = NaiveDate::from_ymd_opt(2026, 9, 3)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let ts2 = NaiveDate::from_ymd_opt(2026, 9, 26)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let records = vec![
            UsageRecord {
                timestamp: Some(ts),
                cost: Some(1.25),
            },
            UsageRecord {
                timestamp: Some(ts),
                cost: Some(0.75),
            },
            UsageRecord {
                timestamp: None,
                cost: Some(9.0),
            },
            UsageRecord {
                timestamp: Some(0),
                cost: Some(9.0),
            },
            UsageRecord {
                timestamp: Some(ts2),
                cost: Some(5.0),
            },
        ];
        let cells = daily_cells(&records, NaiveDate::from_ymd_opt(2026, 9, 25).unwrap());
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].date, NaiveDate::from_ymd_opt(2026, 9, 3).unwrap());
        assert!((cells[0].value - 2.0).abs() < 1e-9);
        assert_eq!(cells[0].unit, UsageUnit::Usd);
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
