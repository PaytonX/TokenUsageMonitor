//! TokenRouter 配置类型（`Settings.router`）与本地 token 生成。
//!
//! v2 语义（2026-10-04 原型确认）：**单路由**——`routes` 只服务第一条
//! （保留 Vec 仅为旧配置平滑迁移，UI 与判定均只看 `routes.first()`）。
//! 切换逻辑为四层判定：主动预警（最小窗口剩余%，可设 0 关闭）/ 硬墙兜底 /
//! 被动观测（限流立即切、连接异常熔断）/ 探视切回（资格判定 + 真实请求探针
//! + 指数退避 + 等待窗口重置）。

use crate::settings::Settings;
use serde::{Deserialize, Serialize};

/// 路由面协议。同协议转发：路由链上的所有候选必须是同一协议的上游。
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

/// 路由链上的一个候选（v2 语义：称「线路」）。把请求转发到哪个账户的哪个模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateConfig {
    /// 账户（`AccountMeta::instance_id`）：凭据与配额快照来源。须为 BearerKey。
    pub account: String,
    /// 转发时重写进请求 body 的上游模型名；**留空 = 透传工具的原始模型名**
    /// （同名模型接多个上游时不用挨个填）。
    pub model: String,
    /// 上游 API 根地址（不含路径），如 `https://api.anthropic.com`。
    pub base_url: String,
    /// 手填订阅日上限（tokens/自然日），视为「日窗口」参与最小窗口判定；
    /// used 取路由器自记账的当日消耗。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_limit_tokens_daily: Option<f64>,
    /// 按量付费的月消耗上限（**账户计价币种**金额，如 ¥ / $），视为「月窗口」。
    /// used 取 provider 快照月窗已用（余额差分账户本就在统计）；仅对有金额
    /// 统计的账户生效，订阅类账户设置了也无害（无数据源，忽略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_cost_limit: Option<f64>,
}

/// 单路由配置（v2：整个 TokenRouter 只有一条）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub name: String,
    pub protocol: RouterProtocol,
    /// 路由链开关：停用后该链的 token 请求返回明确错误（配置保留）。
    #[serde(default = "default_true")]
    pub on: bool,
    /// 本地鉴权 token（工具以它作 API key）。空 = 保存设置时由 Rust 补发。
    #[serde(default)]
    pub token: String,
    /// 有序候选链：链头是主线路，其后是备用。
    #[serde(default)]
    pub candidates: Vec<CandidateConfig>,
}

fn default_true() -> bool {
    true
}

/// `Settings.router`：代理服务全局配置 + 单路由。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterSettings {
    /// 总开关（主界面快速开关翻转的就是它）。停用时监听保留、请求 503。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_router_port")]
    pub port: u16,
    /// 主动预警阈值：**最小（最短周期）在报窗口**的剩余 % 低于它时，新请求
    /// 换下一条线路。0 = 关闭主动预警（纯被动观测 + 硬墙兜底）。
    #[serde(default = "default_proactive_threshold")]
    pub proactive_threshold_percent: u8,
    /// 限流/配额类错误（429/402）的冷却秒数；上游 Retry-After 优先。
    #[serde(default = "default_error_cooldown_secs")]
    pub error_cooldown_secs: u64,
    /// 连接异常/5xx 的熔断阈值：连续 N 次失败才把线路标记短冷却。
    #[serde(default = "default_conn_breaker")]
    pub conn_breaker_count: u32,
    /// 切回探视的起始间隔（秒）；失败后 ×2 指数退避。
    #[serde(default = "default_probe_start_secs")]
    pub probe_start_secs: u64,
    /// 切回探视最大次数；超过后等待窗口重置（reset_at），拿不到则 15 分钟低频。
    #[serde(default = "default_probe_max_attempts")]
    pub probe_max_attempts: u32,
    /// 路由表。**只有第一条生效**（v2 单路由；旧配置多余的条目被忽略）。
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

fn default_router_port() -> u16 {
    43211
}

fn default_proactive_threshold() -> u8 {
    20
}

fn default_error_cooldown_secs() -> u64 {
    300
}

fn default_conn_breaker() -> u32 {
    3
}

fn default_probe_start_secs() -> u64 {
    60
}

fn default_probe_max_attempts() -> u32 {
    5
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_router_port(),
            proactive_threshold_percent: default_proactive_threshold(),
            error_cooldown_secs: default_error_cooldown_secs(),
            conn_breaker_count: default_conn_breaker(),
            probe_start_secs: default_probe_start_secs(),
            probe_max_attempts: default_probe_max_attempts(),
            routes: Vec::new(),
        }
    }
}

impl RouterSettings {
    /// 生效的单路由（v2：第一条）。None = 用户还没配置路由。
    pub fn active_route(&self) -> Option<&RouteConfig> {
        self.routes.first()
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

    fn route_with_token(token: &str) -> RouteConfig {
        RouteConfig {
            id: "r1".into(),
            name: "主力".into(),
            protocol: RouterProtocol::Anthropic,
            on: true,
            token: token.into(),
            candidates: vec![CandidateConfig {
                account: "anthropic-1".into(),
                model: "claude-sonnet-4-5".into(),
                base_url: "https://api.anthropic.com".into(),
                plan_limit_tokens_daily: Some(1_000_000.0),
                monthly_cost_limit: None,
            }],
        }
    }

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
        s.router.routes.push(route_with_token(""));
        s.router.routes.push(RouteConfig {
            token: "existing".into(),
            ..route_with_token("")
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
        s.router.routes.push(route_with_token("tr_r1_x"));
        let dumped = toml::to_string_pretty(&s).expect("serialize");
        let back: Settings = toml::from_str(&dumped).expect("deserialize");
        assert_eq!(back.router, s.router);
        // 旧配置（无 [router] 段）拿到默认值而不是报错。
        let legacy: Settings =
            toml::from_str("poll_interval_seconds = 60").expect("legacy config");
        assert_eq!(legacy.router, RouterSettings::default());
        assert_eq!(legacy.router.port, 43211);
        // v1 旧字段（failover_threshold_percent 等）被 serde 忽略，不阻塞解析。
        let v1 = toml::from_str::<Settings>(
            "poll_interval_seconds = 60\n[router]\nfailover_threshold_percent = 20\nfailback_threshold_percent = 50\n",
        )
        .expect("v1 config must still parse");
        assert_eq!(v1.router.proactive_threshold_percent, 20);
    }

    #[test]
    fn active_route_takes_first_and_handles_empty() {
        let mut s = Settings::default();
        assert!(s.router.active_route().is_none());
        s.router.routes.push(route_with_token("a"));
        s.router.routes.push(route_with_token("b"));
        assert_eq!(s.router.active_route().unwrap().token, "a");
    }

    #[test]
    fn proxy_paths_match_protocol() {
        assert_eq!(RouterProtocol::Anthropic.proxy_path(), "/v1/messages");
        assert_eq!(RouterProtocol::OpenAi.proxy_path(), "/v1/chat/completions");
    }
}
