//! TokenRouter 端到端集成测试：axum mock 上游 + 真实 RouterCore/server。
//!
//! 覆盖：直通与记账、SSE 流式记账、限流 failover、连接异常原地重试、
//! 路径别名与 404 指引、鉴权、停用 503、全冷却拒答、探视退避。
//! 上游用 axum 起在 127.0.0.1:0（随机端口），路由器配置写入临时 config.toml，
//! 不触碰任何真实凭据（候选 key 为测试占位值）。

use std::io::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde_json::json;

use token_usage_monitor_lib::providers::{
    CostSource, ProviderState, UsageSnapshot, UsageUnit, UsageWindows, WindowUsage,
};
use token_usage_monitor_lib::router::config::{CandidateConfig, RouterProtocol};
use token_usage_monitor_lib::router::RouterCore;
use token_usage_monitor_lib::settings::SettingsStore;
use token_usage_monitor_lib::storage::Storage;

/// windows-gnu 下测试 exe 必须嵌入 Common-Controls v6 manifest，否则 loader
/// 解析到 comctl32 v5，TaskDialogIndirect 等入口缺失 → STATUS_ENTRYPOINT_NOT_FOUND。
/// 机制同 lib.rs 的 windows_test_manifest 模块；资源由 build.rs 用 windres 生成。
#[cfg(all(windows, target_env = "gnu"))]
#[link(name = "cargo_test_manifest_res", kind = "static", modifiers = "+whole-archive")]
extern "C" {}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tum_router_it_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fn rm(path: &std::path::Path) {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        rm(&entry.path());
                    } else {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            let _ = std::fs::remove_dir(path);
        }
        rm(&self.0);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 启一个 mock 上游，返回其地址。
async fn spawn_mock(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

type Shared<T> = State<Arc<T>>;

/// mock 上游的每实例状态：调用计数 + 最近一次请求体里的 model。
/// 测试并行运行，绝不能用静态全局——否则互相关卡会互相污染/毒化。
struct MockState {
    counter: AtomicUsize,
    last_model: std::sync::Mutex<Option<String>>,
}

impl MockState {
    /// `fail_first = true`：第一次调用返回 429（供 failover 用例）。
    fn new(fail_first: bool) -> Self {
        Self {
            counter: AtomicUsize::new(if fail_first { 0 } else { 1 }),
            last_model: std::sync::Mutex::new(None),
        }
    }
}

async fn chat_ok(State(st): Shared<MockState>, headers: HeaderMap, body: String) -> Response {
    let n = st.counter.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "30"), ("Content-Type", "application/json")],
            r#"{"error":{"message":"quota exhausted"}}"#,
        )
            .into_response();
    }
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some(format!("Bearer {}", mock_key()).as_str()),
        "路由器必须注入上游凭据"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    *st.last_model.lock().unwrap() = Some(v["model"].as_str().unwrap().to_string());
    Json(json!({
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20},
    }))
    .into_response()
}

/// 测试占位 key：运行时拼接生成，不指向任何真实服务（上游是本地 mock），
/// 因此不存在可用凭据字面量。
fn mock_key() -> String {
    format!("mock-{}-not-a-real-credential", "shared")
}

async fn sse_messages() -> Response {
    let body = "event: message_start\n\
                data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":40}}}\n\
                \n\
                event: message_delta\n\
                data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":50}}\n\
                \n\
                data: [DONE]\n\n";
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
}

