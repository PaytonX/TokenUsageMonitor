//! Mock provider used during skeleton development.
//! Returns a deterministic, varied `UsageSnapshot` so the frontend can be
//! built and verified before any real API call is wired up.

use crate::providers::{Credentials, Provider, ProviderError, UsageSnapshot, UsageUnit, UsageWindows, WindowUsage, BalanceInfo};
use async_trait::async_trait;
use chrono::{Duration, Utc};

pub struct MockProvider {
    id: &'static str,
    display_name: &'static str,
    /// Seed used to vary mock numbers per provider instance.
    seed: f64,
}

impl MockProvider {
    pub fn new(id: &'static str, display_name: &'static str, seed: f64) -> Self {
        Self {
            id,
            display_name,
            seed,
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.display_name
    }

    fn auth_kind(&self) -> crate::providers::AuthKind {
        crate::providers::AuthKind::BearerKey
    }

    async fn fetch_usage(&self, _creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        // Pretend network latency so the loading state is visible in dev.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let now = Utc::now();
        let seed = self.seed;

        Ok(UsageSnapshot {
            provider_id: self.id.to_string(),
            provider_display_name: self.display_name.to_string(),
            plan_tier: Some("Mock".to_string()),
            timestamp: now,
            windows: UsageWindows {
                five_hour: Some(WindowUsage {
                    used: 1_800_000.0 * seed,
                    quota: 5_000_000.0,
                    unit: UsageUnit::Tokens,
                    reset_at: Some(now + Duration::hours(2)),
                    over_quota: false,
                }),
                daily: Some(WindowUsage {
                    used: 9_000_000.0 * seed,
                    quota: 20_000_000.0,
                    unit: UsageUnit::Tokens,
                    reset_at: Some(now + Duration::hours(10)),
                    over_quota: false,
                }),
                weekly: Some(WindowUsage {
                    used: 45_000_000.0 * seed,
                    quota: 100_000_000.0,
                    unit: UsageUnit::Tokens,
                    reset_at: Some(now + Duration::days(4)),
                    over_quota: false,
                }),
                monthly: Some(WindowUsage {
                    used: 180_000_000.0 * seed,
                    quota: 600_000_000.0,
                    unit: UsageUnit::Tokens,
                    reset_at: Some(now + Duration::days(18)),
                    over_quota: false,
                }),
                balance: Some(BalanceInfo {
                    total: 92.50 * seed,
                    granted: 10.0,
                    topped_up: 100.0,
                    currency: "CNY".to_string(),
                    is_available: true,
                }),
            },
            // Heatmap is normally optional; leave it to the SQLite snapshot
            // store when real providers are wired in.
            heatmap: None,
        })
    }
}
