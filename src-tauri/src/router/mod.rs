//! TokenRouter：本地反向代理，在 provider 账户间自动路由切换。
//!
//! spec: docs/superpowers/specs/2026-10-04-token-router-design.md
//! v2 口径（2026-10-04 原型确认）：**单路由** + 四层判定（主动预警 / 硬墙 /
//! 被动观测 / 探视切回），详见 decision.rs 顶部注释。
//!
//! 模块切分：
//! - [`config`]：`Settings.router` 配置类型与本地 token 生成
//! - [`decision`]：四层判定的纯函数状态机
//! - [`forward`]：请求改写（模型重写 / 头清洗）与响应用量旁路扫描
//! - [`server`]：axum 服务与转发主循环
//!
//! 运行态挂在 [`RouterCore`]（`AppState.router`）。只有端口变化才重启服务
//! （[`RouterCore::restart`]）；enabled 与路由配置每请求现读，保存设置无需
//! ping 路由器。服务无论开关常驻绑定：停用时对请求回 503 + 原因，开关
//! 翻转零延迟，且工具侧报错清晰。

pub mod config;
pub mod decision;
pub mod forward;
pub mod server;

use crate::providers::{Credentials, SharedProviderState};
use crate::settings::SettingsStore;
use crate::storage::Storage;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex, RwLock};

/// 服务健康（主界面快速开关的状态点数据源）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RouterHealth {
    pub listening: bool,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_error: Option<String>,
}

/// 路由切换事件负载（`router-switched`）。
#[derive(Debug, Clone, Serialize)]
pub struct RouterSwitchEvent {
    pub route_id: String,
    pub route_name: String,
    /// 原候选模型；None = 路由首次承接请求。
    pub from: Option<String>,
    pub to: String,
    /// failover | failback | initial
    pub reason: String,
}

/// 路由通道探测结果（`test_routing_channel`）。
#[derive(Debug, Clone, Serialize)]
pub struct RoutingProbeResult {
    /// 2xx = 推理端点可达且 Key 有效。
    pub ok: bool,
    /// 上游 HTTP 状态码；网络错误时为 None。
    pub status: Option<u16>,
    /// 实际探测的完整 URL（诊断用）。
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// /models 可用时返回的模型数量。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<usize>,
}

/// 路由器共享核心：IPC（`get_router_status`）、server 任务、设置保存流程
/// 三方读写的全部运行态。
pub struct RouterCore {
    pub settings: Arc<SettingsStore>,
    /// 账户凭据缓存（keyring 镜像）。候选凭据须为 BearerKey 才可路由。
    pub credentials: Arc<RwLock<HashMap<String, Credentials>>>,
    /// scheduler 维护的每账户最新快照（主动预警的配额来源）。
    pub snapshots: SharedProviderState,
    pub storage: Arc<Storage>,
    /// 上游转发专用 client：无总超时（SSE 分钟级），尊重用户出站代理设置。
    /// 代理设置变化时由 save_settings 整体换新（与 AppState.http 同套路）。
    pub http: RwLock<Client>,
    /// 每路由决策运行态，键 = `RouteConfig.id`（v2 单路由：只有第一条）。
    pub routes: RwLock<HashMap<String, decision::RouteState>>,
    /// 服务健康。
    pub health: RwLock<RouterHealth>,
    /// 当前 server 任务的停机信号；由 [`serve`] 写入、[`RouterCore::restart`] 消费。
    shutdown: RwLock<Option<watch::Sender<bool>>>,
    /// 当前 server 任务的 join 句柄；重启时限时等待旧任务让出端口。
    server_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// 事件与系统通知出口；测试注入 None。
    pub app: Option<tauri::AppHandle>,
}

/// 路由记账用的账本来源键（D9 口径：`router:<instance_id>`，无其他写入方，
/// 与 REPLACE 回放型来源天然隔离）。
pub fn ledger_source(account: &str) -> String {
    format!("router:{account}")
}

/// 路由自记账在 `usage_daily` 里的 `kind`。
///
/// 独立于 `'provider'`：后者是**服务端日账**的 kind，前端的 provider 日账分支
/// 会把该 kind 当官方账本做「替换本机归因」。路由自记账是本机转发侧的事实，
/// 混进去会被当成官方值——而它的量通常只是本机用量的子集，替换会直接吃掉
/// 本机归因。独立 kind 让两类数据在账本层面就是可区分的。
pub const ROUTER_LEDGER_KIND: &str = "router";

