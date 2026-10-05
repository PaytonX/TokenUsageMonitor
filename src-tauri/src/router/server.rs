//! axum 服务：鉴权 → 选路 → 改写转发 → 用量记账。
//!
//! 主循环 [`route_request`]：按链序选线路（[`decision::pick_candidate`]），
//! 上游结果分级处理（③ 被动观测）——限流/配额立即冷却并换下一条线路；
//! 连接异常原地重试一次、跨请求累计熔断；参数类 4xx 原样透传；成功则转发
//! 响应（SSE 流式旁路扫描 usage 记账）。④ 切回探视：主线路资格满足但实测
//! 失败时按 ×2 退避安排下一次探针。

use crate::notify;
use crate::router::config::{CandidateConfig, RouteConfig, RouterProtocol};
use crate::router::decision::{
    self, DecisionPolicy, ProbeState, ResolvedCandidate, UpstreamOutcome,
};
use crate::router::forward;
use crate::router::{ledger_source, RouterCore, RouterSwitchEvent};
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

/// 请求体上限：长上下文会话可到几十 MB，axum 默认 2MB 会截断。
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
/// 连接类错误的原地重试退避（毫秒）。
const CONN_RETRY_BACKOFF_MS: u64 = 300;
/// 拿不到 reset_at 时的低频探视间隔（秒）。
const PROBE_FALLBACK_SECS: u64 = 900;

/// 端点描述：同一协议面下不同端点的上游路径与用量形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    AnthropicMessages,
    AnthropicCountTokens,
    OpenAiChat,
    /// OpenAI Responses API（Codex 等）——鉴权/模型字段与 Chat 同族。
    OpenAiResponses,
}

impl Endpoint {
    fn protocol(self) -> RouterProtocol {
        match self {
            Endpoint::AnthropicMessages | Endpoint::AnthropicCountTokens => RouterProtocol::Anthropic,
            Endpoint::OpenAiChat | Endpoint::OpenAiResponses => RouterProtocol::OpenAi,
        }
    }
    /// 转发到上游的规范路径（别名 /chat/completions 也归一到带 /v1 的形式）。
    fn upstream_path(self) -> &'static str {
        match self {
            Endpoint::AnthropicMessages => "/v1/messages",
            Endpoint::AnthropicCountTokens => "/v1/messages/count_tokens",
            Endpoint::OpenAiChat => "/v1/chat/completions",
            Endpoint::OpenAiResponses => "/v1/responses",
        }
    }
    /// 用量形状；None = 不记账（count_tokens / models）。
    fn usage_shape(self) -> Option<forward::UsageShape> {
        match self {
            Endpoint::AnthropicMessages => Some(forward::UsageShape::Anthropic),
            Endpoint::AnthropicCountTokens => None,
            Endpoint::OpenAiChat => Some(forward::UsageShape::OpenAiChat),
            Endpoint::OpenAiResponses => Some(forward::UsageShape::OpenAiResponses),
        }
    }
}

pub fn build_router(core: Arc<RouterCore>) -> axum::Router {
    axum::Router::new()
        .route(
            "/v1/messages",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::AnthropicMessages).await
            }),
        )
        .route(
            "/v1/messages/count_tokens",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::AnthropicCountTokens).await
            }),
        )
        .route(
            "/v1/chat/completions",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::OpenAiChat).await
            }),
        )
        // 兼容不带 /v1 前缀的客户端（Cherry Studio 等在 API 地址后直接拼接
        // /chat/completions 与 /models；此前这些路径落到 axum 默认 404 空
        // body，工具侧只能看到「404 no body」无从排查）。
        .route(
            "/chat/completions",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::OpenAiChat).await
            }),
        )
        // OpenAI Responses API（Codex 等）——OpenAI 兼容路由自动同时服务。
        .route(
            "/v1/responses",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::OpenAiResponses).await
            }),
        )
        .route(
            "/responses",
            axum::routing::post(|state, headers, body| async move {
                proxy_post(state, headers, body, Endpoint::OpenAiResponses).await
            }),
        )
        .route("/v1/models", axum::routing::get(proxy_models))
        .route("/models", axum::routing::get(proxy_models))
        .route("/health", axum::routing::get(health))
        .fallback(not_found)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(core)
}