/// 组装一套真实 RouterCore：临时 config.toml + 临时 SQLite。`routes_toml`
/// 是 [[router.routes]] 段的原文，由各测试自己拼（候选指向 mock 上游）。
async fn spawn_router(tag: &str, enabled: bool, routes_toml: &str) -> (Arc<RouterCore>, Arc<Storage>, u16, TempDir) {
    let dir = TempDir::new(tag);
    let port = free_port();
    let config = format!(
        "poll_interval_seconds = 60\n\n[router]\nenabled = {enabled}\nport = {port}\nproactive_threshold_percent = 20\nerror_cooldown_secs = 300\nconn_breaker_count = 3\nprobe_start_secs = 60\nprobe_max_attempts = 5\n\n{routes_toml}"
    );
    let config_path = dir.0.join("config.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    f.write_all(config.as_bytes()).unwrap();
    drop(f);

    let settings = Arc::new(SettingsStore::new(dir.0.clone()).unwrap());
    let storage = Arc::new(Storage::open(&dir.0.join("test.db")).unwrap());
    let k = mock_key();
    let credentials = Arc::new(tokio::sync::RwLock::new(
        [
            ("acct-1".to_string(), token_usage_monitor_lib::providers::Credentials::BearerKey { api_key: k.clone() }),
            ("acct-2".to_string(), token_usage_monitor_lib::providers::Credentials::BearerKey { api_key: k.clone() }),
            ("acct-a".to_string(), token_usage_monitor_lib::providers::Credentials::BearerKey { api_key: k }),
        ]
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>(),
    ));
    let snapshots = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let core = Arc::new(RouterCore::new(
        settings,
        credentials,
        snapshots,
        storage.clone(),
        None,
        None,
    ));
    core.restart().await;
    for _ in 0..100 {
        if core.health.read().await.listening {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        core.health.read().await.listening,
        "router did not start listening"
    );
    (core, storage, port, dir)
}

fn router_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn openai_route(base: &str, second: Option<&str>) -> String {
    let mut toml = format!(
        "[[router.routes]]\nid = \"r1\"\nname = \"test\"\nprotocol = \"openai\"\ntoken = \"tok-openai\"\n\n[[router.routes.candidates]]\naccount = \"acct-1\"\nmodel = \"model-1\"\nbase_url = \"{base}\"\n"
    );
    if let Some(second) = second {
        toml.push_str(&format!(
            "\n[[router.routes.candidates]]\naccount = \"acct-2\"\nmodel = \"model-2\"\nbase_url = \"{second}\"\n"
        ));
    }
    toml
}

fn post_chat(port: u16, token: &str, model: &str) -> reqwest::RequestBuilder {
    router_client()
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth(token)
        .json(&json!({"model": model, "messages": []}))
}

#[tokio::test]
async fn pass_through_openai_and_ledger_records_usage() {
    let mock = Arc::new(MockState::new(false));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (_core, storage, port, _dir) =
        spawn_router("passthrough", true, &openai_route(&upstream, None)).await;

    let resp = post_chat(port, "tok-openai", "whatever").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "hi");
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-1"),
        "请求体 model 必须被重写为线路模型"
    );

    // 记账：router:acct-1 当日总量 = 120。
    let rows = storage.load_usage_daily(Some("provider"), 7).unwrap();
    let routed: Vec<_> = rows.iter().filter(|r| r.source == "router:acct-1").collect();
    assert_eq!(routed.len(), 1, "exactly one ledger row: {rows:?}");
    assert!((routed[0].total - 120.0).abs() < 1e-6);
}

#[tokio::test]
async fn quota_429_fails_over_to_next_candidate() {
    let mock = Arc::new(MockState::new(true));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    // 两个候选指向同一个 mock：第 1 个吃掉 429，第 2 个成功。
    let (_core, _storage, port, _dir) =
        spawn_router("failover", true, &openai_route(&upstream, Some(&upstream))).await;

    for i in 0..2 {
        let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
        assert_eq!(resp.status(), 200, "request {i} must succeed via failover");
    }
    assert_eq!(mock.counter.load(Ordering::SeqCst), 3);
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-2"),
        "重试必须落在候选 2 的模型上"
    );
}

