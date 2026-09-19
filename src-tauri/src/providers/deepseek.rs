//! DeepSeek API provider.
//!
//! Endpoints:
//!   - `GET https://api.deepseek.com/user/balance`
//!     Returns CNY balance (total / granted / topped_up).
//!
//! DeepSeek publishes **no usage/billing endpoint** (only `/user/balance`
//! exists; `/v1/billing/usage` returns 404), so usage is derived locally by
//! *balance-delta accounting*:
//!
//!   - balance drops  -> the drop is today's spend (accumulated into the
//!     daily heatmap via `Storage::accumulate_daily`);
//!   - balance rises  -> the rise is a top-up (baselined, excluded from
//!     spend, and tracked per calendar month);
//!   - monthly window = month-start balance + top-ups this month - current
//!     balance (DeepSeek is prepaid, so there is no fixed quota).
//!
//! Limitation: on the very first poll there is no baseline yet, so the
//! heatmap and monthly figure start accumulating from that moment.
//!
//! Auth: `Authorization: Bearer <API Key>`
//!
//! Reference: <https://api-docs.deepseek.com/api/get-user-balance>

use crate::providers::{
    AuthKind, BalanceInfo, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";

const KV_LAST_BALANCE: &str = "last_balance";
const KV_MONTH_KEY: &str = "month_key";
const KV_MONTH_START_BALANCE: &str = "month_start_balance";
const KV_MONTH_TOPUP: &str = "month_topup";

/// Balances below this difference are treated as unchanged (float dust).
const DELTA_EPSILON: f64 = 1e-6;

fn fmt_f64(v: f64) -> String {
    format!("{v:.6}")
}

#[derive(Debug, Deserialize)]
struct BalanceResponse {
    is_available: bool,
    #[serde(default)]
    balance_infos: Vec<BalanceInfoRaw>,
}

#[derive(Debug, Deserialize, Clone)]
struct BalanceInfoRaw {
    currency: String,
    total_balance: String,
    granted_balance: String,
    topped_up_balance: String,
}

pub struct DeepSeekProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl DeepSeekProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }

    /// Storage keyspace / account id for this instance.
    fn self_id(&self) -> &str {
        &self.instance_id
    }

    /// Account for a balance change since the previous poll.
    fn record_balance_delta(&self, current: f64) {
        let baseline = self
            .storage
            .kv_get(self.self_id(), KV_LAST_BALANCE)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok());

        match baseline {
            None => {
                // First observation: just establish the baseline.
                let _ = self
                    .storage
                    .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
            }
            Some(prev) => {
                let delta = prev - current;
                if delta > DELTA_EPSILON {
                    // Balance dropped -> that drop is spend.
                    let _ = self
                        .storage
                        .accumulate_daily(self.self_id(), delta, UsageUnit::Cny);
                    let _ = self
                        .storage
                        .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
                } else if delta < -DELTA_EPSILON {
                    // Balance rose -> top-up. Rebaseline and track the top-up
                    // for the monthly window.
                    let topup = self
                        .storage
                        .kv_get(self.self_id(), KV_MONTH_TOPUP)
                        .ok()
                        .flatten()
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let _ = self.storage.kv_set(
                        self.self_id(),
                        KV_MONTH_TOPUP,
                        &fmt_f64(topup + -delta),
                    );
                    let _ = self
                        .storage
                        .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
                }
            }
        }
    }

    /// Monthly spend window derived from balance bookkeeping. Handles the
    /// calendar-month rollover by rebasing on the current balance.
    fn month_spend_window(&self, current: f64) -> Option<WindowUsage> {
        let month_key = chrono::Local::now().format("%Y-%m").to_string();
        let stored_month = self.storage.kv_get(self.self_id(), KV_MONTH_KEY).ok().flatten();

        let month_start = if stored_month.as_deref() != Some(month_key.as_str()) {
            // New month: the month starts from the current balance.
            let _ = self.storage.kv_set(self.self_id(), KV_MONTH_KEY, &month_key);
            let _ = self
                .storage
                .kv_set(self.self_id(), KV_MONTH_START_BALANCE, &fmt_f64(current));
            let _ = self.storage.kv_set(self.self_id(), KV_MONTH_TOPUP, "0");
            current
        } else {
            self.storage
                .kv_get(self.self_id(), KV_MONTH_START_BALANCE)
                .ok()
                .flatten()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(current)
        };

        let topup = self
            .storage
            .kv_get(self.self_id(), KV_MONTH_TOPUP)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(0.0);

        let used = (month_start + topup - current).max(0.0);
        Some(WindowUsage {
            used,
            quota: 0.0, // Prepaid balance - no fixed quota.
            unit: UsageUnit::Cny,
            reset_at: None,
            over_quota: false,
        })
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "deepseek"
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
                message: "DeepSeek requires BearerKey credentials".into(),
            }),
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let resp = self.http.get(BALANCE_URL).bearer_auth(&api_key).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "DeepSeek rejected the API key".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("DeepSeek balance HTTP {}", resp.status()),
            });
        }
        let parsed: BalanceResponse = resp.json().await?;
        let raw = parsed
            .balance_infos
            .iter()
            .find(|b| b.currency == "CNY")
            .or_else(|| parsed.balance_infos.first())
            .cloned();

        let mut monthly = None;
        if let Some(raw) = &raw {
            // Only run the delta accounting when the balance is parseable;
            // otherwise a garbage value would corrupt the baselines.
            if let Ok(current) = raw.total_balance.trim().parse::<f64>() {
                self.record_balance_delta(current);
                monthly = self.month_spend_window(current);
            }
        }

        let balance = raw.map(|raw| BalanceInfo {
            total: raw.total_balance.parse().unwrap_or(0.0),
            granted: raw.granted_balance.parse().unwrap_or(0.0),
            topped_up: raw.topped_up_balance.parse().unwrap_or(0.0),
            currency: raw.currency,
            is_available: parsed.is_available,
        });

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
                balance,
                monthly,
                ..Default::default()
            },
            heatmap,
        })
    }
}
