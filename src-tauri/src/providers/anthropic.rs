//! Anthropic / Claude 账户。
//!
//! Anthropic 未公开订阅配额查询 API，`fetch_usage` 返回「今日经路由消耗」的
//! used-only 日窗（quota=0，与 Kimi 月窗的 used-only 先例同口径）：卡片显示
//! 真实消耗；配额上限由 TokenRouter 候选的手填日限承担——路由决策直读
//! settings（`CandidateConfig.plan_limit_tokens_daily`），本 provider 不参与。
//!
//! Auth: `BearerKey`（sk-ant-… 或中转站 key）。作为路由候选时，请求由
//! TokenRouter 注入 `x-api-key` 转发到候选 `base_url`（官方或中转站），
//! 本模块不做任何网络调用。

use crate::providers::{
    AuthKind, CostSource, Credentials, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;

pub struct AnthropicProvider {
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl AnthropicProvider {
    pub fn new(_http: reqwest::Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            storage,
            instance_id,
            label,
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "anthropic"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, _creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        // 无配额 API 可调：日窗 used = 路由器自记账的当日消耗（quota=0 表示
        // 「无上限刻度」，前端按 used-only 渲染）。尚未经路由时为 0。
        let used = self
            .storage
            .sum_usage_daily_today(&crate::router::ledger_source(&self.instance_id))
            .unwrap_or(0.0);
        Ok(UsageSnapshot {
            provider_id: self.instance_id.clone(),
            provider_display_name: self.label.clone(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                daily: Some(WindowUsage {
                    used,
                    quota: 0.0,
                    unit: UsageUnit::Tokens,
                    reset_at: None,
                    over_quota: false,
                    cost_source: CostSource::Estimated,
                    tokens: None,
                }),
                ..Default::default()
            },
            heatmap: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_storage() -> Arc<Storage> {
        let path = std::env::temp_dir().join(format!(
            "pulse_anthropic_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        Arc::new(Storage::open(&path).unwrap())
    }

    #[tokio::test]
    async fn daily_window_reflects_router_ledger_only() {
        let storage = temp_storage();
        let p = AnthropicProvider::new(reqwest::Client::new(), storage.clone(), "anthropic-1".into(), "Claude 主力".into());

        // 未经路由：日窗为 0。
        let snap = p.fetch_usage(&Credentials::BearerKey { api_key: "k".into() }).await.unwrap();
        let daily = snap.windows.daily.unwrap();
        assert_eq!(daily.used, 0.0);
        assert_eq!(daily.quota, 0.0);
        assert!(matches!(daily.cost_source, CostSource::Estimated));

        // 模拟路由记账：累计 1500 tokens（input 1000 + cache 300 + output 200）。
        // 记**带模型名**——路由现在按实际承载模型分行，used 口径必须跨模型求和。
        storage
            .accumulate_usage_daily(
                crate::router::ROUTER_LEDGER_KIND,
                &crate::router::ledger_source("anthropic-1"),
                chrono::Local::now().date_naive(),
                "claude-sonnet-4-5",
                1000.0,
                300.0,
                200.0,
            )
            .unwrap();
        let snap = p.fetch_usage(&Credentials::BearerKey { api_key: "k".into() }).await.unwrap();
        let daily = snap.windows.daily.unwrap();
        assert_eq!(daily.used, 1500.0);
        assert_eq!(snap.provider_id, "anthropic-1");
    }
}
