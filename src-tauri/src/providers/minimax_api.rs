//! MiniMax pay-as-you-go API provider (skeleton).
//!
//! NOTE (未实装): MiniMax only publicly exposes the Token Plan quota endpoint
//! (`/v1/token_plan/remains`). No documented REST endpoint is available to
//! query a pay-as-you-go account's balance or metered usage. This provider is
//! therefore a **skeleton**: it returns a placeholder snapshot (0 consumption),
//! so the Settings UI can offer the mode (and grey it out as "未实装") without
//! pretending to fetch real data.
//!
//! Once MiniMax publishes a balance/usage query API, implement `fetch_usage`
//! and flip `limited` to `false` in the preset's `sub_modes`.

use crate::providers::{
    AuthKind, Credentials, Provider, ProviderError, UsageSnapshot, UsageUnit, UsageWindows,
    WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use std::sync::Arc;

pub struct MiniMaxApiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl MiniMaxApiProvider {
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

#[async_trait]
impl Provider for MiniMaxApiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "minimax_api"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, _creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 未实装：官方暂未提供余额/用量查询接口，返回占位（0 消费），避免误导。
        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                monthly: Some(WindowUsage {
                    used: 0.0,
                    quota: 0.0, // pay-as-you-go (no fixed quota)
                    unit: UsageUnit::Tokens,
                    reset_at: None,
                    over_quota: false,
                    // 占位值（未实装），标为估算以免误导为真实用量。
                    cost_source: crate::providers::CostSource::Estimated,
                    tokens: None,
                }),
                balance: None,
                ..Default::default()
            },
            heatmap: None,
        })
    }
}