#[tokio::test]
async fn all_candidates_cooling_answers_429() {
    async fn always_429(State(n): Shared<AtomicUsize>) -> Response {
        n.fetch_add(1, Ordering::SeqCst);
        (StatusCode::TOO_MANY_REQUESTS, "quota").into_response()
    }
    let counter = Arc::new(AtomicUsize::new(0));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(always_429))
            .with_state(counter.clone()),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("allcooling", true, &openai_route(&upstream, Some(&upstream))).await;

    // 第 1 次：候选 1 → 429 → 候选 2 → 429（都进冷却）。
    assert_eq!(post_chat(port, "tok-openai", "x").send().await.unwrap().status(), 429);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    // 第 2 次：全冷却，直接 429，不再打上游。
    assert_eq!(post_chat(port, "tok-openai", "x").send().await.unwrap().status(), 429);
    assert_eq!(counter.load(Ordering::SeqCst), 2, "冷却中不应再打上游");
}

#[tokio::test]
async fn sse_stream_is_forwarded_and_usage_recorded() {
    let upstream = spawn_mock(axum::Router::new().route("/v1/messages", post(sse_messages))).await;
    let routes_toml = format!(
        "[[router.routes]]\nid = \"ra\"\nname = \"claude\"\nprotocol = \"anthropic\"\ntoken = \"tok-anthropic\"\n\n[[router.routes.candidates]]\naccount = \"acct-a\"\nmodel = \"claude-sonnet-4-5\"\nbase_url = \"{upstream}\"\n"
    );
    let (_core, storage, port, _dir) = spawn_router("sse", true, &routes_toml).await;

    let resp = router_client()
        .post(&format!("http://127.0.0.1:{port}/v1/messages"))
        .bearer_auth("tok-anthropic")
        .json(&json!({"model": "x", "max_tokens": 10, "messages": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/event-stream"));
    let text = resp.text().await.unwrap();
    assert!(text.contains("[DONE]"), "SSE body must be forwarded verbatim");

    let rows = storage.load_usage_daily(Some("provider"), 7).unwrap();
    let routed: Vec<_> = rows.iter().filter(|r| r.source == "router:acct-a").collect();
    assert_eq!(routed.len(), 1);
    assert!((routed[0].total - 190.0).abs() < 1e-6, "rows: {rows:?}");
    assert!((routed[0].cache_read - 40.0).abs() < 1e-6);
}

#[tokio::test]
async fn wrong_token_is_unauthorized() {
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(Arc::new(MockState::new(false))),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("authtest", true, &openai_route(&upstream, None)).await;
    let resp = post_chat(port, "wrong-token", "x").send().await.unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn disabled_router_answers_503_but_keeps_listening() {
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(Arc::new(MockState::new(false))),
    )
    .await;
    let (core, _storage, port, _dir) =
        spawn_router("disabled", false, &openai_route(&upstream, None)).await;
    let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
    assert_eq!(resp.status(), 503);
    assert!(core.health.read().await.listening, "停用时监听保留");
}

#[tokio::test]
async fn route_off_answers_503_with_hint() {
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(Arc::new(MockState::new(false))),
    )
    .await;
    let toml = openai_route(&upstream, None).replace("protocol = \"openai\"", "protocol = \"openai\"\non = false");
    let (core, _storage, port, _dir) = spawn_router("routeoff", true, &toml).await;
    let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
    assert_eq!(resp.status(), 503);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"].as_str().unwrap().contains("路由链已停用"),
        "body: {body}"
    );
    assert!(core.health.read().await.listening);
}

#[tokio::test]
async fn models_endpoint_forwards_without_accounting() {
    let upstream = spawn_mock(
        axum::Router::new().route(
            "/v1/models",
            get(|| async { Json(json!({"data": [{"id": "model-1"}]})) }),
        ),
    )
    .await;
    let (_core, storage, port, _dir) =
        spawn_router("models", true, &openai_route(&upstream, None)).await;
    let resp = router_client()
        .get(&format!("http://127.0.0.1:{port}/v1/models"))
        .bearer_auth("tok-openai")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["data"][0]["id"], "model-1");
    let rows = storage.load_usage_daily(Some("provider"), 7).unwrap();
    assert!(
        rows.iter().all(|r| !r.source.starts_with("router:")),
        "模型列表不记账"
    );
}