/// 未匹配路径：返回带可用端点指引的 404 JSON（替代 axum 默认的空 body），
/// 并记日志便于排查工具侧的路径拼接问题。
async fn not_found(uri: axum::http::Uri) -> Response {
    tracing::warn!(target: "tum.router", "unmatched path: {}", uri.path());
    let message = format!(
        "未知路径 {}。可用端点：/v1/chat/completions（或 /chat/completions）、/v1/responses（或 /responses）、/v1/messages、/v1/messages/count_tokens、/v1/models（或 /models）、/health。 \
         若工具的 API 地址不含 /v1，直接拼接即可；含 /v1 则同样成立。",
        uri.path()
    );
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"error": {"message": message, "type": "not_found"}}).to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn proxy_post(
    State(core): State<Arc<RouterCore>>,
    headers: HeaderMap,
    body: Bytes,
    endpoint: Endpoint,
) -> Response {
    route_request(core, headers, body, endpoint).await
}

async fn route_request(
    core: Arc<RouterCore>,
    headers: HeaderMap,
    body: Bytes,
    endpoint: Endpoint,
) -> Response {
    let protocol = endpoint.protocol();
    let accounting = endpoint.usage_shape().is_some();
    // ---- 全局开关 + 鉴权 -----------------------------------------------
    let settings = core.settings.get().await;
    if !settings.router.enabled {
        return protocol_error(
            protocol,
            StatusCode::SERVICE_UNAVAILABLE,
            api_error_kind(protocol),
            "TokenRouter 已停用：在主界面快速开关或 设置 → 路由 中开启。",
        );
    }
    let Some(route) = authorize(&settings, &headers) else {
        return protocol_error(
            protocol,
            StatusCode::UNAUTHORIZED,
            auth_error_kind(protocol),
            "缺少或无效的路由 token（应为设置页路由列表中的 tr_… token）。",
        );
    };
    if !route.on {
        return protocol_error(
            protocol,
            StatusCode::SERVICE_UNAVAILABLE,
            api_error_kind(protocol),
            "该路由链已停用（路由卡上的开关可重新启用）。",
        );
    }
    if route.protocol != protocol {
        return protocol_error(
            protocol,
            StatusCode::BAD_REQUEST,
            invalid_request_kind(protocol),
            "路由协议与所请求的端点不符（Anthropic 路由走 /v1/messages，OpenAI 兼容路由走 /v1/chat/completions 或 /v1/responses）。",
        );
    }
    if route.candidates.is_empty() {
        return protocol_error(
            protocol,
            StatusCode::SERVICE_UNAVAILABLE,
            api_error_kind(protocol),
            "该路由还没有配置任何线路。",
        );
    }

    let policy = DecisionPolicy::from_settings(&settings.router);

    // ---- 线路解析：凭据 + 配额判定（快照读取是异步的，须先于 pick） ----
    let mut resolved: Vec<ResolvedCandidate<'_>> = Vec::with_capacity(route.candidates.len());
    let mut api_keys: Vec<Option<String>> = Vec::with_capacity(route.candidates.len());
    for cand in &route.candidates {
        let key = core.api_key_of(&cand.account).await;
        let quota = core.quota_verdict(cand).await;
        resolved.push(ResolvedCandidate {
            config: cand,
            quota,
            usable: key.is_some(),
        });
        api_keys.push(key);
    }

    // ---- ④ 探视态维护：资格丢失即清退避（资格恢复后重新按链序自然尝试） ----
    {
        let mut states = core.routes.write().await;
        let rs = states.entry(route.id.clone()).or_default();
        rs.align(route.candidates.len());
        let main_ok = resolved.first().map(|c| c.quota.can_probe).unwrap_or(false);
        if rs.probe.is_some() && !main_ok {
            rs.probe = None;
        }
    }

    // ---- 尝试循环 --------------------------------------------------------
    // tried：本请求已失败过的线路（不重复尝试）；conn_retried：连接类原地重试
    // 已用（每个线路一次）。循环上限 = 线路数 × 2（原地重试占一档），有界。
    let mut tried = vec![false; route.candidates.len()];
    let mut conn_retried = vec![false; route.candidates.len()];
    let mut last_message = String::new();

    for _ in 0..route.candidates.len() * 2 {
        // 视图：已试过的线路标记为不可用，让 pick 自然跳过（探视/降级逻辑不变）。
        let view: Vec<ResolvedCandidate<'_>> = resolved
            .iter()
            .enumerate()
            .map(|(i, c)| ResolvedCandidate {
                config: c.config,
                quota: c.quota,
                usable: c.usable && !tried[i],
            })
            .collect();
        let picked = {
            let states = core.routes.read().await;
            let rs = states.get(&route.id);
            decision::pick_candidate(
                &view,
                rs.map(|r| r.candidates.as_slice()).unwrap_or(&[]),
                Some(rs.unwrap_or(&decision::RouteState::default())),
                &policy,
                chrono::Utc::now(),
            )
        };
        let Some(idx) = picked else { break };
        let cand = &route.candidates[idx];
        let api_key = api_keys[idx].clone().unwrap_or_default();

        // 候选未指定模型（空串）= 透传工具的原始模型名；指定了才重写。
        let body_bytes = if cand.model.is_empty() {
            body.to_vec()
        } else {
            match forward::rewrite_model_field(&body, &cand.model) {
                Ok(rewritten) => rewritten,
                Err(e) => {
                    return protocol_error(
                        protocol,
                        StatusCode::BAD_REQUEST,
                        invalid_request_kind(protocol),
                        &e,
                    )
                }
            }
        };
        let url = forward::upstream_url(&cand.base_url, endpoint.upstream_path());
        let upstream_headers = forward::build_upstream_headers(protocol, &api_key, &headers);
        let send = core
            .http
            .read()
            .await
            .post(&url)
            .headers(upstream_headers)
            .body(body_bytes)
            .send()
            .await;

        match send {
            Err(e) => {
                last_message = format!("上游 {} 网络错误: {e}", cand.base_url);
                if !conn_retried[idx] {
                    // 连接类先原地重试一次（短暂退避），网络抖动不杀线路。
                    conn_retried[idx] = true;
                    tokio::time::sleep(Duration::from_millis(CONN_RETRY_BACKOFF_MS)).await;
                    continue;
                }
                tried[idx] = true;
                record_outcome(&core, route, idx, &UpstreamOutcome::ServerError, policy, &last_message)
                    .await;
                continue;
            }
            Ok(resp) => {
                let status = resp.status();
                let mut outcome = decision::classify_status(status.as_u16());
                if let UpstreamOutcome::QuotaError { retry_after_secs: None } = &outcome {
                    outcome = UpstreamOutcome::QuotaError {
                        retry_after_secs: parse_retry_after(resp.headers()),
                    };
                }
                if outcome.retryable() {
                    last_message = format!("上游 {} 返回 HTTP {}", cand.base_url, status.as_u16());
                    tried[idx] = true;
                    record_outcome(&core, route, idx, &outcome, policy, &last_message).await;
                    // ④ 主线路限流：资格满足则安排探视退避（×2），资格丢失则清退避。
                    if idx == 0 {
                        update_probe(&core, route, resolved[idx].quota.can_probe, &policy).await;
                    }
                    continue;
                }
                if outcome == UpstreamOutcome::Success {
                    mark_active(&core, route, idx).await;
                    if idx == 0 {
                        clear_probe(&core, route).await;
                    }
                }
                return forward_response(&core, resp, protocol, cand, endpoint, accounting).await;
            }
        }
    }

    protocol_error(
        protocol,
        StatusCode::TOO_MANY_REQUESTS,
        rate_limit_kind(protocol),
        &if last_message.is_empty() {
            "所有线路暂不可用（冷却中或配额不足）。".to_string()
        } else {
            format!("所有线路暂不可用。{last_message}")
        },
    )
}

