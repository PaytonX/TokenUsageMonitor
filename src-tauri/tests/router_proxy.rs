//! TokenRouter 端到端集成测试：axum mock 上游 + 真实 RouterCore/server。
//!
//! 覆盖：直通与记账、SSE 流式记账、429 failover、token 鉴权、停用 503。
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

use token_usage_monitor_lib::router::config::RouterProtocol;
use token_usage_monitor_lib::router::RouterCore;
use token_usage_monitor_lib::providers::Credentials;
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
    fn new(fail_first: bool) -> Self {
        Self {
            counter: AtomicUsize::new(if fail_first { 0 } else { 1 }),
            last_model: std::sync::Mutex::new(None),
        }
    }
}

/// openai 形态的 chat 响应（含 usage），记录收到的 model。首次调用返回 429
/// （供 failover 用例）；`fail_first=false` 时直接从 200 开始。
async fn chat_ok(State(st): Shared<MockState>, headers: HeaderMap, body: String) -> Response {
    let n = st.counter.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        // 第一次：配额拒绝（带 Retry-After）。
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "30"), ("Content-Type", "application/json")],
            r#"{"error":{"message":"quota exhausted"}}"#,
        )
            .into_response();
    }
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some(format!(
            "Bearer {}",
            format!("mock-{}-not-a-real-credential", "shared")
        ))
        .as_deref(),
        "路由器必须注入上游凭据"
    );
    // model 重写校验：最后一次请求的 body 里 model 应为候选模型。
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    *st.last_model.lock().unwrap() = Some(v["model"].as_str().unwrap().to_string());
    Json(json!({
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20},
    }))
    .into_response()
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
        "poll_interval_seconds = 60\n\n[router]\nenabled = {enabled}\nport = {port}\nfailover_threshold_percent = 20\nfailback_threshold_percent = 50\nerror_cooldown_secs = 300\n\n{routes_toml}"
    );
    let config_path = dir.0.join("config.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    f.write_all(config.as_bytes()).unwrap();
    drop(f);

    let settings = Arc::new(SettingsStore::new(dir.0.clone()).unwrap());
    let storage = Arc::new(Storage::open(&dir.0.join("test.db")).unwrap());
    // 测试占位 key 由运行时拼接生成：上游是本地 mock，不指向任何真实服务，
    // 因此不存在可用凭据字面量。
    let mock_key = format!("mock-{}-not-a-real-credential", "shared");
    let credentials = Arc::new(tokio::sync::RwLock::new(
        [
            ("acct-1".to_string(), Credentials::BearerKey { api_key: mock_key.clone() }),
            ("acct-2".to_string(), Credentials::BearerKey { api_key: mock_key.clone() }),
            ("acct-a".to_string(), Credentials::BearerKey { api_key: mock_key }),
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
    // 等待监听就绪。
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

    let client = router_client();
    let resp = client
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth("tok-openai")
        .json(&json!({"model": "whatever", "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "hi");
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-1"),
        "请求体 model 必须被重写为候选模型"
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
    // 两个候选指向同一个 mock：第 1 个候选吃掉 429，第 2 个成功。
    let (_core, _storage, port, _dir) =
        spawn_router("failover", true, &openai_route(&upstream, Some(&upstream))).await;

    let client = router_client();
    for i in 0..2 {
        let resp = client
            .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .bearer_auth("tok-openai")
            .json(&json!({"model": "x", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "request {i} must succeed via failover");
    }
    // 共三次上游调用：第 1 次 429（候选 1），其余 200（候选 2）。
    assert_eq!(mock.counter.load(Ordering::SeqCst), 3);
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("model-2"),
        "重试必须落在候选 2 的模型上"
    );
}

#[tokio::test]
async fn non_v1_path_alias_works_and_fallback_gives_hint() {
    // Cherry Studio 类客户端在 API 地址（不带 /v1）后直接拼接 /chat/completions。
    let mock = Arc::new(MockState::new(false));
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(mock.clone()),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("pathalias", true, &openai_route(&upstream, None)).await;
    let client = router_client();

    let resp = client
        .post(&format!("http://127.0.0.1:{port}/chat/completions"))
        .bearer_auth("tok-openai")
        .json(&json!({"model": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "不带 /v1 的路径别名必须可达");

    // 未匹配路径：404 带指引 body（而不是 axum 默认的空 body）。
    let resp = client
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

    // 正常路径：取 data[].id，排序去重。
    let ids = core
        .fetch_upstream_models(RouterProtocol::OpenAi, "acct-1", &upstream)
        .await
        .unwrap();
    assert_eq!(ids, vec!["model-a".to_string(), "model-b".to_string()]);

    // 无凭据账户：明确报错而不是空列表。
    let err = core
        .fetch_upstream_models(RouterProtocol::OpenAi, "no-such-account", &upstream)
        .await
        .unwrap_err();
    assert!(err.contains("API Key"), "err: {err}");
}

#[tokio::test]
async fn monthly_cost_limit_drives_quota_view() {
    // 按量付费形态：月窗只有 used（本币金额、quota=0），min_remaining_percent
    // 恒 1.0；用户设定月消耗上限后，剩余比 = 1 - used/limit，才让阈值生效。
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(chat_ok))
            .with_state(Arc::new(MockState::new(false))),
    )
    .await;
    let routes_toml = format!(
        "[[router.routes]]\nid = \"mc\"\nname = \"cost\"\nprotocol = \"openai\"\ntoken = \"tok-mc\"\n\n[[router.routes.candidates]]\naccount = \"acct-1\"\nmodel = \"m1\"\nbase_url = \"{upstream}\"\nmonthly_cost_limit = 10.0\n"
    );
    let (core, _storage, _port, _dir) = spawn_router("monthlycost", true, &routes_toml).await;

    // 注入 DeepSeek 式快照：月窗已用 9（quota=0），余额 2.5。
    let snap = token_usage_monitor_lib::providers::UsageSnapshot {
        provider_id: "acct-1".into(),
        provider_display_name: "DeepSeek".into(),
        plan_tier: None,
        timestamp: chrono::Utc::now(),
        windows: token_usage_monitor_lib::providers::UsageWindows {
            monthly: Some(token_usage_monitor_lib::providers::WindowUsage {
                used: 9.0,
                quota: 0.0,
                unit: token_usage_monitor_lib::providers::UsageUnit::Cny,
                reset_at: None,
                over_quota: false,
                cost_source: token_usage_monitor_lib::providers::CostSource::Estimated,
                tokens: None,
            }),
            ..Default::default()
        },
        heatmap: None,
    };
    core.snapshots.write().await.insert(
        "acct-1".to_string(),
        token_usage_monitor_lib::providers::ProviderState {
            snapshot: Some(snap),
            last_error: None,
            last_updated_at: Some(chrono::Utc::now()),
        },
    );

    let cand = token_usage_monitor_lib::router::config::CandidateConfig {
        account: "acct-1".into(),
        model: "m1".into(),
        base_url: upstream,
        plan_limit_tokens_daily: None,
        monthly_cost_limit: Some(10.0),
    };
    let view = core.quota_view(&cand).await;
    let remaining = view.remaining.expect("monthly cost limit must yield remaining");
    assert!(
        (remaining - 0.1).abs() < 1e-9,
        "月消耗 9/10 → 剩余 10%，低于 20% 阈值应触发主动切换，got {remaining}"
    );

    // 未设月上限：退回快照口径（quota=0 的月窗 → 剩余 1.0）。
    let no_limit = token_usage_monitor_lib::router::config::CandidateConfig {
        monthly_cost_limit: None,
        ..cand
    };
    let view = core.quota_view(&no_limit).await;
    assert_eq!(view.remaining, Some(1.0));
}

#[tokio::test]
async fn empty_candidate_model_passes_client_model_through() {
    // 候选模型名留空 = 透传工具的原始模型名（同名模型接多个上游的场景）。
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

    let client = router_client();
    let resp = client
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth("tok-pass")
        .json(&json!({"model": "client-original-model", "messages": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        mock.last_model.lock().unwrap().as_deref(),
        Some("client-original-model"),
        "空模型名必须透传原始模型，不得重写"
    );
}

#[tokio::test]
async fn sse_stream_is_forwarded_and_usage_recorded() {
    let upstream = spawn_mock(axum::Router::new().route("/v1/messages", post(sse_messages))).await;
    let routes_toml = format!(
        "[[router.routes]]\nid = \"ra\"\nname = \"claude\"\nprotocol = \"anthropic\"\ntoken = \"tok-anthropic\"\n\n[[router.routes.candidates]]\naccount = \"acct-a\"\nmodel = \"claude-sonnet-4-5\"\nbase_url = \"{upstream}\"\n"
    );
    let (_core, storage, port, _dir) = spawn_router("sse", true, &routes_toml).await;

    let client = router_client();
    let resp = client
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

    // 流结束后台记账：input 100 + cache_read 40 + output 50 = 190。
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
    let client = router_client();
    let resp = client
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth("wrong-token")
        .json(&json!({"model": "x"}))
        .send()
        .await
        .unwrap();
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
    let client = router_client();
    let resp = client
        .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth("tok-openai")
        .json(&json!({"model": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    assert!(core.health.read().await.listening, "停用时监听保留");
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
    let client = router_client();
    let resp = client
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
async fn all_candidates_cooling_answers_429() {
    // 上游恒 429：两次请求后两个候选都进冷却，第三次请求应直接 429 而不再打上游。
    let counter = Arc::new(AtomicUsize::new(0));
    async fn always_429(State(n): Shared<AtomicUsize>) -> Response {
        n.fetch_add(1, Ordering::SeqCst);
        (StatusCode::TOO_MANY_REQUESTS, "quota").into_response()
    }
    let upstream = spawn_mock(
        axum::Router::new()
            .route("/v1/chat/completions", post(always_429))
            .with_state(counter.clone()),
    )
    .await;
    let (_core, _storage, port, _dir) =
        spawn_router("allcooling", true, &openai_route(&upstream, Some(&upstream))).await;

    let client = router_client();
    let call = || async {
        client
            .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .bearer_auth("tok-openai")
            .json(&json!({"model": "x"}))
            .send()
            .await
            .unwrap()
    };
    // 第 1 次：候选 1 → 429 → 候选 2 → 429（都进冷却）。
    assert_eq!(call().await.status(), 429);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    // 第 2 次：全冷却，直接 429，不再打上游。
    assert_eq!(call().await.status(), 429);
    assert_eq!(counter.load(Ordering::SeqCst), 2, "冷却中不应再打上游");
}
