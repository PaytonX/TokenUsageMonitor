//! Kimi (Moonshot AI) balance provider.
//!
//! Endpoint:
//!   - `GET {base}/v1/users/me/balance`
//!     Returns available / voucher / cash balance. The CN site
//!     (`https://api.moonshot.cn`) bills in CNY, the global site
//!     (`https://api.moonshot.ai`) bills in USD; API keys are NOT
//!     interchangeable, so the sites are separate account kinds
//!     (`kimi` / `kimi_global`) sharing this module via `new_with_base`.
//!
//! The API publishes no usage/billing endpoint, so usage is derived locally
//! by *balance-delta accounting* (same scheme as DeepSeek):
//!   - balance drops -> the drop is today's spend;
//!   - balance rises -> the rise is a top-up (baselined, excluded);
//!   - monthly window = month-start balance + top-ups - current balance.
//!
//! Limitation: on the very first poll there is no baseline yet, so the
//! heatmap and monthly figure start accumulating from that moment.
//!
//! Auth: `Authorization: Bearer <API Key>`

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

const CN_BASE: &str = "https://api.moonshot.cn";
pub const GLOBAL_BASE: &str = "https://api.moonshot.ai";

const KV_LAST_BALANCE: &str = "last_balance";
const KV_MONTH_KEY: &str = "month_key";
const KV_MONTH_START_BALANCE: &str = "month_start_balance";
const KV_MONTH_TOPUP: &str = "month_topup";

/// Balances below this difference are treated as unchanged (float dust).
const DELTA_EPSILON: f64 = 1e-6;

fn fmt_f64(v: f64) -> String {
    format!("{v:.6}")
}

/// Parse a float, rejecting `NaN` / infinity so a corrupted KV row can
/// neither freeze delta accounting nor leak into snapshots.
fn parse_finite(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Billing unit of a site: CNY on the CN host, USD on the global one.
fn site_usage_unit(base: &str) -> UsageUnit {
    if base == GLOBAL_BASE {
        UsageUnit::Usd
    } else {
        UsageUnit::Cny
    }
}

/// Currency code reported in `BalanceInfo` for a site.
fn site_currency(base: &str) -> &'static str {
    if base == GLOBAL_BASE {
        "USD"
    } else {
        "CNY"
    }
}

/// Kimi's balance response: `{ "code": 0, "data": { ... } }` — `code == 0`
/// means success, and all balances are JSON numbers (unlike DeepSeek's
/// string-encoded balances).
#[derive(Debug, Deserialize)]
struct BalanceResponse {
    #[serde(default)]
    code: i32,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: BalanceData,
}

#[derive(Debug, Deserialize, Default)]
struct BalanceData {
    #[serde(default)]
    available_balance: f64,
    #[serde(default)]
    voucher_balance: f64,
    #[serde(default)]
    cash_balance: f64,
}

pub struct KimiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
    base: &'static str,
}