/// GET /v1/models：模型列表。无 failover 语义，取第一个凭据可用的候选直连。
async fn proxy_models(State(core): State<Arc<RouterCore>>, headers: HeaderMap) -> Response {
    let settings = core.settings.get().await;
    if !settings.router.enabled {
        return protocol_error(
            RouterProtocol::OpenAi,
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "TokenRouter 已停用。",
        );
    }
    let Some(route) = authorize(&settings, &headers) else {
        return protocol_error(
            RouterProtocol::OpenAi,
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "缺少或无效的路由 token。",
        );
    };
    if route.protocol != RouterProtocol::OpenAi {
        return protocol_error(
            RouterProtocol::OpenAi,
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "该路由不是 OpenAI 兼容协议。",
        );
    }
    for cand in &route.candidates {
        let Some(api_key) = core.api_key_of(&cand.account).await else {
            continue;
        };
        let url = forward::upstream_url(&cand.base_url, "/v1/models");
        let upstream_headers =
            forward::build_upstream_headers(RouterProtocol::OpenAi, &api_key, &headers);
        let send = core
            .http
            .read()
            .await
            .get(&url)
            .headers(upstream_headers)
            .send()
            .await;
        match send {
            Ok(resp) => {
                return forward_response(&core, resp, RouterProtocol::OpenAi, cand, Endpoint::OpenAiChat, false).await;
            }
            Err(_) => continue,
        }
    }
    protocol_error(
        RouterProtocol::OpenAi,
        StatusCode::BAD_GATEWAY,
        "server_error",
        "没有凭据可用的线路。",
    )
}

