//! Xiaomi MiMo provider.
//!
//! MiMo by Xiaomi offers two billing modes, modelled here as two separate
//! provider kinds (two independent presets in the Settings "add account"
//! grid, so the user chooses which one to add):
//!
//!   1. `xiaomi_plan` — Message / Token **Plan** (配额制). A fixed monthly
//!      quota of Credits per tier:
//!        Lite ¥39/m  = 4.1B Credits
//!        Standard ¥99/m = 11B Credits
//!        Pro ¥329/m    = 38B Credits
//!        Max ¥659/m    = 82B Credits
//!      Credited models (mimo-v2.6-pro / -flash) deduct Credits per token.
//!      Quota exhaustion suspends service. Modeled here as a quota'd monthly
//!      window in `UsageUnit::Credits` (mirrors MiniMax's quota windows).
//!
//!   2. `xiaomi_api` — Pay-as-you-go **API** (按量计费). Billed per token at
//!      a unit price with no fixed quota:
//!        mimo-v2.6-pro  ≈ ¥3 / 1M input tokens, ¥6 / 1M output tokens
//!        mimo-v2.6-flash≈ ¥1 / 1M input tokens, ¥2 / 1M output tokens
//!      Modeled here as a `balance` + a `quota:0.0` monthly spend window,
//!      which the frontend automatically renders as "按量付费" (avatar +
//!      aligned amount block) via `isPayAsYouGo`.
//!
//! TODO: MiMo does publish an account/balance & plan-quota API on the MiMo
//! open platform (<https://platform.xiaomimimo.com>). Until a concrete
//! endpoint + response shape is provided, `fetch_usage` returns deterministic
//! sample data so the frontend can be verified end-to-end. Replace the sample
//! construction with real calls once the endpoints are known.
//!
//! Auth: `Authorization: Bearer <API Key>`

use crate::providers::{
    AuthKind, BalanceInfo, Credentials, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use reqwest::Client;
use std::sync::Arc;

/// Xiaomi MiMo monthly plan quota in Credits for each published tier.
/// (Values are illustrative defaults; the real plan endpoint should override.)
const PLAN_QUOTA_CREDITS: f64 = 4_100_000_000.0; // Lite tier (4.1B)

pub struct XiaoMiPlanProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl XiaoMiPlanProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }
}

pub struct XiaoMiApiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl XiaoMiApiProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }
}

#[async_trait]
impl Provider for XiaoMiPlanProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "xiaomi_plan"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, _creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        // Simulate latency so the loading state is visible in dev.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let now = Utc::now();

        // TODO: replace with a real call to the MiMo Token Plan quota endpoint
        // (e.g. GET platform.xiaomimimo.com/#/console/plan-manage quota API) and
        // derive used/quota/reset_at from the response.
        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: Some("Lite".to_string()),
            timestamp: now,
            windows: UsageWindows {
                // Monthly plan quota in aggregate Credits.
                monthly: Some(WindowUsage {
                    used: 720_000_000.0, // 示例占位值（非真实）：官方暂无配额查询 REST 接口
                    quota: PLAN_QUOTA_CREDITS,
                    unit: UsageUnit::Credits,
                    reset_at: Some(now + Duration::days(22)),
                    over_quota: false,
                    cost_source: crate::providers::CostSource::ProviderReported,
                    tokens: None,
                }),
                ..Default::default()
            },
            heatmap: None,
        })
    }
}

#[async_trait]
impl Provider for XiaoMiApiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "xiaomi_api"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, _creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let now = Utc::now();

        // TODO: replace with a real call to the MiMo API billing/usage endpoint
        // (balance + current-month spend). Quota = 0 marks this as pay-as-you-go,
        // so the frontend renders avatar + amount block automatically.
        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: now,
            windows: UsageWindows {
                balance: Some(BalanceInfo {
                    total: 88.50,      // 示例占位值（非真实）：官方暂无余额查询 REST 接口
                    granted: 100.0,
                    topped_up: 0.0,
                    currency: "CNY".to_string(),
                    is_available: true,
                }),
                monthly: Some(WindowUsage {
                    used: 11.50, // 示例占位值（非真实）：本月消费，官方暂无用量查询 REST 接口
                    quota: 0.0,  // no fixed quota -> pay-as-you-go
                    unit: UsageUnit::Cny,
                    reset_at: None,
                    over_quota: false,
                    // 示例占位值，标为估算以免误导为真实消费。
                    cost_source: crate::providers::CostSource::Estimated,
                    tokens: None,
                }),
                ..Default::default()
            },
            heatmap: None,
        })
    }
}