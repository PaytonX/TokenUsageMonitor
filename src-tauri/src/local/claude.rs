//! Claude Code local log scanner.
//!
//! Reads the CLI/desktop session logs under `~/.claude/projects/**/*.jsonl`.
//! Each `type:"assistant"` line carries a token breakdown in `message.usage`
//! (`input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`,
//! `output_tokens`) plus a UTC `timestamp` and `message.model`. We aggregate
//! those tokens by local day into a [`LocalToolReport`].
//!
//! Robustness: parsing is version-tolerant (JSON `Value`), sub-agent logs are
//! skipped (their tokens are counted inside the parent session), and lines that
//! aren't assistant messages are skipped before the JSON parse.

use super::{LocalToolReport};
use crate::local::{finalize_days, finalize_models};
use chrono::{DateTime, Local, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Tools/aliases the UI can show; the id is stable across scans.
const TOOL_ID: &str = "claude-code";
const TOOL_NAME: &str = "Claude Code";
/// Keep the most recent N days of daily aggregates.
const KEEP_DAYS: usize = 90;

/// Serialized field names used by the Claude Code log format (stable across
/// recent versions). Add future aliases here if the format drifts.
const K_MSG: &str = "message";
const K_INPUT: &str = "input_tokens";
const K_CACHE_CREATE: &str = "cache_creation_input_tokens";
const K_CACHE_READ: &str = "cache_read_input_tokens";
const K_OUTPUT: &str = "output_tokens";

/// Parse one assistant line into (input, cache_read, output, local_day, model)
/// if the line carries usage; otherwise `None`. `pub(crate)` so the byte-level
/// delta scanner can reuse it on appended lines.
pub(crate) fn parse_assistant(line: &str) -> Option<(f64, f64, f64, String, String)> {
    if !line.contains(K_MSG) {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let usage = v.pointer("/message/usage")?;
    let f = |name: &str| -> f64 {
        usage
            .get(name)
            .and_then(|n| n.as_f64())
            .unwrap_or_default()
    };
    let input = f(K_INPUT);
    let cache_read = f(K_CACHE_READ) + f(K_CACHE_CREATE);
    let output = f(K_OUTPUT);
    // Token counts could be zero on some messages; skip noise (no usage).
    if input <= 0.0 && cache_read <= 0.0 && output <= 0.0 {
        return None;
    }
    let date = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(day_of)
        .unwrap_or_default();
    let model = v
        .pointer("/message/model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    Some((input, cache_read, output, date, model))
}

/// Convert a UTC ISO timestamp to a local `YYYY-MM-DD` day key.
fn day_of(iso: &str) -> Option<String> {
    let dt: DateTime<Utc> = DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&Utc))
        .ok()?;
    Some(dt.with_timezone(&Local).date_naive().to_string())
}

/// Scan the Claude Code session logs and aggregate token usage by local day.
pub fn scan() -> LocalToolReport {
    let now = Utc::now().to_rfc3339();
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut session_count: u64 = 0;
    let mut projects: std::collections::HashSet<String> = Default::default();

    // Scan Windows home and every WSL distro home (`.claude/projects`).
    for projects_dir in super::wsl::existing_dotdirs(".claude/projects") {
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan_project(&path, &mut by_day, &mut by_model, &mut session_count, &mut projects, true);
                }
            }
        }
    }

    let daily = finalize_days(by_day.clone(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    // Claude Code 日志未记录成本 → 对已知价目表的模型用 pricing 估算成本。
    let mut models = finalize_models(by_model, &BTreeMap::new(), KEEP_DAYS);
    for m in &mut models {
        if m.cost > 0.0 {
            continue;
        }
        let input: f64 = m.daily.iter().map(|d| d.input + d.cache_read).sum();
        let output: f64 = m.daily.iter().map(|d| d.output).sum();
        if let Some(cost) = crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
            input,
            cache_read: 0.0,
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
        project_count: projects.len() as u64,
        scanned_at: now,
        models,
    }
}

/// Recursively scan one folder for `*.jsonl` logs, aggregating token usage from
/// *every* file (including the `subagents/` directories — those carry real
/// consumed tokens). Only the top-level session files (immediate jsonl under a
/// project folder) increment `session_count`; deeper sub-agent files count their
/// tokens but not as a distinct session.
fn scan_project(
    dir: &Path,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
    session_count: &mut u64,
    projects: &mut std::collections::HashSet<String>,
    count_sessions_here: bool,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_project(&path, by_day, by_model, session_count, projects, false);
        } else if is_jsonl(&path) {
            if count_sessions_here {
                *session_count += 1; // a top-level session file
                if let Some(dirname) = dir.file_name().and_then(|n| n.to_str()) {
                    projects.insert(dirname.to_string());
                }
            }
            scan_file(&path, by_day, by_model);
        }
    }
}

fn is_jsonl(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

/// Read one `*.jsonl` file, aggregating assistant-message token usage.
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
        if let Some((input, cache_read, output, date, model)) = parse_assistant(&line) {
            if date.is_empty() {
                continue;
            }
            let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
            e.0 += input;
            e.1 += cache_read;
            e.2 += output;
            // 无模型的消息 token 仍计入"未标记模型"桶，保证按日总量与按模型汇总一致。
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
    fn parses_assistant_line_usage() {
        let line = r#"{"type":"assistant","timestamp":"2026-01-02T03:04:05Z","message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":10,"cache_read_input_tokens":20,"output_tokens":5},"model":"claude-sonnet-4-20250514"}}"#;
        let (i, cr, o, date, model) = parse_assistant(line).unwrap();
        assert_eq!(i, 100.0);
        assert_eq!(cr, 30.0); // cache_read + cache_creation
        assert_eq!(o, 5.0);
        assert!(!date.is_empty());
        assert_eq!(model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn ignores_non_assistant_and_empty_usage() {
        assert!(parse_assistant(r#"{"type":"user","message":"hi"}"#).is_none());
        assert!(parse_assistant(
            r#"{"type":"assistant","timestamp":"2026-01-02T00:00:00Z","message":{"usage":{"input_tokens":0}}}"#
        )
        .is_none());
    }

    #[test]
    fn finalize_days_keeps_window() {
        let mut map = BTreeMap::new();
        map.insert("2026-01-01".to_string(), (1.0, 0.0, 0.0));
        map.insert("2026-01-02".to_string(), (2.0, 1.0, 0.0));
        let days = finalize_days(map, 1);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-01-02");
        assert_eq!(days[0].total, 3.0);
    }

    #[test]
    fn day_of_normalizes_utc_to_local() {
        // Fixed offset-free path: just ensure it yields a YYYY-MM-DD string.
        let d = day_of("2026-01-02T03:04:05Z").unwrap();
        assert_eq!(d.len(), 10);
    }

    #[test]
    #[ignore] // environment-dependent smoke scan (prints real-log summary)
    fn smoke_scan_real_logs() {
        for r in [
            scan(),
            crate::local::cherry::scan(),
            crate::local::minimax::scan(),
        ] {
            println!(
                "id={} sessions={} projects={} days={} total={:.0} first={:?} last={:?}",
                r.id,
                r.session_count,
                r.project_count,
                r.daily.len(),
                r.total_tokens,
                r.daily.first().map(|d| &d.date),
                r.daily.last().map(|d| &d.date),
            );
            for d in r.daily.iter().rev().take(3) {
                println!("  {} total={:.0}", d.date, d.total);
            }
            for mu in r.models.iter().take(4) {
                println!(
                    "  model={} total={:.0} cost={:.5} {}",
                    mu.model, mu.total_tokens, mu.cost, mu.currency
                );
            }
        }
    }
}