/// 无鉴权诊断端点：只暴露监听/开关状态，不含任何路由与账户数据。
async fn health(State(core): State<Arc<RouterCore>>) -> Response {
    let h = core.health.read().await.clone();
    let enabled = core.settings.read_blocking().router.enabled;
    let payload = json!({
        "ok": true,
        "listening": h.listening,
        "enabled": enabled,
        "port": h.port,
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ---- 内部工具 --------------------------------------------------------------

/// Bearer token → 路由。空 token 的路由永不匹配（保存流程会补发）。
fn authorize<'a>(
    settings: &'a crate::settings::Settings,
    headers: &HeaderMap,
) -> Option<&'a RouteConfig> {
    let token = bearer_token(headers)?;
    settings
        .router
        .routes
        .iter()
        .find(|r| !r.token.is_empty() && r.token == token)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?;
    value.trim().parse::<u64>().ok()
}

async fn record_outcome(
    core: &Arc<RouterCore>,
    route: &RouteConfig,
    idx: usize,
    outcome: &UpstreamOutcome,
    policy: DecisionPolicy,
    message: &str,
) {
    let mut states = core.routes.write().await;
    let rs = states.entry(route.id.clone()).or_default();
    rs.align(route.candidates.len());
    decision::on_result(
        &mut rs.candidates,
        idx,
        outcome,
        &policy,
        chrono::Utc::now(),
        Some(message),
    );
}

/// 主线路限流后的探视退避（④）：资格满足 → 建立/推进退避（×2）；
/// 达到最大次数后等待窗口重置（reset_at），拿不到则 15 分钟低频探视。
async fn update_probe(
    core: &Arc<RouterCore>,
    route: &RouteConfig,
    eligible: bool,
    policy: &DecisionPolicy,
) {
    let now = chrono::Utc::now();
    let Some(cand) = route.candidates.first() else { return };
    if !eligible {
        // 资格丢失（有墙打满）：等冷却自然结束再判定，探视态清空。
        let mut states = core.routes.write().await;
        if let Some(rs) = states.get_mut(&route.id) {
            rs.probe = None;
        }
        return;
    }
    let reset_at = core.next_reset_at(&cand.account).await;
    let mut states = core.routes.write().await;
    let rs = states.entry(route.id.clone()).or_default();
    rs.align(route.candidates.len());
    let probe = rs.probe.get_or_insert_with(|| ProbeState {
        attempts: 0,
        next_probe_at: now,
    });
    decision::advance_probe(probe, policy, now);
    if probe.attempts >= policy.probe_max_attempts {
        probe.next_probe_at = reset_at.unwrap_or(now + chrono::Duration::seconds(PROBE_FALLBACK_SECS as i64));
    }
    tracing::info!(
        target: "tum.router",
        route = %route.name,
        attempt = probe.attempts,
        "main line probe failed; next probe scheduled"
    );
}

/// 探视成功（主线路真实请求 2xx）→ 清退避，流量自然回主线路。
async fn clear_probe(core: &Arc<RouterCore>, route: &RouteConfig) {
    let mut states = core.routes.write().await;
    if let Some(rs) = states.get_mut(&route.id) {
        rs.probe = None;
    }
}

/// 活跃线路变化 → 事件 + 系统通知（每次变化一条，不随请求刷屏）。
async fn mark_active(core: &Arc<RouterCore>, route: &RouteConfig, idx: usize) {
    let switched = {
        let mut states = core.routes.write().await;
        let rs = states.entry(route.id.clone()).or_default();
        rs.align(route.candidates.len());
        if rs.last_used_index != Some(idx) {
            // 显式捕获旧值：None = 首次承接（initial），Some(old) = 真实切换。
            let previous = rs.last_used_index;
            rs.last_used_index = Some(idx);
            Some(previous)
        } else {
            None
        }
    };
    let Some(previous) = switched else { return };
    let Some(app) = core.app.as_ref() else { return };
    let from = previous
        .and_then(|i| route.candidates.get(i))
        .map(|c| c.model.clone());
    let to = route.candidates[idx].model.clone();
    let reason = match (previous, idx) {
        (None, _) => "initial",
        (_, 0) => "failback",
        _ => "failover",
    };
    let _ = app.emit(
        "router-switched",
        &RouterSwitchEvent {
            route_id: route.id.clone(),
            route_name: route.name.clone(),
            from: from.clone(),
            to: to.clone(),
            reason: reason.to_string(),
        },
    );
    if reason != "initial" {
        let reason_text = if reason == "failover" {
            "主线路配额受限，已自动切换"
        } else {
            "主线路已恢复，切回主线路"
        };
        notify::deliver(
            app,
            "TokenRouter 已切换",
            &format!(
                "{}：{} → {}（{}）",
                route.name,
                from.as_deref().unwrap_or("—"),
                to,
                reason_text
            ),
        );
    }
}

/// 转发上游响应。accounting=true 时旁路提取 usage 记账（SSE 走流式扫描，
/// 普通 JSON 读全量后提取）。
async fn forward_response(
    core: &Arc<RouterCore>,
    upstream: reqwest::Response,
    protocol: RouterProtocol,
    cand: &CandidateConfig,
    endpoint: Endpoint,
    accounting: bool,
) -> Response {
    let shape = endpoint.usage_shape();
    let status = upstream.status();
    let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
    let source = ledger_source(&cand.account);

    if !accounting {
        // count_tokens / models：小响应，读全量透传，不记账。
        return match upstream.bytes().await {
            Err(e) => protocol_error(
                protocol,
                StatusCode::BAD_GATEWAY,
                api_error_kind(protocol),
                &format!("读取上游响应失败: {e}"),
            ),
            Ok(b) => respond(status, content_type, Body::from(b)),
        };
    }

    let is_sse = content_type
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);
    if is_sse {
        let storage = core.storage.clone();
        let stream = forward::ScanStream::new(
            upstream.bytes_stream(),
            shape.expect("SSE only on accounting endpoints"),
            Box::new(move |input, cache_read, output| {
                let _ = storage.accumulate_usage_daily(
                    &source,
                    chrono::Local::now().date_naive(),
                    "",
                    input,
                    cache_read,
                    output,
                );
            }),
        );
        return respond(status, content_type, Body::from_stream(stream));
    }

    match upstream.bytes().await {
        Err(e) => protocol_error(
            protocol,
            StatusCode::BAD_GATEWAY,
            api_error_kind(protocol),
            &format!("读取上游响应失败: {e}"),
        ),
        Ok(b) => {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some((input, cache_read, output)) =
                    forward::extract_usage_json(shape.expect("json accounting endpoint"), &v)
                {
                    let _ = core.storage.accumulate_usage_daily(
                        &source,
                        chrono::Local::now().date_naive(),
                        "",
                        input,
                        cache_read,
                        output,
                    );
                }
            }
            respond(status, content_type, Body::from(b))
        }
    }
}

fn respond(status: StatusCode, content_type: Option<HeaderValue>, body: Body) -> Response {
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            content_type.unwrap_or_else(|| HeaderValue::from_static("application/json")),
        )
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 协议形错误体：Anthropic / OpenAI 各自的 error envelope，工具侧能直接解析。
fn protocol_error(
    protocol: RouterProtocol,
    status: StatusCode,
    kind: &str,
    message: &str,
) -> Response {
    let payload = match protocol {
        RouterProtocol::Anthropic => json!({
            "type": "error",
            "error": {"type": kind, "message": message},
        }),
        RouterProtocol::OpenAi => json!({
            "error": {"message": message, "type": kind, "param": null, "code": null},
        }),
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn auth_error_kind(_protocol: RouterProtocol) -> &'static str {
    "authentication_error"
}

fn invalid_request_kind(_protocol: RouterProtocol) -> &'static str {
    "invalid_request_error"
}

fn rate_limit_kind(_protocol: RouterProtocol) -> &'static str {
    "rate_limit_error"
}

fn api_error_kind(protocol: RouterProtocol) -> &'static str {
    match protocol {
        RouterProtocol::Anthropic => "api_error",
        RouterProtocol::OpenAi => "server_error",
    }
}