impl KimiProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self::new_with_base(http, storage, instance_id, label, CN_BASE)
    }

    /// Site-specific constructor: `base` selects the CN or global API host.
    /// KV keys are namespaced by `instance_id`, so the two sites keep
    /// independent balance baselines.
    pub fn new_with_base(
        http: Client,
        storage: Arc<Storage>,
        instance_id: String,
        label: String,
        base: &'static str,
    ) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
            base,
        }
    }

    /// Storage keyspace / account id for this instance.
    fn self_id(&self) -> &str {
        &self.instance_id
    }

    /// Account for a balance change since the previous poll (same scheme as
    /// DeepSeek: drops = spend, rises = top-ups).
    fn record_balance_delta(&self, current: f64) {
        let baseline = self
            .storage
            .kv_get(self.self_id(), KV_LAST_BALANCE)
            .ok()
            .flatten()
            .and_then(|s| parse_finite(&s));

        match baseline {
            None => {
                let _ = self
                    .storage
                    .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
            }
            Some(prev) => {
                let delta = prev - current;
                if delta > DELTA_EPSILON {
                    let _ = self.storage.accumulate_daily(
                        self.self_id(),
                        delta,
                        site_usage_unit(self.base),
                    );
                    let _ = self
                        .storage
                        .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
                } else if delta < -DELTA_EPSILON {
                    let topup = self
                        .storage
                        .kv_get(self.self_id(), KV_MONTH_TOPUP)
                        .ok()
                        .flatten()
                        .and_then(|s| parse_finite(&s))
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
                .and_then(|s| parse_finite(&s))
                .unwrap_or(current)
        };

        let topup = self
            .storage
            .kv_get(self.self_id(), KV_MONTH_TOPUP)
            .ok()
            .flatten()
            .and_then(|s| parse_finite(&s))
            .unwrap_or(0.0);

        let used = (month_start + topup - current).max(0.0);
        Some(WindowUsage {
            used,
            quota: 0.0,
            unit: site_usage_unit(self.base),
            reset_at: None,
            over_quota: false,
            cost_source: crate::providers::CostSource::Estimated,
            tokens: None,
        })
    }
}

#[async_trait]
impl Provider for KimiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "kimi"
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
                    message: "Kimi requires BearerKey credentials".into(),
                })
            }
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let resp = self
            .http
            .get(format!("{}/v1/users/me/balance", self.base))
            .bearer_auth(&api_key)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "Kimi rejected the API key".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("Kimi balance HTTP {}", resp.status()),
            });
        }
        let parsed: BalanceResponse = resp.json().await?;

        // Moonshot reports business errors as HTTP 200 with a non-zero
        // `code`; surface them instead of returning an empty snapshot.
        if parsed.code != 0 {
            let detail = parsed.msg.map(|m| format!(": {m}")).unwrap_or_default();
            return Err(ProviderError::Network {
                message: format!("Kimi balance code {}{}", parsed.code, detail),
            });
        }
        if !parsed.data.available_balance.is_finite() {
            return Err(ProviderError::Network {
                message: "Kimi balance returned non-finite value".into(),
            });
        }

        let current = parsed.data.available_balance;
        self.record_balance_delta(current);
        let monthly = self.month_spend_window(current);
        let balance = Some(BalanceInfo {
            total: parsed.data.available_balance,
            granted: parsed.data.voucher_balance,
            topped_up: parsed.data.cash_balance,
            currency: site_currency(self.base).to_string(),
            is_available: true,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kimi_balance_response() {
        let raw = r#"{"code":0,"data":{"available_balance":110.5,"voucher_balance":10.25,"cash_balance":100.25},"scode":"ok","status":true}"#;
        let parsed: BalanceResponse = serde_json::from_str(raw).expect("parse balance fixture");
        assert_eq!(parsed.code, 0);
        assert!((parsed.data.available_balance - 110.5).abs() < 1e-9);
        assert!((parsed.data.voucher_balance - 10.25).abs() < 1e-9);
        assert!((parsed.data.cash_balance - 100.25).abs() < 1e-9);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        let parsed: BalanceResponse = serde_json::from_str("{}").expect("parse empty fixture");
        assert_eq!(parsed.code, 0);
        assert_eq!(parsed.data.available_balance, 0.0);
        assert_eq!(parsed.data.voucher_balance, 0.0);
        assert_eq!(parsed.data.cash_balance, 0.0);
    }

    #[test]
    fn site_selects_billing_unit_and_currency() {
        assert_eq!(site_usage_unit(CN_BASE), UsageUnit::Cny);
        assert_eq!(site_usage_unit(GLOBAL_BASE), UsageUnit::Usd);
        assert_eq!(site_currency(CN_BASE), "CNY");
        assert_eq!(site_currency(GLOBAL_BASE), "USD");
    }

    fn temp_storage(tag: &str) -> Arc<Storage> {
        let path = std::env::temp_dir().join(format!(
            "pulse_kimi_test_{}_{}_{}.db",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        Arc::new(Storage::open(&path).unwrap())
    }

    #[test]
    fn balance_error_code_surfaces_as_network_error() {
        let parsed: BalanceResponse =
            serde_json::from_str(r#"{"code":-1,"msg":"invalid api key"}"#)
                .expect("parse error fixture");
        assert_eq!(parsed.code, -1);
        assert_eq!(parsed.msg, Some("invalid api key".to_string()));
    }

    #[test]
    fn first_poll_establishes_baseline_only() {
        let storage = temp_storage("baseline");
        let provider =
            KimiProvider::new(Client::new(), storage.clone(), "k1".into(), "Kimi".into());
        provider.record_balance_delta(100.0);
        assert_eq!(
            storage.kv_get("k1", KV_LAST_BALANCE).unwrap(),
            Some("100.000000".to_string())
        );
        assert!(storage.load_heatmap("k1", 200).unwrap().is_empty());
    }

    #[test]
    fn balance_drop_records_spend() {
        let storage = temp_storage("drop");
        let provider =
            KimiProvider::new(Client::new(), storage.clone(), "k1".into(), "Kimi".into());
        provider.record_balance_delta(100.0);
        provider.record_balance_delta(90.0);
        let cells = storage.load_heatmap("k1", 200).unwrap();
        assert_eq!(cells.len(), 1);
        assert!((cells[0].value - 10.0).abs() < 1e-9);
        assert!(matches!(cells[0].unit, UsageUnit::Cny));
        assert_eq!(
            storage.kv_get("k1", KV_LAST_BALANCE).unwrap(),
            Some("90.000000".to_string())
        );
    }

    #[test]
    fn balance_rise_tracks_topup_without_spend() {
        let storage = temp_storage("rise");
        let provider =
            KimiProvider::new(Client::new(), storage.clone(), "k1".into(), "Kimi".into());
        provider.record_balance_delta(90.0);
        provider.record_balance_delta(100.0);
        assert!(storage.load_heatmap("k1", 200).unwrap().is_empty());
        assert_eq!(
            storage.kv_get("k1", KV_MONTH_TOPUP).unwrap(),
            Some("10.000000".to_string())
        );
        assert_eq!(
            storage.kv_get("k1", KV_LAST_BALANCE).unwrap(),
            Some("100.000000".to_string())
        );
    }

    #[test]
    fn tiny_delta_leaves_baseline_untouched() {
        let storage = temp_storage("tiny");
        let provider =
            KimiProvider::new(Client::new(), storage.clone(), "k1".into(), "Kimi".into());
        provider.record_balance_delta(100.0);
        provider.record_balance_delta(100.0000005);
        assert_eq!(
            storage.kv_get("k1", KV_LAST_BALANCE).unwrap(),
            Some("100.000000".to_string())
        );
        assert!(storage.load_heatmap("k1", 200).unwrap().is_empty());
        assert_eq!(storage.kv_get("k1", KV_MONTH_TOPUP).unwrap(), None);
    }

    #[test]
    fn corrupted_nan_baseline_is_ignored() {
        let storage = temp_storage("nan");
        let provider =
            KimiProvider::new(Client::new(), storage.clone(), "k1".into(), "Kimi".into());
        provider.record_balance_delta(100.0);
        storage
            .kv_set("k1", KV_LAST_BALANCE, "NaN")
            .unwrap();
        provider.record_balance_delta(90.0);
        assert_eq!(
            storage.kv_get("k1", KV_LAST_BALANCE).unwrap(),
            Some("90.000000".to_string())
        );
        assert!(storage.load_heatmap("k1", 200).unwrap().is_empty());
    }

    #[test]
    fn parse_finite_rejects_non_finite() {
        assert_eq!(parse_finite("1.5"), Some(1.5));
        assert!(parse_finite("NaN").is_none());
        assert!(parse_finite("inf").is_none());
        assert!(parse_finite("x").is_none());
    }
}