/// 从凭据里取**可用于代理转发**的上游推理 API Key。
///
/// - `BearerKey`：直接取（OpenAI / Anthropic 兼容类账户）。
/// - `AccessKeySecret`：取可选的 `api_key`（火山 AgentPlan 等——AK/SK 只用于
///   查询用量的 HMAC 签名，代理转发必须用真正的推理 Key；未填返回 None，
///   该账户可监控但不可路由）。
/// - `LocalToken`：本地登录态不做转发，返回 None。
pub fn routing_api_key_of(creds: Option<&Credentials>) -> Option<String> {
    let key = match creds? {
        Credentials::BearerKey { api_key } => api_key.clone(),
        Credentials::AccessKeySecret { api_key, .. } => api_key.clone()?,
        Credentials::LocalToken { .. } => return None,
    };
    (!key.trim().is_empty() && key != "mock").then_some(key)
}

/// 约束的周期序：用于「最小（最短周期）窗口」选择。
pub mod period {
    pub const FIVE_HOUR: u8 = 0;
    pub const DAILY: u8 = 1;
    pub const WEEKLY: u8 = 2;
    pub const MONTHLY: u8 = 3;
}

impl RouterCore {
    pub fn new(
        settings: Arc<SettingsStore>,
        credentials: Arc<RwLock<HashMap<String, Credentials>>>,
        snapshots: SharedProviderState,
        storage: Arc<Storage>,
        proxy_url: Option<&str>,
        app: Option<tauri::AppHandle>,
    ) -> Self {
        let http = build_router_http_client(proxy_url).unwrap_or_else(|_| {
            Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("building router fallback client")
        });
        Self {
            settings,
            credentials,
            snapshots,
            storage,
            http: RwLock::new(http),
            routes: RwLock::new(HashMap::new()),
            health: RwLock::new(RouterHealth::default()),
            shutdown: RwLock::new(None),
            server_task: Mutex::new(None),
            app,
        }
    }

    /// 代理设置变化时换新上游 client（save_settings 的 proxy 分支调用）。
    pub async fn set_http_client(&self, client: Client) {
        *self.http.write().await = client;
    }

    /// 候选的配额判定（① 最小窗口 + ② 硬墙）。约束集：
    /// - provider 在报窗口（five_hour/daily/weekly/monthly，有的才算）；
    /// - 手填日限（tokens，视为「日窗口」，used = 路由自记账当日总量）；
    /// - 手填月限（金额，视为「月窗口」，used = provider 快照月窗已用，
    ///   仅余额差分类账户有数据；无数据则该约束不参与）。
    pub async fn quota_verdict(&self, cand: &config::CandidateConfig) -> decision::QuotaVerdict {
        let mut constraints: Vec<decision::Constraint> = Vec::new();
        let snaps = self.snapshots.read().await;
        let snapshot = snaps.get(&cand.account).and_then(|s| s.snapshot.as_ref());
        if let Some(snapshot) = snapshot {
            let windows = [
                (period::FIVE_HOUR, snapshot.windows.five_hour.as_ref()),
                (period::DAILY, snapshot.windows.daily.as_ref()),
                (period::WEEKLY, snapshot.windows.weekly.as_ref()),
                (period::MONTHLY, snapshot.windows.monthly.as_ref()),
            ];
            for (rank, w) in windows {
                let Some(w) = w else { continue };
                let remaining = if w.quota > 0.0 {
                    Some(1.0 - w.percent())
                } else {
                    None // used-only 窗口（余额差分/订阅口径），无配额刻度
                };
                let full = w.over_quota || (w.quota > 0.0 && w.used >= w.quota);
                constraints.push(decision::Constraint {
                    period_rank: rank,
                    remaining_percent: remaining,
                    full,
                });
            }
        }
        drop(snaps);

        // 手填日限（tokens）：used = 路由自记账的当日消耗。
        if let Some(limit) = cand.plan_limit_tokens_daily.filter(|l| *l > 0.0) {
            let used = self
                .storage
                .sum_usage_daily_today(&ledger_source(&cand.account))
                .unwrap_or(0.0);
            constraints.push(decision::Constraint {
                period_rank: period::DAILY,
                remaining_percent: Some((1.0 - used / limit).clamp(0.0, 1.0)),
                full: used >= limit,
            });
        }

        // 手填月限（金额）：used = provider 快照月窗已用（余额差分统计）。
        if let Some(limit) = cand.monthly_cost_limit.filter(|l| *l > 0.0) {
            let snaps = self.snapshots.read().await;
            let monthly_used = snaps
                .get(&cand.account)
                .and_then(|s| s.snapshot.as_ref())
                .and_then(|s| s.windows.monthly.as_ref())
                .map(|w| w.used)
                .unwrap_or(0.0);
            drop(snaps);
            if monthly_used > 0.0 {
                constraints.push(decision::Constraint {
                    period_rank: period::MONTHLY,
                    remaining_percent: Some((1.0 - monthly_used / limit).clamp(0.0, 1.0)),
                    full: monthly_used >= limit,
                });
            }
        }

        decision::assess(&constraints)
    }