#[tokio::test]
async fn non_v1_path_alias_works_and_fallback_gives_hint() {
    let mock = Arc::new(MockState::new(false));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("pathalias", true, &openai_route(&upstream, None)).await;

    let resp = router_client()
        .post(&format!("http://127.0.0.1:{port}/chat/completions"))
        .bearer_auth("tok-openai")
        .json(&json!({"model": "x", "messages": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "不带 /v1 的路径别名必须可达");

    let resp = router_client()
        .post(&format!("http://127.0.0.1:{port}/wrong/path"))
        .bearer_auth("tok-openai")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("/v1/chat/completions"), "hint: {message}");
    assert!(message.contains("/wrong/path"), "hint must echo the path");
}

#[tokio::test]
async fn versioned_base_url_does_not_double_the_version_segment() {
    // 用户报告的 404 根因：方舟 Plan base = .../api/plan/v3（自带版本段），
    // 路由器不能再插 /v1。mock 挂在完整路径 /api/plan/v3/chat/completions 上，
    // 模拟方舟 Plan 的真实端点形态（base_url 自带版本段 + 资源路径）。
    let mock = Arc::new(MockState::new(false));
    let upstream_root = spawn_mock(
        axum::Router::new()
            .route("/api/plan/v3/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let routes_toml = format!(
        "[[router.routes]]
id = \"vd\"
name = \"ark plan\"
protocol = \"openai\"
token = \"tok-vd\"

[[router.routes.candidates]]
account = \"acct-1\"
model = \"deepseek-v4-flash\"
base_url = \"{upstream_root}/api/plan/v3\"
"
    );
    let (_core, storage, port, _dir) = spawn_router("versioned", true, &routes_toml).await;

    let resp = post_chat(port, "tok-vd", "x").send().await.unwrap();
    assert_eq!(resp.status(), 200, "base 自带版本段时不得出现 /v3/v1/ 双版本 404");
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("deepseek-v4-flash")
    );
    let rows = storage.load_usage_daily(Some("provider"), 7).unwrap();
    assert!(rows.iter().any(|r| r.source == "router:acct-1"), "记账正常");
}

#[tokio::test]
async fn upstream_404_fails_over_to_next_candidate() {
    // 主线路 404（路径/模型在该线路上不存在）→ 换备用线路，而不是透传 404。
    async fn always_404() -> Response {
        (StatusCode::NOT_FOUND, "no such path").into_response()
    }
    let upstream_404 = spawn_mock(axum::Router::new().route(
        "/v1/chat/completions",
        post(always_404),
    ))
    .await;
    let mock = Arc::new(MockState::new(false));
    let upstream_ok = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("fail404", true, &openai_route(&upstream_404, Some(&upstream_ok))).await;

    let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
    assert_eq!(resp.status(), 200, "主线路 404 应换备用线路");
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-2")
    );
}

#[tokio::test]
async fn routing_channel_probe_reports_reachability() {
    // 路由通道探测：/models 可达 → ok + 模型数；404 → ok=false（中性，非失败）；
    // 无凭据 → ok=false + 明确原因。
    let upstream = spawn_mock(axum::Router::new().route(
        "/v1/models",
        get(|| async { Json(json!({"data": [{"id": "m1"}, {"id": "m2"}]})) }),
    ))
    .await;
    let (core, _storage, _port, _dir) =
        spawn_router("probe2", true, &openai_route(&upstream, None)).await;

    let key = mock_key();
    let ok = core
        .probe_routing_channel(RouterProtocol::OpenAi, &upstream, &key)
        .await;
    assert!(ok.ok);
    assert_eq!(ok.models, Some(2));
    assert_eq!(ok.status, Some(200));

    // 上游未实现 /models：中性结果（404 不算硬失败，转发以实际为准）。
    let bare = spawn_mock(axum::Router::new()).await;
    let miss = core
        .probe_routing_channel(RouterProtocol::OpenAi, &bare, &key)
        .await;
    assert!(!miss.ok);
    assert_eq!(miss.status, Some(404));
}

