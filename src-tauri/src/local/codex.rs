//! OpenAI Codex CLI local session scanner.
//!
//! Codex writes its rollouts to `~/.codex/sessions/YYYY-MM-DD/*.jsonl` (one JSON
//! object per line). Each completed request carries a `usage` object with
//! `prompt_tokens` / `completion_tokens` / `total_tokens` (and
//! `prompt_tokens_details.cached_tokens` on newer builds). We read only that
//! usage + the top-level `timestamp`/`model`, never the conversation text.
//!
//! The nesting has drifted across Codex versions (`usage` can sit at the top
//! level or under `payload`), so parsing is deliberately shallow and tolerant:
//! we look for a `"usage"` object wherever it appears and read `timestamp`/
//! `model` from the same object or the line root.

use super::{finalize_days, finalize_models, LocalToolReport};
use chrono::{DateTime, Local, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const TOOL_ID: &str = "codex";
const TOOL_NAME: &str = "Codex";
const KEEP_DAYS: usize = 90;

/// Parse one session line into `(input, cache_read, output, local_day, model)`
/// when it carries a codex `usage` object; otherwise `None`. `pub(crate)` so the
/// byte-level delta scanner can reuse it on appended lines.
pub(crate) fn parse_line(line: &str) -> Option<(f64, f64, f64, String, String)> {
    if !line.contains("usage") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    // New format (Codex >= 0.160, desktop rollouts): usage arrives as
    // event_msg lines with payload.type == "token_count". The per-request
    // numbers live in payload.info.last_token_usage; total_token_usage is a
    // session-cumulative counter and must never be summed (it repeats on
    // every event, so accumulating it would square-count the whole session).
    // Field semantics mirror the old shape: input_tokens includes
    // cached_input_tokens (a subset, like the old prompt/cached pair), and
    // reasoning_output_tokens is a subset of output_tokens.
    if v.pointer("/payload/type").and_then(|t| t.as_str()) == Some("token_count") {
        let u = v.pointer("/payload/info/last_token_usage")?;
        let input_total = u.get("input_tokens").and_then(|n| n.as_f64()).unwrap_or(0.0);
        let cached = u
            .get("cached_input_tokens")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let output = u.get("output_tokens").and_then(|n| n.as_f64()).unwrap_or(0.0);
        let input = (input_total - cached).max(0.0);
        if input <= 0.0 && cached <= 0.0 && output <= 0.0 {
            return None;
        }
        let date = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(day_of)
            .unwrap_or_default();
        // token_count events carry no model field; the empty model routes the
        // tokens into the "unclassified model" bucket so per-model sums keep
        // matching per-day totals.
        return Some((input, cached, output, date, String::new()));
    }
    // usage may sit at line root or under `payload` (version drift).
    let usage = v.get("usage").or_else(|| v.pointer("/payload/usage"))?;
    let f = |name: &str| -> f64 {
        usage.get(name).and_then(|n| n.as_f64()).unwrap_or_default()
    };
    let prompt = f("prompt_tokens");
    let output = f("completion_tokens");
    // cached is a subset of prompt_tokens; split it out so fresh input is
    // `prompt - cached` (mirrors Claude Code's cache_read semantics).
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|n| n.as_f64())
        .unwrap_or(0.0);
    let input = (prompt - cached).max(0.0);
    if input <= 0.0 && cached <= 0.0 && output <= 0.0 {
        return None;
    }
    let date = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(day_of)
        .unwrap_or_default();
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.pointer("/payload/model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    Some((input, cached, output, date, model))
}

fn day_of(iso: &str) -> Option<String> {
    let dt: DateTime<Utc> = DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&Utc))
        .ok()?;
    Some(dt.with_timezone(&Local).date_naive().to_string())
}

/// Scan Codex session logs under Windows and every WSL distro home.
pub fn scan() -> LocalToolReport {
    let now = Utc::now().to_rfc3339();
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut session_count: u64 = 0;
    let mut session_dirs: std::collections::HashSet<String> = Default::default();

    for sessions in super::wsl::existing_dotdirs(".codex/sessions") {
        scan_tree(&sessions, &mut by_day, &mut by_model, &mut session_count, &mut session_dirs);
    }

    let daily = finalize_days(by_day.clone(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    let mut models = finalize_models(by_model, &BTreeMap::new(), KEEP_DAYS);
    // Estimate cost for known models (Codex logs don't report cost).
    for m in &mut models {
        if m.cost > 0.0 {
            continue;
        }
        // Cache reads are billed at a discount by `compute_cost`, so they must
        // be passed through separately. Pre-merging them into `input` would
        // charge them at the full input rate — with cache at 80-98% of real
        // traffic that inflated the estimate badly.
        let input: f64 = m.daily.iter().map(|d| d.input).sum();
        let cache_read: f64 = m.daily.iter().map(|d| d.cache_read).sum();
        let output: f64 = m.daily.iter().map(|d| d.output).sum();
        if let Some(cost) = crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
            input,
            cache_read,
            output,
            model_id: Some(m.model.clone()),
        }) {
            if cost > 0.0 {
                m.cost = cost;
                m.currency = "USD".to_string();
                m.cost_estimated = true;
            }
        }
    }

    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count,
        project_count: session_dirs.len() as u64,
        scanned_at: now,
        models,
    }
}