    /// 该账户快照里最近的未来窗口重置时间（探视放弃后等待的锚点）。
    pub async fn next_reset_at(&self, account: &str) -> Option<DateTime<Utc>> {
        let snaps = self.snapshots.read().await;
        let now = Utc::now();
        snaps
            .get(account)
            .and_then(|s| s.snapshot.as_ref())
            .map(|s| {
                [
                    s.windows.five_hour.as_ref(),
                    s.windows.daily.as_ref(),
                    s.windows.weekly.as_ref(),
                    s.windows.monthly.as_ref(),
                ]
                .into_iter()
                .flatten()
                .filter_map(|w| w.reset_at)
                .filter(|t| *t > now)
                .min()
            })
            .unwrap_or(None)
    }

    /// （重）启动本地代理服务。旧任务先收到停机信号并限时等待退出，
    /// 让出端口后再绑新——端口变更路径依赖这一顺序。
    pub async fn restart(self: &Arc<Self>) {
        if let Some(tx) = self.shutdown.write().await.take() {
            let _ = tx.send(true);
        }
        if let Some(task) = self.server_task.lock().await.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), task).await;
        }
        let core = self.clone();
        let task = tauri::async_runtime::spawn(async move {
            Self::serve(core).await;
        });
        *self.server_task.lock().await = Some(task);
    }

    /// 后台拉起服务的便捷入口（启动装配与 IPC 都用）。
    pub fn spawn(self: &Arc<Self>) {
        let core = self.clone();
        tauri::async_runtime::spawn(async move {
            core.restart().await;
        });
    }

    async fn serve(core: Arc<Self>) {
        let port = core.settings.read_blocking().router.port;
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(target: "tum.router", "bind 127.0.0.1:{port} failed: {e}");
                *core.health.write().await = RouterHealth {
                    listening: false,
                    port,
                    bind_error: Some(e.to_string()),
                };
                return;
            }
        };
        *core.health.write().await = RouterHealth {
            listening: true,
            port,
            bind_error: None,
        };
        let (tx, mut rx) = watch::channel(false);
        *core.shutdown.write().await = Some(tx);
        let app = server::build_router(core.clone());
        tracing::info!(target: "tum.router", "TokenRouter listening on 127.0.0.1:{port}");
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.changed().await;
            })
            .await;
        if let Err(e) = served {
            tracing::warn!(target: "tum.router", "server error: {e}");
        }
        // 只清掉自己写下的状态：重启竞态里新任务可能已经写入新端口。
        let mut health = core.health.write().await;
        if health.port == port {
            health.listening = false;
        }
    }
}

/// 路由器上游 client：连接 10s / 读空闲 300s，无总超时（SSE 长流）。
/// 出站代理设置与主监控共用口径（http/socks5/socks5h）。
pub fn build_router_http_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(300));
    if let Some(url) = proxy_url.map(str::trim).filter(|u| !u.is_empty()) {
        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("invalid proxy url {url:?}: {e}"))?;
        builder = builder.no_proxy().proxy(proxy);
    }
    builder
        .build()
        .map_err(|e| format!("building router client failed: {e}"))
}

/// `get_router_status` 的负载（v2 单路由）：健康 + 路由实时状态。
#[derive(Debug, Serialize)]
pub struct RouterStatusPayload {
    pub enabled: bool,
    pub health: RouterHealth,
    /// 未配置路由时为 None。
    pub route: Option<RouteStatus>,
}

#[derive(Debug, Serialize)]
pub struct RouteStatus {
    pub id: String,
    pub name: String,
    pub on: bool,
    pub protocol: config::RouterProtocol,
    /// 当前激活线路（最近一次实际承接请求者）；尚未承接过 = None。
    pub active_index: Option<usize>,
    /// 切回探视状态（主线路资格满足但实测失败、退避等待中）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<ProbeStatus>,
    pub candidates: Vec<CandidateStatus>,
}

