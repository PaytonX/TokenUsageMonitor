//! DeepSeek Harness (dsh) local session scanner.
//!
//! dsh stores one zstd-compressed JSONL per session under
//! `~/.dsh/sessions/<project-slug>/<session-dir>/session.v{3,4}.jsonl.zstd`.
//! We read only usage numbers and the line's `time` (epoch millis); conversation
//! text is never touched.
//!
//! ## Two usage carriers, and why `assistant/message` is the primary one
//!
//! dsh has written usage in two places, and they are **not** interchangeable:
//!
//! ```text
//! // legacy layout — usage is a stream chunk on an attempt record
//! {"type":"assistant/attempt","time":1790522454929,
//!  "data":{"stream":[{"type":"chunk",
//!                     "chunk":{"type":"usage",
//!                              "usage":{"inputTokens":N,"outputTokens":M}}}]}}
//!
//! // current layout — usage sits directly on each assistant message
//! {"type":"assistant/message","time":1790577965809,
//!  "data":{"turn":1,"step":1,
//!          "usage":{"inputTokens":18172,"outputTokens":2001,
//!                   "cacheReadTokens":0,"totalTokens":20173},
//!          "message":{...}}}
//! ```
//!
//! Measured on a real machine, the active v4 sessions carry **921** usage
//! records on `assistant/message` and only **24** on `assistant/attempt`. Their
//! timestamps do not overlap: the attempt stream describes a request's
//! settlement, while each `assistant/message` reports the per-step usage of the
//! reply it produced. So we prefer `assistant/message` and only fall back to the
//! attempt stream for sessions that predate it — we must **not** sum both, or
//! sessions carrying both layouts would be counted twice.
//!
//! ## Where the model name comes from
//!
//! Unlike the legacy attempt layout, each `assistant/message` carries its own
//! model on the same line as the usage, under
//! `data.message.source.model` (mirrored at
//! `data.message.source.replayState.response.model`).
//!
//! We read `source.model` — the **requested** model — rather than the sibling
//! `responseModel` (e.g. `deepseek-v4-flash-ga-260731`). The pricing table is
//! keyed by requested model id and matched by prefix, so the server-side
//! versioned alias would miss every tier and leave the row unpriced. On a real
//! machine the two model fields agreed on 924 of 924 usage lines, so reading
//! the requested one loses nothing.
//!
//! ## …and why the billing account is a separate dimension
//!
//! The same `source` object also carries `provider` — the upstream that will
//! actually bill this call (`opencode-go`, `ark-cn`, `local-router`, …). Model
//! and provider are orthogonal: `deepseek-v4.1-flash` sent through OpenCode Go
//! is billed by OpenCode, not DeepSeek. Attributing by model-name prefix alone
//! (the old behaviour) filed that usage under DeepSeek and made the trend page
//! disagree with the user's invoices. We therefore bucket per
//! `(provider, model)` and hand both up to the ledger.
//!
//! Sessions do switch models mid-run (observed: `MiniMax-M3` 675 lines,
//! `deepseek-v4-flash` 249), so the per-model split is a real distinction
//! rather than a cosmetic one. Lines with no model at all still fall into the
//! unclassified bucket so the per-model view still reconciles with the daily
//! total.
//!
//! ## Cache reads are tracked separately
//!
//! `totalTokens` is **not** `inputTokens + outputTokens`: the difference is
//! `cacheReadTokens`. Treating the total as input/output would silently inflate
//! input by the cached portion, and cached tokens are billed at a different
//! rate. We therefore read the three fields separately and keep the cache
//! portion in its own bucket (matching how the Claude Code scanner does it).
//! We also never trust `totalTokens` itself, since dsh writes it even for failed
//! attempts. Zero-valued usage (auth failures, quota errors) is skipped so a
//! broken key doesn't fabricate rows.