fn scan_tree(
    dir: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
    session_count: &mut u64,
    session_dirs: &mut std::collections::HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_tree(&path, by_day, by_model, session_count, session_dirs);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                session_dirs.insert(name.to_string());
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            *session_count += 1;
            scan_file(&path, by_day, by_model);
        }
    }
}

/// 预扫一个 Codex session 文件，取出它的**会话级模型名**。
///
/// 取自 `turn_context` 事件——那是 Codex 每轮开始时落盘的上下文，其中带
/// `model`。用法量事件本身不带模型，配置直连上游时这里是真实模型名；配置
/// 指向 TokenRouter 时这里是占位名 `LocalRouter`（由调用方转成哨兵）。
fn session_model_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if !line.contains("\"turn_context\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
            let m = m.trim();
            if !m.is_empty() {
                return Some(m.to_string());
            }
        }
    }
    None
}

fn scan_file(
    path: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) {
    // Codex 的用量事件（`token_usage_record`）**不携带 model 字段**，模型记在
    // 同文件的 `turn_context` 事件里。一份文件就是一个会话，所以先扫出会话
    // 模型，再回填到那些自身无模型的用量行上——否则 Codex 全部用量都只能落
    // 「未标记模型」。
    let session_model = session_model_of(path);
    let Ok(file) = File::open(path) else {
        return;
    };
    let reader = BufReader::with_capacity(64 * 1024, file);
    for line in reader.lines().flatten() {
        if let Some((input, cache_read, output, date, model)) = parse_line(&line) {
            if date.is_empty() {
                continue;
            }
            let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
            e.0 += input;
            e.1 += cache_read;
            e.2 += output;
            // 优先级：行内自带 > 会话级回填 > 未标记。回填出来的若是指向本地
            // 路由的占位名（`LocalRouter`），再打成哨兵交由路由自记账归因。
            let model = if !model.is_empty() {
                model
            } else if let Some(sm) = &session_model {
                sm.clone()
            } else {
                crate::local::UNCLASSIFIED_MODEL.to_string()
            };
            let model = if crate::local::is_routed_placeholder(&model) {
                crate::local::ROUTED_MODEL.to_string()
            } else {
                model
            };
            let m = by_model.entry(model).or_default();
            let me = m.entry(date).or_insert((0.0, 0.0, 0.0));
            me.0 += input;
            me.1 += cache_read;
            me.2 += output;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_flat_usage_line() {
        let line = r#"{"timestamp":"2026-01-02T03:04:05Z","model":"gpt-5","usage":{"prompt_tokens":200,"completion_tokens":50,"total_tokens":250}}"#;
        let (i, cr, o, date, model) = parse_line(line).unwrap();
        assert_eq!(i, 200.0);
        assert_eq!(cr, 0.0);
        assert_eq!(o, 50.0);
        assert_eq!(date.len(), 10);
        assert_eq!(model, "gpt-5");
    }

    #[test]
    fn splits_cached_tokens_out_of_prompt() {
        let line = r#"{"timestamp":"2026-01-02T00:00:00Z","usage":{"prompt_tokens":500,"prompt_tokens_details":{"cached_tokens":120},"completion_tokens":30}}"#;
        let (i, cr, o, _, _) = parse_line(line).unwrap();
        assert_eq!(i, 380.0); // 500 - cached 120
        assert_eq!(cr, 120.0);
        assert_eq!(o, 30.0);
    }

    #[test]
    fn parses_nested_payload_usage() {
        let line = r#"{"timestamp":"2026-01-02T00:00:00Z","type":"response_item","payload":{"model":"gpt-5","usage":{"prompt_tokens":10,"completion_tokens":4}}}"#;
        let (i, _cr, o, _, model) = parse_line(line).unwrap();
        assert_eq!(i, 10.0);
        assert_eq!(o, 4.0);
        assert_eq!(model, "gpt-5");
    }

    #[test]
    fn ignores_lines_without_usage() {
        assert!(parse_line(r#"{"type":"message","content":"hi"}"#).is_none());
    }

    #[test]
    fn day_of_normalizes_utc_to_local() {
        assert_eq!(day_of("2026-01-02T03:04:05Z").unwrap().len(), 10);
    }

    #[test]
    fn parses_token_count_event_from_last_usage() {
        // Mirrors a real 2026-10 rollout line (cli 0.160). The cumulative
        // total_token_usage must be ignored in favour of the per-request
        // last_token_usage, otherwise each event re-counts the session.
        let line = r#"{"timestamp":"2026-10-05T13:11:15.397Z","ordinal":26,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":47669,"cached_input_tokens":23552,"cache_write_input_tokens":0,"output_tokens":517,"reasoning_output_tokens":85,"total_tokens":48186},"last_token_usage":{"input_tokens":24067,"cached_input_tokens":23552,"cache_write_input_tokens":0,"output_tokens":93,"reasoning_output_tokens":21,"total_tokens":24160},"model_context_window":850000},"rate_limits":{}}}"#;
        let (i, cr, o, date, model) = parse_line(line).unwrap();
        assert_eq!(i, 515.0); // 24067 - cached 23552
        assert_eq!(cr, 23552.0);
        assert_eq!(o, 93.0);
        assert_eq!(date.len(), 10);
        assert!(model.is_empty());
    }

    #[test]
    fn ignores_zero_usage_token_count_event() {
        let line = r#"{"timestamp":"2026-10-05T13:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"total_tokens":0},"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"total_tokens":0}}}}"#;
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn token_count_without_last_usage_is_rejected() {
        // Only the cumulative counter present: counting it would double-count
        // every prior event of the session, so refuse rather than guess.
        let line = r#"{"timestamp":"2026-10-05T13:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"total_tokens":110}}}}"#;
        assert!(parse_line(line).is_none());
    }

    fn write_session(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn session_model_comes_from_turn_context() {
        // Codex 的用量事件不带 model，模型只在 turn_context 里。
        let dir = std::env::temp_dir().join(format!("tum-codex-sm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write_session(
            &dir,
            "s1.jsonl",
            &[
                r#"{"type":"turn_context","model":"LocalRouter","cwd":"D:/x"}"#,
                r#"{"timestamp":"2026-10-05T13:11:15Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"total_tokens":90}}}}"#,
            ],
        );
        assert_eq!(session_model_of(&p).as_deref(), Some("LocalRouter"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_model_is_none_without_turn_context() {
        let dir = std::env::temp_dir().join(format!("tum-codex-sm2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write_session(&dir, "s2.jsonl", &[r#"{"type":"event_msg"}"#]);
        assert_eq!(session_model_of(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_file_attributes_usage_to_session_model_and_marks_routed() {
        // 端到端：无模型的用量行按会话模型归因；`LocalRouter` 是路由占位名，
        // 必须打成哨兵而不是当成真实模型。
        let dir = std::env::temp_dir().join(format!("tum-codex-sm3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, ctx) in [("routed.jsonl", "LocalRouter"), ("direct.jsonl", "gpt-5")] {
            let p = write_session(
                &dir,
                name,
                &[
                    &format!(r#"{{"type":"turn_context","model":"{ctx}"}}"#),
                    r#"{"timestamp":"2026-10-05T13:11:15Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"total_tokens":90}}}}"#,
                ],
            );
            let mut by_day = BTreeMap::new();
            let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
            scan_file(&p, &mut by_day, &mut by_model);
            let day: Vec<f64> = by_day.values().map(|d| d.0 + d.1 + d.2).collect();
            // input_tokens=100 是含缓存的 prompt，解析后 input=100-20=80，
            // 合计 80 + 20(cache) + 10(output) = 110。
            assert_eq!(day.iter().sum::<f64>(), 110.0, "按日总量与源一致 ({name})");
        }

        let mut by_day = BTreeMap::new();
        let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
        for name in ["routed.jsonl", "direct.jsonl"] {
            scan_file(&dir.join(name), &mut by_day, &mut by_model);
        }
        let keys: Vec<&String> = by_model.keys().collect();
        assert!(
            keys.iter().any(|k| k.as_str() == crate::local::ROUTED_MODEL),
            "LocalRouter 会话须打成哨兵，实际键: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.as_str() == "gpt-5"),
            "直连会话保留真实模型名，实际键: {keys:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
