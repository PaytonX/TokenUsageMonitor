//! TokenRouter：本地反向代理，在 provider 账户间自动路由切换。
//!
//! spec: docs/superpowers/specs/2026-10-04-token-router-design.md
//!
//! 模块切分：
//! - [`config`]：`Settings.router` 配置类型与本地 token 生成
//! - [`decision`]：路由决策纯函数状态机（链序选择 / 冷却 / fail-back）
//! - [`forward`]：请求改写（模型重写 / 头清洗）与响应用量旁路扫描
//! - [`server`]：axum 服务与转发主循环
//!
//! 运行态挂在 [`RouterCore`]（`AppState.router`）。只有端口变化才重启服务
//! （[`RouterCore::restart`]）；enabled 与路由表每请求现读，保存设置无需
//! ping 路由器。服务无论开关常驻绑定：停用时对请求回 503 + 原因，开关
//! 翻转零延迟，且工具侧报错清晰。

pub mod config;
pub mod decision;
pub mod forward;
pub mod server;

use crate::providers::{Credentials, SharedProviderState};
use crate::settings::SettingsStore;
use crate::storage::Storage;
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

/// 路由器共享核心：IPC（`get_router_status`）、server 任务、设置保存流程
/// 三方读写的全部运行态。
pub struct RouterCore {
    pub settings: Arc<SettingsStore>,
    /// 账户凭据缓存（keyring 镜像）。候选凭据须为 BearerKey 才可路由。
    pub credentials: Arc<RwLock<HashMap<String, Credentials>>>,
    /// scheduler 维护的每账户最新快照（主动切换的配额来源）。
    pub snapshots: SharedProviderState,
    pub storage: Arc<Storage>,
    /// 上游转发专用 client：无总超时（SSE 分钟级），尊重用户出站代理设置。
    /// 代理设置变化时由 save_settings 整体换新（与 AppState.http 同套路）。
    pub http: RwLock<Client>,
    /// 每路由决策运行态，键 = `RouteConfig.id`。
    pub routes: RwLock<HashMap<String, decision::RouteState>>,
    /// 服务健康。
    pub health: RwLock<RouterHealth>,
    /// 当前 server 任务的停机信号；由 [`serve`] 写入、[`restart`] 消费。
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

    /// 候选的配额视图：min(provider 快照最紧窗口, 手填日限剩余比)。
    /// 快照缺失/错误 → None 分量；手填日限的 used = 路由自记账当日总量。
    pub async fn quota_view(&self, cand: &config::CandidateConfig) -> decision::QuotaView {
        let mut view = decision::QuotaView::default();
        let snaps = self.snapshots.read().await;
        if let Some(snapshot) = snaps.get(&cand.account).and_then(|s| s.snapshot.as_ref()) {
            view = view.combine(decision::QuotaView {
                remaining: Some(snapshot.min_remaining_percent()),
            });
        }
        drop(snaps);
        if let Some(limit) = cand.plan_limit_tokens_daily.filter(|l| *l > 0.0) {
            let used = self
                .storage
                .sum_usage_daily_today(&ledger_source(&cand.account))
                .unwrap_or(0.0);
            view = view.combine(decision::QuotaView {
                remaining: Some((1.0 - used / limit).clamp(0.0, 1.0)),
            });
        }
        view
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

/// `get_router_status` 的负载：健康 + 每路由/每候选实时状态。
#[derive(Debug, Serialize)]
pub struct RouterStatusPayload {
    pub enabled: bool,
    pub health: RouterHealth,
    pub routes: Vec<RouteStatus>,
}

#[derive(Debug, Serialize)]
pub struct RouteStatus {
    pub id: String,
    pub name: String,
    pub protocol: config::RouterProtocol,
    /// 当前激活候选（最近一次实际承接请求者）；尚未承接过 = None。
    pub active_index: Option<usize>,
    pub candidates: Vec<CandidateStatus>,
}

#[derive(Debug, Serialize)]
pub struct CandidateStatus {
    pub account: String,
    pub model: String,
    pub base_url: String,
    /// ok | cooldown | low_quota | unusable
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_reason: Option<decision::CooldownReason>,
    /// 综合剩余比例（快照 ∩ 手填日限的较小者）；无数据 = None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 组装状态负载（IPC 用）。快照读取走 [`RouterCore::quota_view`]。
pub async fn status_payload(core: &Arc<RouterCore>) -> RouterStatusPayload {
    let settings = core.settings.get().await;
    let now = Utc::now();
    let routes_state = core.routes.read().await;
    let mut routes = Vec::with_capacity(settings.router.routes.len());
    for route in &settings.router.routes {
        let rs = routes_state.get(&route.id);
        let mut candidates = Vec::with_capacity(route.candidates.len());
        for (i, cand) in route.candidates.iter().enumerate() {
            let quota = core.quota_view(cand).await;
            let st = rs
                .and_then(|r| r.candidates.get(i))
                .cloned()
                .unwrap_or_default();
            let usable = core.is_usable(cand).await;
            let state = if !usable {
                "unusable"
            } else if st.cooldown_until.is_some_and(|u| u > now) {
                "cooldown"
            } else if quota
                .remaining
                .is_some_and(|r| r * 100.0 < settings.router.failover_threshold_percent as f64)
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
                remaining_percent: quota.remaining.map(|r| (r * 100.0).clamp(0.0, 100.0)),
                last_error: st.last_error,
            });
        }
        routes.push(RouteStatus {
            id: route.id.clone(),
            name: route.name.clone(),
            protocol: route.protocol,
            active_index: rs.and_then(|r| r.last_used_index),
            candidates,
        });
    }
    let health = core.health.read().await.clone();
    RouterStatusPayload {
        enabled: settings.router.enabled,
        health,
        routes,
    }
}

impl RouterCore {
    /// 候选凭据是否可路由（BearerKey 且非空）。
    pub async fn is_usable(&self, cand: &config::CandidateConfig) -> bool {
        self.credentials
            .read()
            .await
            .get(&cand.account)
            .map(|c| matches!(c, Credentials::BearerKey { api_key } if !api_key.trim().is_empty()))
            .unwrap_or(false)
    }

    /// 取候选的上游 API key（已验证为非空 BearerKey）。
    pub async fn api_key_of(&self, account: &str) -> Option<String> {
        match self.credentials.read().await.get(account) {
            Some(Credentials::BearerKey { api_key }) if !api_key.trim().is_empty() => {
                Some(api_key.clone())
            }
            _ => None,
        }
    }
}