#[derive(Debug, Serialize)]
pub struct ProbeStatus {
    pub attempts: u32,
    pub next_probe_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CandidateStatus {
    pub account: String,
    pub model: String,
    pub base_url: String,
    /// ok | low_quota | full | cooldown | unusable
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_reason: Option<decision::CooldownReason>,
    /// 最小（最短周期）窗口剩余 %；无刻度数据 = None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 组装状态负载（IPC 用）。
pub async fn status_payload(core: &Arc<RouterCore>) -> RouterStatusPayload {
    let settings = core.settings.get().await;
    let now = Utc::now();
    let policy = decision::DecisionPolicy::from_settings(&settings.router);
    let routes_state = core.routes.read().await;
    let route_config = settings.router.active_route().cloned();
    let mut route: Option<RouteStatus> = None;
    if let Some(route_cfg) = route_config {
        let rs = routes_state.get(&route_cfg.id);
        let mut candidates = Vec::with_capacity(route_cfg.candidates.len());
        for (i, cand) in route_cfg.candidates.iter().enumerate() {
            let verdict = core.quota_verdict(cand).await;
            let st = rs
                .and_then(|r| r.candidates.get(i))
                .cloned()
                .unwrap_or_default();
            let usable = core.is_usable(cand).await;
            let cooling = st.cooldown_until.is_some_and(|u| u > now);
            let state = if !usable {
                "unusable"
            } else if cooling {
                "cooldown"
            } else if verdict.hard_full {
                "full"
            } else if verdict
                .min_window_remaining
                .is_some_and(|r| r * 100.0 < settings.router.proactive_threshold_percent as f64)
            {
                "low_quota"
            } else {
                "ok"
            };
            candidates.push(CandidateStatus {
                account: cand.account.clone(),
                model: cand.model.clone(),
                base_url: cand.base_url.clone(),
                state: state.to_string(),
                cooldown_until: st.cooldown_until,
                cooldown_reason: st.cooldown_reason,
                remaining_percent: verdict.min_window_remaining.map(|r| (r * 100.0).clamp(0.0, 100.0)),
                last_error: st.last_error,
            });
        }
        route = Some(RouteStatus {
            id: route_cfg.id.clone(),
            name: route_cfg.name.clone(),
            on: route_cfg.on,
            protocol: route_cfg.protocol,
            active_index: rs.and_then(|r| r.last_used_index),
            probe: rs.and_then(|r| r.probe.as_ref()).map(|p| ProbeStatus {
                attempts: p.attempts,
                next_probe_at: p.next_probe_at,
            }),
            candidates,
        });
    }
    drop(routes_state);
    let health = core.health.read().await.clone();
    RouterStatusPayload {
        enabled: settings.router.enabled,
        health,
        route,
    }
}

impl RouterCore {
    /// 候选凭据是否可路由。
    ///
    /// 两种形态可用：`BearerKey`（OpenAI/Anthropic 兼容类账户），以及
    /// `AccessKeySecret` **额外填写了推理 API Key** 的账户（火山 AgentPlan：
    /// AK/SK 只用于查用量的 HMAC 签名，代理转发需要真正的推理 Key）。
    /// 仅填了 AK/SK 而没有 API Key 的账户可正常监控，但不可路由。
    pub async fn is_usable(&self, cand: &config::CandidateConfig) -> bool {
        routing_api_key_of(self.credentials.read().await.get(&cand.account)).is_some()
    }

    /// 取候选的上游推理 API Key（非空才返回）。
    pub async fn api_key_of(&self, account: &str) -> Option<String> {
        routing_api_key_of(self.credentials.read().await.get(account))
    }

    /// 拉取上游的模型列表（`GET {base}/v1/models`），供设置页「模型名」
    /// 下拉选择。Anthropic 与 OpenAI 兼容上游的响应同形
    /// （`{"data":[{"id":…}, …]}`），按 id 升序去重返回。
    pub async fn fetch_upstream_models(
        &self,
        protocol: config::RouterProtocol,
        account: &str,
        base_url: &str,
    ) -> Result<Vec<String>, String> {
        let api_key = self
            .api_key_of(account)
            .await
            .ok_or_else(|| "该账户没有可用的 API Key 凭据（先在「账户与额度」里保存凭据）".to_string())?;
        let url = forward::upstream_url(base_url, "/v1/models");
        let headers = forward::build_upstream_headers(protocol, &api_key, &HeaderMap::default());
        let resp = self
            .http
            .read()
            .await
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("请求上游失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("上游返回 HTTP {}", resp.status()));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("响应不是合法 JSON: {e}"))?;
        let mut ids: Vec<String> = v
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Err("上游没有返回任何模型（data 为空或格式不符）".to_string());
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// 路由通道探测（`test_routing_channel`）：对上游推理端点发一次免费的
    /// `GET /v1/models`，验证「路径可达 + Key 有效」。与监控通道（fetch_usage
    /// 的用量签名接口）完全独立——火山等监控/推理分离的 Provider 上，测试
    /// 连接通过不代表路由可通。
    pub async fn probe_routing_channel(
        &self,
        protocol: config::RouterProtocol,
        base_url: &str,
        api_key: &str,
    ) -> RoutingProbeResult {
        let url = forward::upstream_url(base_url, "/v1/models");
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return RoutingProbeResult {
                ok: false,
                status: None,
                url,
                error: Some("仅支持 http/https 上游地址".to_string()),
                models: None,
            };
        }
        let headers = forward::build_upstream_headers(protocol, api_key, &HeaderMap::default());
        let send = self.http.read().await.get(&url).headers(headers).send().await;
        match send {
            Err(e) => RoutingProbeResult {
                ok: false,
                status: None,
                url,
                error: Some(format!("网络错误: {e}")),
                models: None,
            },
            Ok(r) => {
                let status = r.status().as_u16();
                if r.status().is_success() {
                    let models = r.json::<serde_json::Value>().await.ok().and_then(|v| {
                        v.get("data").and_then(|d| d.as_array()).map(|a| a.len())
                    });
                    RoutingProbeResult {
                        ok: true,
                        status: Some(status),
                        url,
                        error: None,
                        models,
                    }
                } else {
                    RoutingProbeResult {
                        ok: false,
                        status: Some(status),
                        url,
                        error: Some(format!("HTTP {status}")),
                        models: None,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod routing_key_tests {
    use super::routing_api_key_of;
    use crate::providers::Credentials;

    /// 测试占位 key：运行时拼接生成，不指向任何真实服务，因此源码里不存在
    /// 可用凭据字面量。
    fn placeholder_key() -> String {
        format!("test-{}-not-a-real-credential", "key")
    }

    #[test]
    fn bearer_key_accounts_are_routable() {
        let key = placeholder_key();
        let c = Credentials::BearerKey { api_key: key.clone() };
        assert_eq!(routing_api_key_of(Some(&c)), Some(key));
    }

    #[test]
    fn access_key_secret_needs_the_optional_routing_key() {
        // 火山 AgentPlan 典型形态：只填了 AK/SK → 可监控但不可路由。
        let no_key = Credentials::AccessKeySecret {
            access_key: "AK".into(),
            secret_key: "SK".into(),
            api_key: None,
        };
        assert_eq!(routing_api_key_of(Some(&no_key)), None);

        // 额外填了推理 API Key → 可路由，用它转发（而非 AK/SK）。
        let with_key = Credentials::AccessKeySecret {
            access_key: "AK".into(),
            secret_key: "SK".into(),
            api_key: Some("ark-inference-key".into()),
        };
        assert_eq!(
            routing_api_key_of(Some(&with_key)),
            Some("ark-inference-key".to_string())
        );
    }

    #[test]
    fn blank_and_mock_keys_are_rejected() {
        for key in ["", "   ", "mock"] {
            let c = Credentials::BearerKey {
                api_key: key.into(),
            };
            assert_eq!(routing_api_key_of(Some(&c)), None, "key {key:?}");
        }
        let blank_ak = Credentials::AccessKeySecret {
            access_key: "AK".into(),
            secret_key: "SK".into(),
            api_key: Some("  ".into()),
        };
        assert_eq!(routing_api_key_of(Some(&blank_ak)), None);
    }

    #[test]
    fn missing_credentials_and_local_token_are_not_routable() {
        assert_eq!(routing_api_key_of(None), None);
        let local = Credentials::LocalToken {
            token: "local-login".into(),
        };
        assert_eq!(routing_api_key_of(Some(&local)), None);
    }

    #[test]
    fn access_key_secret_without_api_key_still_deserializes_from_old_configs() {
        // 旧 keyring 条目没有 api_key 字段 → serde 默认 None，不得解析失败。
        let raw = r#"{"kind":"access_key_secret","access_key":"AK","secret_key":"SK"}"#;
        let c: Credentials = serde_json::from_str(raw).expect("legacy creds must parse");
        match &c {
            Credentials::AccessKeySecret { api_key, .. } => assert!(api_key.is_none()),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
