//! TokenRouter 配置类型（`Settings.router`）与本地 token 生成。
//!
//! 路由器对上游零假设：候选自带 `base_url`（官方端点或中转站皆可），
//! 凭据仍按账户从 keyring 取（须为 BearerKey）。本模块只有类型与纯函数。

use crate::settings::Settings;
use serde::{Deserialize, Serialize};

/// 路由面协议。同协议转发：一条路由链上的所有候选必须是同一协议的上游。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterProtocol {
    Anthropic,
    OpenAi,
}

impl RouterProtocol {
    /// 该协议的标准代理路径（客户端请求与上游转发的路径一致）。
    pub fn proxy_path(self) -> &'static str {
        match self {
            RouterProtocol::Anthropic => "/v1/messages",
            RouterProtocol::OpenAi => "/v1/chat/completions",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RouterProtocol::Anthropic => "Anthropic",
            RouterProtocol::OpenAi => "OpenAI 兼容",
        }
    }
}

/// 路由链上的一个候选：把请求转发到哪个账户的哪个模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateConfig {
    /// 账户（`AccountMeta::instance_id`）：凭据与配额快照来源。须为 BearerKey。
    pub account: String,
    /// 转发时重写进请求 body 的上游模型名。
    pub model: String,
    /// 上游 API 根地址（不含路径），如 `https://api.anthropic.com`。
    pub base_url: String,
    /// 手填订阅日上限（tokens/自然日）。仅对不上报配额的 provider 有意义
    /// （如 Claude 订阅）；used 取路由器自记账的当日消耗。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_limit_tokens_daily: Option<f64>,
}

/// 一条命名路由：同协议候选的有序链 + 本地鉴权 token。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub name: String,
    pub protocol: RouterProtocol,
    /// 本地鉴权 token（工具以它作 API key）。空 = 保存设置时由 Rust 补发。
    #[serde(default)]
    pub token: String,
    /// 有序候选链：链头是主模型，其后是备选。
    #[serde(default)]
    pub candidates: Vec<CandidateConfig>,
}

/// `Settings.router`：代理服务全局配置 + 路由表。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterSettings {
    /// 总开关（主界面快速开关翻转的就是它）。停用时监听保留、请求 503。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_router_port")]
    pub port: u16,
    /// 主动切换阈值：候选剩余% 低于它时新请求导向下一候选。
    #[serde(default = "default_failover_threshold")]
    pub failover_threshold_percent: u8,
    /// 配额冷却到期后，恢复到该剩余% 以上才回用（防抖）。
    #[serde(default = "default_failback_threshold")]
    pub failback_threshold_percent: u8,
    /// 配额/凭据错误的默认冷却秒数（尊重上游 Retry-After 时取大者口径见 decision）。
    #[serde(default = "default_error_cooldown_secs")]
    pub error_cooldown_secs: u64,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

fn default_router_port() -> u16 {
    43211
}

fn default_failover_threshold() -> u8 {
    20
}

fn default_failback_threshold() -> u8 {
    50
}

fn default_error_cooldown_secs() -> u64 {
    300
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_router_port(),
            failover_threshold_percent: default_failover_threshold(),
            failback_threshold_percent: default_failback_threshold(),
            error_cooldown_secs: default_error_cooldown_secs(),
            routes: Vec::new(),
        }
    }
}

/// 生成一个本地路由 token：`tr_<routeId>_<32hex>`。
///
/// 熵源与 `generate_hub_token` 相同（getrandom / BCryptGenRandom）；routeId
/// 中的非安全字符折叠为 `_`，保证 token 可作为任意工具的 API key 字面量。
pub fn generate_route_token(route_id: &str) -> String {
    let mut buf = [0u8; 16];
    let hex = if getrandom::fill(&mut buf).is_err() {
        // 取不到系统随机源时退回常量——调用方（保存流程）仍会落盘，可用性
        // 优先于强度；与 hub token 的处理一致。
        "0".repeat(32)
    } else {
        buf.iter().map(|b| format!("{b:02x}")).collect()
    };
    let safe_id: String = route_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("tr_{safe_id}_{hex}")
}

/// 保存设置前补发空 token 的路由 token。返回是否有改动（调用方据此重写盘）。
pub fn ensure_route_tokens(settings: &mut Settings) -> bool {
    let mut changed = false;
    for route in &mut settings.router.routes {
        if route.token.trim().is_empty() {
            route.token = generate_route_token(&route.id);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_has_expected_shape_and_uniqueness() {
        let a = generate_route_token("r1");
        let b = generate_route_token("r1");
        assert!(a.starts_with("tr_r1_"), "shape: {a}");
        assert_eq!(a.len(), "tr_r1_".len() + 32);
        assert_ne!(a, b, "CSPRNG sanity");
        // routeId 里的怪字符折叠为 _
        let weird = generate_route_token("我的 route/1");
        assert!(weird.starts_with("tr____route_1_"), "sanitized: {weird}");
    }

    #[test]
    fn ensure_route_tokens_fills_only_empty_and_is_idempotent() {
        let mut s = Settings::default();
        s.router.routes.push(RouteConfig {
            id: "r1".into(),
            name: "A".into(),
            protocol: RouterProtocol::Anthropic,
            token: String::new(),
            candidates: Vec::new(),
        });
        s.router.routes.push(RouteConfig {
            id: "r2".into(),
            name: "B".into(),
            protocol: RouterProtocol::OpenAi,
            token: "existing".into(),
            candidates: Vec::new(),
        });
        assert!(ensure_route_tokens(&mut s));
        assert!(s.router.routes[0].token.starts_with("tr_r1_"));
        assert_eq!(s.router.routes[1].token, "existing");
        // 第二次保存不应再改任何 token。
        assert!(!ensure_route_tokens(&mut s));
    }

    #[test]
    fn router_settings_round_trips_through_toml() {
        let mut s = Settings::default();
        s.router.enabled = true;
        s.router.routes.push(RouteConfig {
            id: "r1".into(),
            name: "主力".into(),
            protocol: RouterProtocol::Anthropic,
            token: "tr_r1_x".into(),
            candidates: vec![CandidateConfig {
                account: "anthropic-1".into(),
                model: "claude-sonnet-4-5".into(),
                base_url: "https://api.anthropic.com".into(),
                plan_limit_tokens_daily: Some(1_000_000.0),
            }],
        });
        let dumped = toml::to_string_pretty(&s).expect("serialize");
        let back: Settings = toml::from_str(&dumped).expect("deserialize");
        assert_eq!(back.router, s.router);
        // 旧配置（无 [router] 段）拿到默认值而不是报错。
        let legacy: Settings =
            toml::from_str("poll_interval_seconds = 60").expect("legacy config");
        assert_eq!(legacy.router, RouterSettings::default());
        assert_eq!(legacy.router.port, 43211);
    }

    #[test]
    fn proxy_paths_match_protocol() {
        assert_eq!(RouterProtocol::Anthropic.proxy_path(), "/v1/messages");
        assert_eq!(RouterProtocol::OpenAi.proxy_path(), "/v1/chat/completions");
    }
}
