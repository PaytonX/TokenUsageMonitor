//! TokenRouter 请求/响应改写：模型重写、头清洗、用量提取（JSON 与 SSE 两种形态）。
//!
//! 设计取向：对上游零假设。客户端头只透传白名单，凭据由路由器注入；
//! 响应字节原样透传的同时旁路扫描 usage（不缓冲整个流）。

use crate::router::config::RouterProtocol;
use axum::http::{HeaderMap, HeaderValue};
use bytes::Bytes;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Anthropic 官方 API 必需的版本头；客户端未带时补默认值。
const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// 从客户端请求构造上游请求头：白名单透传 + 上游凭据注入。
///
/// 不透传 authorization / x-api-key / host / content-length / accept-encoding
/// / connection 等——工具自己的凭据绝不外泄给上游，长度与压缩由 reqwest 重算。
pub fn build_upstream_headers(
    protocol: RouterProtocol,
    api_key: &str,
    client: &HeaderMap,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in ["content-type", "accept", "user-agent", "anthropic-version", "anthropic-beta"] {
        if let Some(v) = client.get(name) {
            out.insert(name, v.clone());
        }
    }
    if protocol == RouterProtocol::Anthropic && !out.contains_key("anthropic-version") {
        out.insert("anthropic-version", HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION));
    }
    match protocol {
        RouterProtocol::Anthropic => {
            if let Ok(v) = HeaderValue::from_str(api_key) {
                out.insert("x-api-key", v);
            }
        }
        RouterProtocol::OpenAi => {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {api_key}")) {
                out.insert("authorization", v);
            }
        }
    }
    out
}

/// 把请求 body 的 `model` 字段重写为候选模型。body 必须是 JSON 对象。
pub fn rewrite_model_field(body: &[u8], model: &str) -> Result<Vec<u8>, String> {
    let mut v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("请求体不是合法 JSON: {e}"))?;
    let obj = v.as_object_mut().ok_or("请求体必须是 JSON 对象")?;
    obj.insert("model".to_string(), serde_json::Value::String(model.to_string()));
    serde_json::to_vec(&v).map_err(|e| format!("序列化失败: {e}"))
}

fn json_num(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite() && *f >= 0.0)
}

/// 非流式 JSON 响应的 `(input, cache_read, output)` 提取。
/// Anthropic 的 cache_creation（缓存写入）计入 input——订阅配额按全部
/// 处理 token 计费，账本的 cache_read 列只装缓存命中。
pub fn extract_usage_json(
    protocol: RouterProtocol,
    v: &serde_json::Value,
) -> Option<(f64, f64, f64)> {
    let usage = v.get("usage")?;
    match protocol {
        RouterProtocol::Anthropic => {
            let mut input = json_num(usage.get("input_tokens")?)?;
            input += usage
                .get("cache_creation_input_tokens")
                .and_then(json_num)
                .unwrap_or(0.0);
            let cache_read = usage
                .get("cache_read_input_tokens")
                .and_then(json_num)
                .unwrap_or(0.0);
            let output = json_num(usage.get("output_tokens")?)?;
            Some((input, cache_read, output))
        }
        RouterProtocol::OpenAi => {
            let input = json_num(usage.get("prompt_tokens")?)?;
            let cache_read = usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(json_num)
                .unwrap_or(0.0);
            let output = json_num(usage.get("completion_tokens")?)?;
            Some((input, cache_read, output))
        }
    }
}

/// SSE 流的 usage 累计器：跨 chunk 缓冲、按完整 `data:` 行解析。
///
/// 两种协议的流式语义都是「累计值覆盖」而非逐帧相加：
/// - Anthropic：`message_start` 带 input/cache_read，`message_delta` 带累计 output；
/// - OpenAI：终帧带完整 `usage` 对象（未开 stream_options 时可能没有 → 记 0）。
#[derive(Debug)]
pub struct SseUsageScanner {
    buf: Vec<u8>,
    protocol: RouterProtocol,
    input: f64,
    cache_read: f64,
    output: f64,
    finished: bool,
}