use super::{finalize_days, LocalModelUsage, LocalToolReport};
use chrono::{Local, TimeZone, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;

const TOOL_ID: &str = "deepseek-harness";
const TOOL_NAME: &str = "DeepSeek Harness";
const KEEP_DAYS: usize = 90;

/// Guard against a corrupt or hostile file claiming an absurd decompressed
/// size. Observed real sessions decompress to ~1-4 MB; 256 MB leaves several
/// orders of magnitude of headroom while still bounding memory.
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

/// Parse one decompressed JSONL line into
/// `(input, cache_read, output, local_day, model, provider)` when it carries
/// dsh usage.
///
/// Handles both carriers described in the module docs. Only one is ever
/// returned per line, so a line can never contribute twice.
pub(crate) fn parse_line(line: &str) -> Option<(f64, f64, f64, String, String, String)> {
    if !line.contains("usage") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let kind = v.get("type").and_then(|t| t.as_str())?;

    let usage = match kind {
        // Current layout: usage sits next to the message it bills.
        "assistant/message" => v.pointer("/data/usage")?,
        // Legacy layout: usage is the last matching chunk of the attempt stream.
        // Retried attempts stream incremental updates, so the final chunk holds
        // the settled totals rather than a mid-flight partial.
        "assistant/attempt" => {
            let stream = v.pointer("/data/stream")?.as_array()?;
            stream.iter().rev().find_map(|c| {
                let chunk = c.get("chunk")?;
                (chunk.get("type")?.as_str()? == "usage").then(|| chunk.get("usage"))?
            })?
        }
        _ => return None,
    };

    let num = |name: &str| -> f64 { usage.get(name).and_then(|n| n.as_f64()).unwrap_or(0.0) };
    let input = num("inputTokens");
    let cache_read = num("cacheReadTokens");
    let output = num("outputTokens");
    // Failed attempts log a zero usage record; counting it would add empty rows
    // and mask the real problem (e.g. an invalid API key or a quota error).
    if input <= 0.0 && cache_read <= 0.0 && output <= 0.0 {
        return None;
    }

    let date = v
        .get("time")
        .and_then(|t| t.as_u64())
        .and_then(day_of_epoch_millis)
        .unwrap_or_default();

    // The model rides along on the message itself. We prefer the requested model
    // over `responseModel` because the price table is keyed by requested id;
    // `data.model` is kept as a fallback for older builds that stamped it
    // there. Anything unresolved falls into the unclassified bucket.
    let model = v
        .pointer("/data/message/source/model")
        .and_then(|m| m.as_str())
        .or_else(|| v.pointer("/data/model").and_then(|m| m.as_str()))
        .unwrap_or_default()
        .to_string();

    // 计费账户：`source.provider` 是 dsh 这一条消息实际打到的上游
    // （`opencode-go` / `ark-cn` / `local-router`…）。它和模型名是两个维度：
    // `deepseek-v4.1-flash` 经 OpenCode Go 调用时账单记在 OpenCode，只按模型名
    // 归因会把这部分用量算进 DeepSeek——用户实际观测到的错账就是这么来的。
    // 老版本日志没有这个字段，留空串表示"源数据没记"。
    let provider = v
        .pointer("/data/message/source/provider")
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();

    // 走本机路由的会话，模型名是 `auto`：路由在服务端决定实际承载模型，客户端
    // 只看到这个占位名。它不是"未标记模型"（上游确实回了模型名），也不是任何
    // 真实模型，所以统一落成 `__via_router__` 哨兵，由 TokenRouter 自己的记账行
    // 在前端做替换归因。判据看 provider，不看 "auto" 这个字面量。
    let model = if super::is_router_provider(&provider) {
        super::ROUTED_MODEL.to_string()
    } else {
        model
    };

    Some((input, cache_read, output, date, model, provider))
}

fn day_of_epoch_millis(ms: u64) -> Option<String> {
    // Reject anything that cannot round-trip through i64: a huge u64 would wrap
    // to a negative and silently become a *valid but wrong* past date instead
    // of being dropped.
    let ms = i64::try_from(ms).ok()?;
    let dt = Utc.timestamp_millis_opt(ms).single()?;
    Some(dt.with_timezone(&Local).date_naive().to_string())
}

/// Scan dsh sessions under the Windows home and every WSL distro home.
pub fn scan() -> LocalToolReport {
    let now = Utc::now().to_rfc3339();
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_account: BTreeMap<(String, String), BTreeMap<String, (f64, f64, f64)>> =
        BTreeMap::new();
    let mut session_count: u64 = 0;
    let mut project_dirs: HashSet<String> = HashSet::new();

    for sessions in super::wsl::existing_dotdirs(".dsh/sessions") {
        scan_tree(
            &sessions,
            &mut by_day,
            &mut by_account,
            &mut session_count,
            &mut project_dirs,
        );
    }

    let daily = finalize_days(by_day.clone(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();

    // 按 (计费账户, 模型) 分桶，而不是只按模型名：同一模型可能经不同账户发生
    // （订阅 vs 本机路由），并成一行就没法把账单归给正确的 provider。
    let mut models: Vec<LocalModelUsage> = Vec::new();
    for ((provider, model), days) in by_account {
        let series = finalize_days(days, KEEP_DAYS);
        let row_tokens: f64 = series.iter().map(|d| d.total).sum();
        if row_tokens <= 0.0 {
            continue;
        }
        let mut usage = LocalModelUsage {
            model,
            provider,
            total_tokens: row_tokens,
            cost: 0.0,
            currency: String::new(),
            cost_estimated: false,
            daily: series,
        };
        // Cache reads are billed at a discount by `compute_cost`, so they must
        // be passed through separately. Pre-merging them into `input` would
        // charge them at the full input rate — with cache at 80-98% of real
        // traffic that inflated the estimate badly.
        let input: f64 = usage.daily.iter().map(|d| d.input).sum();
        let cache_read: f64 = usage.daily.iter().map(|d| d.cache_read).sum();
        let output: f64 = usage.daily.iter().map(|d| d.output).sum();
        if let Some(cost) = crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
            input,
            cache_read,
            output,
            model_id: Some(usage.model.clone()),
        }) {
            if cost > 0.0 {
                usage.cost = cost;
                usage.currency = "USD".to_string();
                usage.cost_estimated = true;
            }
        }
        models.push(usage);
    }
    models.sort_by(|a, b| {
        b.total_tokens
            .partial_cmp(&a.total_tokens)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count,
        project_count: project_dirs.len() as u64,
        scanned_at: now,
        models,
    }
}

/// `(计费账户, 模型)` → 逐日三项。账户与模型是两个独立维度，见 `scan`。
type AccountModelDays = BTreeMap<(String, String), BTreeMap<String, (f64, f64, f64)>>;

fn scan_tree(
    dir: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_account: &mut AccountModelDays,
    session_count: &mut u64,
    project_dirs: &mut HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Top level under sessions/ is the project slug.
                if let Some(parent) = path.parent() {
                    if parent.file_name().and_then(|n| n.to_str()) == Some("sessions") {
                        project_dirs.insert(name.to_string());
                    }
                }
            }
            scan_tree(&path, by_day, by_account, session_count, project_dirs);
        } else if is_session_file(&path) && scan_file(&path, by_day, by_account) {
            // Only the newest version of a session directory is counted; a
            // v3+v4 pair is one session, not two.
            *session_count += 1;
        }
    }
}