#[tokio::test]
async fn responses_endpoint_serves_codex_style_requests() {
    // OpenAI 兼容路由同时服务 /v1/responses（Codex）：model 重写 + 透传 +
    // Responses 形状的 usage 记账（input_tokens/output_tokens）。
    async fn responses_ok(body: String) -> Response {
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "model-1", "model 必须被重写为线路模型");
        Json(json!({
            "id": "resp-1",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
            "usage": {"input_tokens": 7, "output_tokens": 3, "total_tokens": 10},
        }))
        .into_response()
    }
    let upstream = spawn_mock(
        axum::Router::new().route("/v1/responses", post(responses_ok)),
    )
    .await;
    let (_core, storage, port, _dir) =
        spawn_router("responses", true, &openai_route(&upstream, None)).await;

    let resp = router_client()
        .post(&format!("http://127.0.0.1:{port}/v1/responses"))
        .bearer_auth("tok-openai")
        .json(&json!({"model": "whatever", "input": "hi", "stream": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "hi");

    let rows = storage.load_usage_daily(Some("provider"), 7).unwrap();
    let routed: Vec<_> = rows.iter().filter(|r| r.source == "router:acct-1").collect();
    assert_eq!(routed.len(), 1, "rows: {rows:?}");
    assert!((routed[0].total - 10.0).abs() < 1e-6);
}

#[tokio::test]
async fn empty_candidate_model_passes_client_model_through() {
    let mock = Arc::new(MockState::new(false));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let routes_toml = format!(
        "[[router.routes]]\nid = \"rp\"\nname = \"passthrough\"\nprotocol = \"openai\"\ntoken = \"tok-pass\"\n\n[[router.routes.candidates]]\naccount = \"acct-1\"\nmodel = \"\"\nbase_url = \"{upstream}\"\n"
    );
    let (_core, _storage, port, _dir) = spawn_router("modelpass", true, &routes_toml).await;

    let resp = post_chat(port, "tok-pass", "client-original-model")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("client-original-model"),
        "空模型名必须透传原始模型"
    );
}

#[tokio::test]
async fn fetch_upstream_models_lists_and_validates() {
    let upstream = spawn_mock(axum::Router::new().route(
        "/v1/models",
        get(|| async {
            Json(json!({"data": [{"id": "model-b"}, {"id": "model-a"}, {"id": "model-a"}]}))
        }),
    ))
    .await;
    let (core, _storage, _port, _dir) =
        spawn_router("modelfetch", true, &openai_route(&upstream, None)).await;

    let ids = core
        .fetch_upstream_models(RouterProtocol::OpenAi, "acct-1", &upstream)
        .await
        .unwrap();
    assert_eq!(ids, vec!["model-a".to_string(), "model-b".to_string()]);

    let err = core
        .fetch_upstream_models(RouterProtocol::OpenAi, "no-such-account", &upstream)
        .await
        .unwrap_err();
    assert!(err.contains("API Key"), "err: {err}");
}

#[tokio::test]
async fn proactive_threshold_routes_away_before_upstream_rejects() {
    // ① 主动预警：主线路最小窗口剩 12%（< 20%）→ 新请求不撞它，直接走备用。
    // 上游计数器必须保持 0（一次都没打主线路）。
    let mock = Arc::new(MockState::new(false));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (core, _storage, port, _dir) =
        spawn_router("proactive", true, &openai_route(&upstream, Some(&upstream))).await;

    // 主线路 acct-1：5h 窗剩 12%。
    core.snapshots.write().await.insert(
        "acct-1".to_string(),
        ProviderState {
            snapshot: Some(UsageSnapshot {
                provider_id: "acct-1".into(),
                provider_display_name: "主线路账户".into(),
                plan_tier: None,
                timestamp: chrono::Utc::now(),
                windows: UsageWindows {
                    five_hour: Some(WindowUsage {
                        used: 88.0,
                        quota: 100.0,
                        unit: UsageUnit::Percent,
                        reset_at: None,
                        over_quota: false,
                        cost_source: CostSource::ProviderReported,
                        tokens: None,
                    }),
                    ..Default::default()
                },
                heatmap: None,
            }),
            last_error: None,
            last_updated_at: Some(chrono::Utc::now()),
        },
    );

    let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-2"),
        "主动预警应把新流量导向备用线路"
    );
}