impl SseUsageScanner {
    pub fn new(protocol: RouterProtocol) -> Self {
        Self {
            buf: Vec::new(),
            protocol,
            input: 0.0,
            cache_read: 0.0,
            output: 0.0,
            finished: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if self.finished {
            return;
        }
        self.buf.extend_from_slice(chunk);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            self.absorb_line(&line[..line.len() - 1]);
        }
    }

    /// 流结束：吸收无换行结尾的残留行。
    pub fn finish(&mut self) {
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.absorb_line(&line);
        }
        self.finished = true;
    }

    pub fn totals(&self) -> (f64, f64, f64) {
        (self.input, self.cache_read, self.output)
    }

    fn absorb_line(&mut self, line: &[u8]) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            return;
        };
        match self.protocol {
            RouterProtocol::Anthropic => {
                let event_type = v.get("type").and_then(|t| t.as_str());
                if event_type == Some("message_start") {
                    if let Some(u) = v.pointer("/message/usage") {
                        let mut input = u.get("input_tokens").and_then(json_num).unwrap_or(self.input);
                        input += u
                            .get("cache_creation_input_tokens")
                            .and_then(json_num)
                            .unwrap_or(0.0);
                        self.input = input;
                        if let Some(c) = u.get("cache_read_input_tokens").and_then(json_num) {
                            self.cache_read = c;
                        }
                    }
                } else if event_type == Some("message_delta") {
                    if let Some(o) = v.pointer("/usage/output_tokens").and_then(json_num) {
                        self.output = o;
                    }
                }
            }
            RouterProtocol::OpenAi => {
                if let Some(u) = v.get("usage") {
                    if let Some(i) = u.get("prompt_tokens").and_then(json_num) {
                        self.input = i;
                    }
                    if let Some(c) = u.pointer("/prompt_tokens_details/cached_tokens").and_then(json_num) {
                        self.cache_read = c;
                    }
                    if let Some(o) = u.get("completion_tokens").and_then(json_num) {
                        self.output = o;
                    }
                }
            }
        }
    }
}

/// 透传上游字节流，同时喂给扫描器；流正常结束或客户端中途断开时，
/// 以已累计的 usage 触发记账回调（上游已经消耗，断开不等于免费）。
pub struct ScanStream<S> {
    inner: S,
    scanner: SseUsageScanner,
    on_end: Option<Box<dyn FnOnce(f64, f64, f64) + Send>>,
}

impl<S> ScanStream<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    pub fn new(
        inner: S,
        protocol: RouterProtocol,
        on_end: Box<dyn FnOnce(f64, f64, f64) + Send>,
    ) -> Self {
        Self {
            inner,
            scanner: SseUsageScanner::new(protocol),
            on_end: Some(on_end),
        }
    }
}