/// Whether `path` is a dsh session log (`session.v<digits>.jsonl.zstd`).
/// Plain `*.jsonl` and other `.zstd` payloads are deliberately excluded.
fn is_session_file(path: &Path) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("zstd") {
        return false;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("session.v") && n.ends_with(".jsonl.zstd"))
}

/// Aggregate one session log. Returns `true` when this file is the newest
/// version present for its session directory, i.e. when it is *the* file that
/// should be counted as one session.
fn scan_file(
    path: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_account: &mut AccountModelDays,
) -> bool {
    // A session directory may hold both v3 and v4; only read the newest.
    let Some(version) = session_version(path) else {
        return false;
    };
    if has_newer_sibling(path, version) {
        return false;
    }
    // A session that exists but cannot be decoded still counts as a session;
    // it simply contributes no tokens.
    if let Some(text) = decompress_to_string(path) {
        aggregate_lines(&text, by_day, by_account);
    }
    true
}

fn aggregate_lines(
    text: &str,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_account: &mut AccountModelDays,
) {
    for line in text.lines() {
        if let Some((input, cache_read, output, date, model, provider)) = parse_line(line) {
            if date.is_empty() {
                continue;
            }
            let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
            e.0 += input;
            e.1 += cache_read;
            e.2 += output;
            let model = if model.is_empty() {
                crate::local::UNCLASSIFIED_MODEL.to_string()
            } else {
                model
            };
            let m = by_account.entry((provider, model)).or_default();
            let me = m.entry(date).or_insert((0.0, 0.0, 0.0));
            me.0 += input;
            me.1 += cache_read;
            me.2 += output;
        }
    }
}

/// Extract the numeric version from `session.v4.jsonl.zstd`.
fn session_version(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix("session.v")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Whether a higher-version session file exists in the same directory.
///
/// dsh writes `session.v3.jsonl.zstd` and `session.v4.jsonl.zstd` side by side
/// for one session when it upgrades mid-session. Reading both would double
/// count tokens *and* sessions, so the lower version yields.
fn has_newer_sibling(path: &Path, version: u32) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        is_session_file(&p) && session_version(&p).is_some_and(|v| v > version)
    })
}

/// Decompress a `.zstd` file to a String, bounded by `MAX_DECOMPRESSED_BYTES`.
fn decompress_to_string(path: &Path) -> Option<String> {
    decompress_capped(path, MAX_DECOMPRESSED_BYTES)
}