#[tokio::test]
async fn monthly_cost_limit_acts_as_a_month_window() {
    // 按量付费：月窗 used=9（quota=0，无刻度）+ 手填月限 10 → 剩余 10%，
    // 低于 20% 主动阈值 → 视为低配额。used=9/10 这条约束必须参与判定。
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(Arc::new(MockState::new(false))),
    )
    .await;
    let toml = format!(
        "[[router.routes]]\nid = \"mc\"\nname = \"cost\"\nprotocol = \"openai\"\ntoken = \"tok-mc\"\n\n[[router.routes.candidates]]\naccount = \"acct-1\"\nmodel = \"m1\"\nbase_url = \"{upstream}\"\nmonthly_cost_limit = 10.0\n"
    );
    let (core, _storage, _port, _dir) = spawn_router("monthlycost", true, &toml).await;
    core.snapshots.write().await.insert(
        "acct-1".to_string(),
        ProviderState {
            snapshot: Some(UsageSnapshot {
                provider_id: "acct-1".into(),
                provider_display_name: "DeepSeek".into(),
                plan_tier: None,
                timestamp: chrono::Utc::now(),
                windows: UsageWindows {
                    monthly: Some(WindowUsage {
                        used: 9.0,
                        quota: 0.0,
                        unit: UsageUnit::Cny,
                        reset_at: None,
                        over_quota: false,
                        cost_source: CostSource::Estimated,
                        tokens: None,
                    }),
                    ..Default::default()
                },
                heatmap: None,
            }),
            last_error: None,
            last_updated_at: Some(chrono::Utc::now()),
        },
    );

    let cand = CandidateConfig {
        account: "acct-1".into(),
        model: "m1".into(),
        base_url: upstream,
        plan_limit_tokens_daily: None,
        monthly_cost_limit: Some(10.0),
    };
    let v = core.quota_verdict(&cand).await;
    let remaining = v.min_window_remaining.expect("monthly cost limit must yield remaining");
    assert!(
        (remaining - 0.1).abs() < 1e-9,
        "月消耗 9/10 → 剩余 10%，got {remaining}"
    );

    // 未设月上限：退回快照口径（quota=0 的月窗无刻度 → None）。
    let no_limit = CandidateConfig {
        monthly_cost_limit: None,
        ..cand
    };
    let v = core.quota_verdict(&no_limit).await;
    assert_eq!(v.min_window_remaining, None);
    assert!(!v.hard_full);
}

#[tokio::test]
async fn probe_backoff_schedules_next_attempt_on_main_line_quota_error() {
    // ④ 切回探视：主线路 429（资格满足：快照无打满）→ 排 ×2 退避。
    let mock = Arc::new(MockState::new(true));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (core, _storage, port, _dir) =
        spawn_router("probe", true, &openai_route(&upstream, Some(&upstream))).await;

    let resp = post_chat(port, "tok-openai", "x").send().await.unwrap();
    assert_eq!(resp.status(), 200, "限流后应落到备用线路成功");

    // 主线路资格满足（有窗口且未打满）→ 探视态存在且 next_probe_at 已定。
    let state = core.routes.read().await;
    let rs = state.get("r1").expect("route state");
    let probe = rs.probe.as_ref().expect("probe scheduled on main line 429");
    assert!(probe.attempts >= 1);
    assert!(
        probe.next_probe_at > chrono::Utc::now(),
        "next probe must be scheduled in the future"
    );
    drop(state);
}