impl<S> Stream for ScanStream<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    type Item = reqwest::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                this.scanner.feed(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                this.scanner.finish();
                if let Some(cb) = this.on_end.take() {
                    let (i, c, o) = this.scanner.totals();
                    cb(i, c, o);
                }
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

impl<S> Drop for ScanStream<S> {
    fn drop(&mut self) {
        if let Some(cb) = self.on_end.take() {
            self.scanner.finish();
            let (i, c, o) = self.scanner.totals();
            cb(i, c, o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_headers_inject_x_api_key_and_default_version() {
        let mut client = HeaderMap::new();
        client.insert("authorization", HeaderValue::from_static("Bearer sk-tool-secret"));
        client.insert("content-type", HeaderValue::from_static("application/json"));
        client.insert("anthropic-beta", HeaderValue::from_static("prompt-caching-2024-07-31"));
        let out = build_upstream_headers(RouterProtocol::Anthropic, "sk-upstream", &client);
        assert_eq!(out.get("x-api-key").unwrap(), "sk-upstream");
        assert!(out.get("authorization").is_none(), "工具凭据绝不外泄");
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(out.get("anthropic-beta").unwrap(), "prompt-caching-2024-07-31");
        assert_eq!(out.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn client_version_header_is_preserved() {
        let mut client = HeaderMap::new();
        client.insert("anthropic-version", HeaderValue::from_static("2023-01-01"));
        let out = build_upstream_headers(RouterProtocol::Anthropic, "k", &client);
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-01-01");
    }

    #[test]
    fn openai_headers_inject_bearer_only() {
        let out = build_upstream_headers(RouterProtocol::OpenAi, "sk-up", &HeaderMap::new());
        assert_eq!(out.get("authorization").unwrap(), "Bearer sk-up");
        assert!(out.get("x-api-key").is_none());
        assert!(out.get("anthropic-version").is_none());
    }

    #[test]
    fn rewrite_model_replaces_and_adds_field() {
        let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
        let out = rewrite_model_field(body, "MiniMax-M2").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "MiniMax-M2");
        assert_eq!(v["stream"], true);
        assert_eq!(v["messages"][0]["role"], "user");
    }

    #[test]
    fn rewrite_model_rejects_non_json() {
        assert!(rewrite_model_field(b"not json", "m").is_err());
        assert!(rewrite_model_field(br#"[1,2]"#, "m").is_err());
    }

    #[test]
    fn extract_usage_json_anthropic_folds_cache_creation_into_input() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"usage":{"input_tokens":100,"cache_creation_input_tokens":50,"cache_read_input_tokens":200,"output_tokens":30}}"#,
        )
        .unwrap();
        let (i, c, o) = extract_usage_json(RouterProtocol::Anthropic, &v).unwrap();
        assert_eq!((i, c, o), (150.0, 200.0, 30.0));
    }

    #[test]
    fn extract_usage_json_openai_reads_details() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"usage":{"prompt_tokens":80,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":60}}}"#,
        )
        .unwrap();
        let (i, c, o) = extract_usage_json(RouterProtocol::OpenAi, &v).unwrap();
        assert_eq!((i, c, o), (80.0, 60.0, 20.0));
    }

    #[test]
    fn scanner_reassembles_lines_split_across_chunks() {
        let mut sc = SseUsageScanner::new(RouterProtocol::Anthropic);
        let start = br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":100,"cache_read_input_tokens":40}}}

"#;
        // 把整块按任意边界切成两半喂入，验证跨 chunk 缓冲。
        let (a, b) = start.split_at(start.len() / 2);
        sc.feed(a);
        sc.feed(b);
        let delta = br#"event: message_delta
data: {"type":"message_delta","delta":{},"usage":{"output_tokens":77}}

data: [DONE]

"#;
        sc.feed(delta);
        sc.finish();
        assert_eq!(sc.totals(), (100.0, 40.0, 77.0));
    }

    #[test]
    fn scanner_handles_openai_usage_frame_and_done() {
        let mut sc = SseUsageScanner::new(RouterProtocol::OpenAi);
        sc.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n");
        sc.feed(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":5}}}\n\n");
        sc.feed(b"data: [DONE]\n\n");
        sc.finish();
        assert_eq!(sc.totals(), (9.0, 5.0, 3.0));
    }

    #[test]
    fn scanner_finish_flushes_trailing_line_without_newline() {
        let mut sc = SseUsageScanner::new(RouterProtocol::OpenAi);
        sc.feed(b"data: {\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}");
        sc.finish();
        assert_eq!(sc.totals(), (4.0, 0.0, 1.0));
    }

    #[test]
    fn scanner_ignores_garbage_and_content_lines() {
        let mut sc = SseUsageScanner::new(RouterProtocol::Anthropic);
        sc.feed(b": keep-alive\n");
        sc.feed(b"data: not-json\n");
        sc.feed(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n");
        sc.finish();
        assert_eq!(sc.totals(), (0.0, 0.0, 0.0));
    }

    #[test]
    fn scanner_feed_after_finish_is_noop() {
        let mut sc = SseUsageScanner::new(RouterProtocol::Anthropic);
        sc.finish();
        sc.feed(b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n");
        assert_eq!(sc.totals(), (0.0, 0.0, 0.0));
    }
}
