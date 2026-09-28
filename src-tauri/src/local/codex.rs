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

fn scan_file(
    path: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) {
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
            // 无模型的行 token 归入"未标记模型"桶，避免按模型汇总漏算。
            let model = if model.is_empty() {
                crate::local::UNCLASSIFIED_MODEL.to_string()
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
}