/// Decompress at most `limit` expanded bytes.
///
/// The cap sits on the decoder's **output**, not on the file handle. That is
/// the side that matters: a compression bomb is tiny compressed but enormous
/// expanded, so capping the compressed input would stop nothing while still
/// letting a corrupt frame header make us allocate without bound. We read one
/// byte past the limit so crossing the threshold ends the read cleanly, then
/// reject the result — a truncated file would otherwise be parsed as if it were
/// complete and silently under-count a session.
fn decompress_capped(path: &Path, limit: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = zstd::stream::read::Decoder::new(file).ok()?;
    let mut out = String::new();
    std::io::Read::take(decoder, limit + 1)
        .read_to_string(&mut out)
        .ok()?;
    (out.len() as u64 <= limit).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `message_line_with_model` 里写死的计费账户，真实日志里长这样
    /// （另外还见过 `opencode-go`、`local-router`、`ark-cn`）。
    const PROVIDER: &str = "volcanoagentplan";

    /// Legacy layout: usage is a chunk inside an attempt's stream.
    fn line_with(chunks: &str) -> String {
        format!(
            r#"{{"type":"assistant/attempt","seq":1,"time":1790522454929,"data":{{"turn":1,"step":1,"stream":[{chunks}]}}}}"#
        )
    }

    fn usage_chunk(input: u32, output: u32) -> String {
        format!(
            r#"{{"type":"chunk","time":1790522454927,"chunk":{{"type":"usage","usage":{{"inputTokens":{input},"outputTokens":{output},"totalTokens":{}}}}}}}"#,
            input + output
        )
    }

    /// Current layout, shaped after an observed dsh v4 session: usage sits
    /// directly on `data.usage` beside the assistant message it bills.
    fn message_line(input: u32, cache_read: u32, output: u32, total: u32) -> String {
        format!(
            r#"{{"type":"assistant/message","seq":7,"time":1790577965809,"data":{{"turn":1,"step":1,"usage":{{"inputTokens":{input},"cacheReadTokens":{cache_read},"outputTokens":{output},"totalTokens":{total}}},"message":{{"role":"assistant","content":[]}}}}}}"#
        )
    }

    /// The real shape: `source` also carries `replayState.response` with a
    /// versioned `responseModel`. Only `source.model` is the requested model.
    fn message_line_with_model(input: u32, output: u32, model: &str) -> String {
        format!(
            r#"{{"type":"assistant/message","seq":9,"time":1790577965809,"data":{{"turn":1,"step":2,"usage":{{"inputTokens":{input},"cacheReadTokens":0,"outputTokens":{output},"totalTokens":{}}},"message":{{"role":"assistant","content":[],"source":{{"kind":"model","provider":"{PROVIDER}","model":"{model}","replayState":{{"response":{{"kind":"pi-ai","model":"{model}","responseModel":"{model}-ga-260731"}}}}}}}}}}}}"#,
            input + output
        )
    }

    /// Same shape as [`message_line_with_model`] but with an explicit billing
    /// account, for the cases where one model is served by several accounts.
    fn message_line_with_source(input: u32, output: u32, model: &str, provider: &str) -> String {
        // 用 json! 拼而不是手写 format!：手写大括号极易少一层，解析失败时
        // `parse_line` 只会静默返回 None，测试会以"should parse"这种毫无信息
        // 的断言失败（本函数上一版就是这么坏的）。
        serde_json::json!({
            "type": "assistant/message",
            "seq": 9,
            "time": 1790577965809u64,
            "data": {
                "turn": 1,
                "step": 2,
                "usage": {
                    "inputTokens": input,
                    "cacheReadTokens": 0,
                    "outputTokens": output,
                    "totalTokens": input + output,
                },
                "message": {
                    "role": "assistant",
                    "content": [],
                    "source": {"kind": "model", "provider": provider, "model": model},
                },
            },
        })
        .to_string()
    }

    #[test]
    fn parses_usage_from_nested_stream_chunk() {
        let line = line_with(&usage_chunk(1200, 340));
        let (input, cache_read, output, date, model, _p) = parse_line(&line).expect("should parse");
        assert_eq!(input, 1200.0);
        assert_eq!(output, 340.0);
        // The legacy attempt layout has no cache field, so cache must read as 0
        // rather than inherit a stale value from another line.
        assert_eq!(cache_read, 0.0);
        // 1790522454929 ms → local day; only assert non-empty and well-formed
        // so the test does not depend on the runner's timezone.
        assert_eq!(date.len(), 10, "expected YYYY-MM-DD, got {date}");
        assert!(date.contains('-'));
        assert!(model.is_empty(), "no model stamped on this build");
    }

    /// dsh writes totalTokens too, but we recompute it. If the payload ever
    /// disagreed, trusting the field would silently mis-total the day.
    #[test]
    fn ignores_total_tokens_field() {
        let line = r#"{"type":"assistant/attempt","time":1790522454929,"data":{"stream":[{"type":"chunk","chunk":{"type":"usage","usage":{"inputTokens":10,"outputTokens":5,"totalTokens":999999}}}]}}"#;
        let (input, _cache, output, _, _, _) = parse_line(line).expect("should parse");
        assert_eq!(input, 10.0);
        assert_eq!(output, 5.0);
    }

    /// Retried attempts stream several usage chunks; the last one is the
    /// settled total. Reading the first would under-count.
    #[test]
    fn takes_last_usage_chunk_on_retry() {
        let line = line_with(&format!("{},{}", usage_chunk(10, 1), usage_chunk(10, 1)));
        let (input, _cache, output, _, _, _) = parse_line(&line).expect("should parse");
        assert_eq!(input, 10.0, "must not sum chunks");
        assert_eq!(output, 1.0, "must not sum chunks");
    }

    /// A failed request (bad API key) still logs a usage chunk, but with zeros.
    /// Counting it would add empty rows and hide the real problem.
    #[test]
    fn skips_zero_usage_from_failed_attempts() {
        let line = line_with(&usage_chunk(0, 0));
        assert!(parse_line(&line).is_none(), "zero usage must be skipped");
    }

    /// The layout that actually dominates real sessions (921 records vs 24 on
    /// a live machine). If this regresses, the tool silently reports near-zero
    /// usage while the user is actively chatting — the worst failure mode.
    #[test]
    fn parses_usage_from_current_message_layout() {
        let line = message_line(18172, 0, 2001, 20173);
        let (input, cache_read, output, date, model, _p) = parse_line(&line).expect("should parse");
        assert_eq!(input, 18172.0);
        assert_eq!(cache_read, 0.0);
        assert_eq!(output, 2001.0);
        assert_eq!(date.len(), 10, "expected YYYY-MM-DD, got {date}");
        assert!(model.is_empty(), "no model stamped on this build");
    }

    /// `totalTokens` is input+output+cacheRead, NOT input+output. Folding the
    /// cache portion into input would inflate the input rate, since cached
    /// tokens are billed differently. This is the exact shape seen on disk.
    #[test]
    fn separates_cache_read_from_input() {
        let line = message_line(403, 12800, 266, 13469);
        let (input, cache_read, output, _, _, _) = parse_line(&line).expect("should parse");
        assert_eq!(input, 403.0);
        assert_eq!(cache_read, 12800.0);
        assert_eq!(output, 266.0);
        // Guard the invariant the pricing layer relies on.
        assert_ne!(
            input + output,
            13469.0,
            "totalTokens includes cacheRead, so it must not be treated as input+output"
        );
    }

    /// A message whose usage is all zeros (auth failure / quota error) is noise.
    #[test]
    fn skips_zero_usage_from_failed_messages() {
        assert!(parse_line(&message_line(0, 0, 0, 0)).is_none());
    }

    /// A cache-only message still carries billable usage and must not be dropped.
    #[test]
    fn keeps_cache_only_message() {
        let line = message_line(0, 9472, 0, 9472);
        let (input, cache_read, output, _, _, _) = parse_line(&line).expect("should parse");
        assert_eq!(input, 0.0);
        assert_eq!(cache_read, 9472.0);
        assert_eq!(output, 0.0);
    }

    /// The model rides on the message, so per-model splitting needs no
    /// cross-event correlation. Verified against real logs: 924/924 usage
    /// lines carry it, split across two models mid-session.
    #[test]
    fn reads_model_from_message_source() {
        let line = message_line_with_model(9677, 427, "deepseek-v4-flash");
        let (_, _, _, _, model, _) = parse_line(&line).expect("should parse");
        assert_eq!(model, "deepseek-v4-flash");
    }

    /// `replayState.response.responseModel` is a server-side versioned alias
    /// (`deepseek-v4-flash-ga-260731`). The price table is keyed by the
    /// requested id, so picking it would leave every row unpriced.
    #[test]
    fn prefers_requested_model_over_response_model() {
        let line = message_line_with_model(100, 10, "MiniMax-M3");
        let (_, _, _, _, model, _) = parse_line(&line).expect("should parse");
        assert_eq!(
            model, "MiniMax-M3",
            "must not pick the -ga- versioned alias"
        );
        // Whether the id resolves to a price depends on the estimation switch;
        // the model-extraction guarantee above holds either way.
        if crate::pricing::COST_ESTIMATION_ENABLED {
            assert!(
                crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
                    input: 1_000_000.0,
                    cache_read: 0.0,
                    output: 1_000_000.0,
                    model_id: Some(model.clone()),
                })
                .is_some(),
                "the extracted model must resolve to a price tier"
            );
        }
    }

    /// Both observed models must be priceable, otherwise splitting them buys
    /// two unpriced buckets instead of one.
    #[test]
    fn observed_models_are_priceable() {
        for m in ["deepseek-v4-flash", "MiniMax-M3"] {
            let line = message_line_with_model(10, 5, m);
            let (_, _, _, _, model, _) = parse_line(&line).expect("should parse");
            assert_eq!(model, m, "model must survive extraction verbatim");
            if crate::pricing::COST_ESTIMATION_ENABLED {
                assert!(
                    crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
                        input: 1_000.0,
                        cache_read: 0.0,
                        output: 1_000.0,
                        model_id: Some(model),
                    })
                    .is_some(),
                    "{m} should resolve to a price tier"
                );
            }
        }
    }

    /// A message with no `source` at all still yields an empty model, which the
    /// aggregator maps to the unclassified bucket so totals still reconcile.
    #[test]
    fn message_without_source_leaves_model_unset() {
        let line = message_line(100, 0, 50, 150);
        let (_, _, _, _, model, _) = parse_line(&line).expect("should parse");
        assert!(model.is_empty(), "expected unclassified, got {model}");
    }

    /// Sessions switch models mid-run; aggregation must keep them in separate
    /// buckets while the per-day total stays the sum of both.
    #[test]
    fn aggregation_splits_models_and_still_reconciles() {
        let text = format!(
            "{}\n{}\n{}",
            message_line_with_model(1000, 100, "MiniMax-M3"),
            message_line_with_model(4000, 40, "deepseek-v4-flash"),
            message_line_with_model(500, 50, "MiniMax-M3"),
        );
        let mut by_day = BTreeMap::new();
        let mut by_model = BTreeMap::new();
        aggregate_lines(&text, &mut by_day, &mut by_model);

        let day_total: f64 = by_day.values().map(|v| v.0 + v.1 + v.2).sum();
        let model_total: f64 = by_model
            .values()
            .flat_map(|m| m.values())
            .map(|v| v.0 + v.1 + v.2)
            .sum();
        assert_eq!(day_total, 5690.0, "1000+100+4000+40+500+50");
        assert_eq!(
            model_total, day_total,
            "per-model must reconcile with per-day"
        );

        let mm: f64 = by_model
            .get(&(PROVIDER.into(), "MiniMax-M3".into()))
            .unwrap()
            .values()
            .map(|v| v.0 + v.1 + v.2)
            .sum();
        assert_eq!(mm, 1650.0, "1500 + 150");
        let ds: f64 = by_model
            .get(&(PROVIDER.into(), "deepseek-v4-flash".into()))
            .unwrap()
            .values()
            .map(|v| v.0 + v.1 + v.2)
            .sum();
        assert_eq!(ds, 4040.0);
    }

    /// 计费账户必须跟着每条消息走：真实日志里 `deepseek-v4.1-flash` 大部分
    /// 是经 `opencode-go` 调用的。只读模型名会把账单算进 DeepSeek。
    #[test]
    fn reads_provider_from_message_source() {
        let line = message_line_with_model(100, 10, "deepseek-v4.1-flash");
        let (_, _, _, _, model, provider) = parse_line(&line).expect("should parse");
        assert_eq!(model, "deepseek-v4.1-flash");
        assert_eq!(provider, PROVIDER);
    }

    /// 老版 attempt 布局没有 `message.source`，账户只能留空（归因层再按模型名
    /// 兜底），绝不能凭空捏一个账户。
    #[test]
    fn legacy_attempt_layout_leaves_provider_unset() {
        let line = line_with(&usage_chunk(1200, 340));
        let (_, _, _, _, _, provider) = parse_line(&line).expect("should parse");
        assert!(provider.is_empty(), "expected no provider, got {provider}");
    }

    /// 走本机路由的会话模型名是 `auto`，必须落成哨兵而不是当真实模型记账。
    /// 审计实测这批 1.14 亿 tokens 之前既没有进 `__via_router__` 替换链，也没
    /// 归到任何真实模型，在模型页变成一个叫 "auto" 的假桶。
    #[test]
    fn routed_session_becomes_the_router_sentinel() {
        let line = message_line_with_source(1000, 200, "auto", "local-router");
        let (_, _, _, _, model, provider) = parse_line(&line).expect("should parse");
        assert_eq!(model, crate::local::ROUTED_MODEL);
        // 账户仍然要留着：它说明这批 token 是路由代付的，前端做替换归因要用。
        assert_eq!(provider, "local-router");
    }

    /// 反例：`auto` 出现在真实上游账户下时是上游自己给的模型名，不能凭字面量
    /// 当路由。判据必须是账户，不是 "auto" 这三个字母。
    #[test]
    fn auto_on_a_real_provider_is_not_a_routed_session() {
        let line = message_line_with_source(1000, 200, "auto", "opencode-go");
        let (_, _, _, _, model, provider) = parse_line(&line).expect("should parse");
        assert_eq!(model, "auto");
        assert_eq!(provider, "opencode-go");
    }

    /// 同一个模型经两个账户调用时要分成两桶：合成一行等于把账单强行归给其中
    /// 一方，正是用户看到的"用量都算进 DeepSeek"那个 bug。
    #[test]
    fn same_model_on_two_providers_splits_into_two_buckets() {
        let text = format!(
            "{}\n{}\n{}",
            message_line_with_source(900, 90, "deepseek-v4.1-flash", "opencode-go"),
            message_line_with_source(100, 10, "deepseek-v4.1-flash", "deepseek"),
            message_line_with_source(400, 40, "deepseek-v4.1-flash", "opencode-go"),
        );
        let mut by_day = BTreeMap::new();
        let mut by_account = BTreeMap::new();
        aggregate_lines(&text, &mut by_day, &mut by_account);

        assert_eq!(by_account.len(), 2, "two accounts must stay separate");
        let sum_for = |provider: &str| -> f64 {
            by_account
                .get(&(provider.to_string(), "deepseek-v4.1-flash".to_string()))
                .unwrap()
                .values()
                .map(|v| v.0 + v.1 + v.2)
                .sum()
        };
        assert_eq!(sum_for("opencode-go"), 1430.0, "900+90 + 400+40");
        assert_eq!(sum_for("deepseek"), 110.0, "100+10");
        let day_total: f64 = by_day.values().map(|v| v.0 + v.1 + v.2).sum();
        assert_eq!(day_total, 1540.0, "per-day total must still reconcile");
    }

    /// 同上，但模型名的空值桶也要按账户拆开：未记模型的调用同样可能来自不同
    /// 账户，合桶会让未分类用量归错账单。
    #[test]
    fn unclassified_model_is_still_split_by_provider() {
        let text = format!(
            "{}\n{}",
            message_line_with_source(10, 1, "", "opencode-go"),
            message_line_with_source(20, 2, "", "ark-cn"),
        );
        let mut by_day = BTreeMap::new();
        let mut by_account = BTreeMap::new();
        aggregate_lines(&text, &mut by_day, &mut by_account);

        let unclassified = crate::local::UNCLASSIFIED_MODEL.to_string();
        assert_eq!(by_account.len(), 2);
        assert!(by_account
            .contains_key(&("opencode-go".to_string(), unclassified.clone())));
        assert!(by_account.contains_key(&("ark-cn".to_string(), unclassified)));
    }

    #[test]
    fn ignores_lines_without_usage_or_with_wrong_type() {
        assert!(parse_line(r#"{"type":"tool/call","data":{}}"#).is_none());
        assert!(parse_line(r#"{"type":"assistant/attempt","data":{}}"#).is_none());
        // assistant/attempt but the stream has no usage chunk
        assert!(parse_line(&line_with(
            r#"{"type":"chunk","chunk":{"type":"finish","reason":{"kind":"stop"}}}"#
        ))
        .is_none());
        assert!(parse_line("not json at all").is_none());
    }

    #[test]
    fn recognizes_only_session_zstd_files() {
        assert!(is_session_file(Path::new("/x/session.v4.jsonl.zstd")));
        assert!(is_session_file(Path::new("/x/session.v3.jsonl.zstd")));
        assert!(!is_session_file(Path::new("/x/session.jsonl")));
        assert!(!is_session_file(Path::new("/x/other.v4.jsonl.zstd")));
        assert!(!is_session_file(Path::new("/x/session.v4.json")));
    }

    #[test]
    fn parses_session_version() {
        assert_eq!(
            session_version(Path::new("/x/session.v4.jsonl.zstd")),
            Some(4)
        );
        assert_eq!(
            session_version(Path::new("/x/session.v3.jsonl.zstd")),
            Some(3)
        );
        assert_eq!(session_version(Path::new("/x/random.txt")), None);
    }

    /// Only the newest version in a session dir should be read, else an
    /// upgraded session gets counted twice.
    #[test]
    fn older_version_is_skipped_when_newer_exists() {
        let dir = std::env::temp_dir().join(format!("dsh-ver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v3 = dir.join("session.v3.jsonl.zstd");
        let v4 = dir.join("session.v4.jsonl.zstd");
        std::fs::write(&v3, b"x").unwrap();
        std::fs::write(&v4, b"x").unwrap();
        assert!(has_newer_sibling(&v3, 3), "v3 must yield to v4");
        assert!(!has_newer_sibling(&v4, 4), "v4 is the newest");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn epoch_millis_maps_to_a_day() {
        let day = day_of_epoch_millis(1790522454929).expect("valid millis");
        assert_eq!(day.len(), 10);
        // Out-of-range timestamps must not panic.
        assert!(day_of_epoch_millis(u64::MAX).is_none());
        assert!(day_of_epoch_millis(0).is_some());
    }

    /// Round-trip a real zstd payload through the decompressor to prove the
    /// reader path works end to end (not just the JSON parsing).
    #[test]
    fn decompresses_round_trip_payload() {
        let dir = std::env::temp_dir().join(format!("dsh-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.v4.jsonl.zstd");
        let original = line_with(&usage_chunk(4242, 24));
        let buf = zstd::stream::encode_all(original.as_bytes(), 3).unwrap();
        std::fs::write(&path, &buf).unwrap();

        let text = decompress_to_string(&path).expect("should decompress");
        let (input, _cache, output, _, _, _) =
            parse_line(text.lines().next().unwrap()).expect("should parse");
        assert_eq!(input, 4242.0);
        assert_eq!(output, 24.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn write_zstd(path: &Path, body: &str) {
        let buf = zstd::stream::encode_all(body.as_bytes(), 3).unwrap();
        std::fs::write(path, &buf).unwrap();
    }

    /// Build `<root>/sessions/` and point a scanner at it, so tests exercise the
    /// same discovery path as the real `~/.dsh` layout. Only the `sessions/`
    /// root is created: every directory under it counts as a *project*, so
    /// pre-seeding one here would leak into the project count assertions.
    fn session_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let sessions = root.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        sessions
    }

    /// The v3/v4 dedup used to be asserted only on the helper predicate, so a
    /// future refactor could still have counted a session twice end to end.
    /// This drives `scan_tree` over a real directory tree.
    #[test]
    fn scan_tree_counts_one_session_and_reads_only_newest_version() {
        let sessions = session_root("dedup");
        let s1 = sessions.join("proj").join("s1");
        std::fs::create_dir_all(&s1).unwrap();
        write_zstd(
            &s1.join("session.v3.jsonl.zstd"),
            &line_with(&usage_chunk(100, 10)),
        );
        write_zstd(
            &s1.join("session.v4.jsonl.zstd"),
            &line_with(&usage_chunk(200, 20)),
        );

        let mut by_day = BTreeMap::new();
        let mut by_model = BTreeMap::new();
        let mut session_count = 0;
        let mut projects = HashSet::new();
        scan_tree(
            &sessions,
            &mut by_day,
            &mut by_model,
            &mut session_count,
            &mut projects,
        );

        // One directory = one session, even though two version files exist.
        assert_eq!(session_count, 1, "v3+v4 must count as a single session");
        // Only v4's tokens are aggregated; v3 is fully superseded.
        let total_in: f64 = by_day.values().map(|v| v.0).sum();
        let total_out: f64 = by_day.values().map(|v| v.2).sum();
        assert_eq!(total_in, 200.0, "must not add v3 and v4 together");
        assert_eq!(total_out, 20.0);
        assert_eq!(projects.len(), 1, "project slug is the dir under sessions/");
        std::fs::remove_dir_all(sessions.parent().unwrap()).ok();
    }

    /// Each session directory is its own session, and each keeps its own
    /// project attribution.
    #[test]
    fn scan_tree_sums_across_sessions_and_projects() {
        let sessions = session_root("multi");
        let p1 = sessions.join("projA");
        let p2 = sessions.join("projB");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::create_dir_all(&p2).unwrap();
        write_zstd(
            &p1.join("session.v4.jsonl.zstd"),
            &line_with(&usage_chunk(50, 5)),
        );
        write_zstd(
            &p2.join("session.v4.jsonl.zstd"),
            &line_with(&usage_chunk(70, 7)),
        );

        let mut by_day = BTreeMap::new();
        let mut by_model = BTreeMap::new();
        let mut session_count = 0;
        let mut projects = HashSet::new();
        scan_tree(
            &sessions,
            &mut by_day,
            &mut by_model,
            &mut session_count,
            &mut projects,
        );

        assert_eq!(session_count, 2);
        assert_eq!(projects.len(), 2);
        let total: f64 = by_day.values().map(|v| v.0 + v.2).sum();
        assert_eq!(total, 132.0);
        std::fs::remove_dir_all(sessions.parent().unwrap()).ok();
    }

    /// A file that explodes past the cap must be rejected outright. Returning a
    /// truncated prefix would be worse than failing: the caller would parse an
    /// incomplete session and quietly under-report usage.
    #[test]
    fn rejects_payload_that_exceeds_the_decompress_cap() {
        let dir = std::env::temp_dir().join(format!("dsh-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.v4.jsonl.zstd");
        let body = "x".repeat(64 * 1024);
        write_zstd(&path, &body);

        // Same file is fine under a generous cap and rejected under a tight one.
        assert_eq!(
            decompress_capped(&path, 1024 * 1024).unwrap().len(),
            body.len()
        );
        assert!(
            decompress_capped(&path, 1024).is_none(),
            "over-cap payload must be rejected, not truncated"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The default cap must stay well below the memory a hostile file could ask
    /// for, while leaving orders of magnitude of headroom for real sessions.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn default_cap_is_bounded_and_generous() {
        // These look like tautologies to the linter, and that is the point: the
        // cap is a security boundary, so any future edit that widens or guts it
        // must fail here rather than quietly change how much memory a hostile
        // log can force us to allocate.
        assert!(MAX_DECOMPRESSED_BYTES <= 512 * 1024 * 1024);
        assert!(MAX_DECOMPRESSED_BYTES >= 64 * 1024 * 1024);
    }

    /// A non-zstd file must fail cleanly rather than panic or return garbage.
    #[test]
    fn rejects_non_zstd_payload() {
        let dir = std::env::temp_dir().join(format!("dsh-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.v4.jsonl.zstd");
        std::fs::write(&path, b"this is definitely not zstd").unwrap();
        assert!(decompress_to_string(&path).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
