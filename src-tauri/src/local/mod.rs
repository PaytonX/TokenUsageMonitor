//! Local AI tools usage collection (B7 / C8).
//!
//! A second, parallel data channel to the live API polling: instead of querying
//! a provider's balance endpoint, we parse the **local session logs** of AI
//! coding tools and aggregate token usage by day. This first slice targets
//! **Claude Code** (its JSONL logs under `~/.claude/projects/` are stable and
//! carry a per-`assistant`-message token breakdown); more tools can be added
//! behind the same [`LocalToolReport`] shape.
//!
//! Design keeps with the "don't guess" rule: only fields real logs expose are
//! read, and each tool scans independently so a missing/odd tool never breaks
//! the others.

pub mod cache;
pub mod cherry;
pub mod claude;
pub mod codex;
pub mod delta;
pub mod hermes;
pub mod minimax;
pub mod watch;
pub mod wsl;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Model label for token rows whose source logged usage **without any model
/// id** (empty/NULL model, and no recoverable name in the raw payload). These
/// tokens are real consumption but have no model attribution; we surface them
/// under this stable bucket so the per-model view reconciles with the per-day
/// total instead of silently dropping them, without guessing a model.
pub const UNCLASSIFIED_MODEL: &str = "未标记模型";

/// Scan every supported local tool in parallel (each tool owns independent I/O),
/// then drop tools with no usage. Used by `get_local_tools` and the startup
/// pre-warm so the first open of the tools view is instant.
pub fn scan_all(storage: &crate::storage::Storage) -> Vec<LocalToolReport> {
    let mut tools: Vec<LocalToolReport> = Vec::with_capacity(5);
    std::thread::scope(|s| {
        let claude = s.spawn(claude::scan);
        let cherry = s.spawn(cherry::scan);
        let minimax = s.spawn(minimax::scan);
        let codex = s.spawn(codex::scan);
        let hermes = s.spawn(move || hermes::scan(storage));
        tools.push(claude.join().unwrap_or_default());
        tools.push(cherry.join().unwrap_or_default());
        tools.push(minimax.join().unwrap_or_default());
        tools.push(codex.join().unwrap_or_default());
        tools.push(hermes.join().unwrap_or_default());
    });
    // 保留有近 90 天数据的工具；无数据的则仅当"已安装"（存在日志源）时保留，
    // 让未装工具消失、已装但近期未用量的工具（如 Codex）仍出现在面板。
    tools.retain(|t| !t.daily.is_empty() || cache::tool_installed(&t.id));
    tools
}

/// 受支持工具 id 列表（供 watch 增量扫描与前端工具胶囊使用）。
pub const TOOL_IDS: [&str; 5] = ["claude-code", "cherry-studio", "minimax-code", "codex", "hermes"];

/// 单个工具的一次扫描（watch 用到：只重扫变化的那一个）。
pub fn scan_tool(tool_id: &str, storage: &crate::storage::Storage) -> Option<LocalToolReport> {
    let report = match tool_id {
        "claude-code" => claude::scan(),
        "cherry-studio" => cherry::scan(),
        "minimax-code" => minimax::scan(),
        "codex" => codex::scan(),
        "hermes" => hermes::scan(storage),
        _ => return None,
    };
    // 与 scan_all 同口径：无数据时仅当"已安装"才保留（否则从缓存移除该工具）。
    if report.daily.is_empty() && report.total_tokens <= 0.0 && !cache::tool_installed(tool_id) {
        return None;
    }
    Some(report)
}

/// One day of aggregated token usage for a tool. `date` is `YYYY-MM-DD` in the
/// user's local timezone.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalDay {
    pub date: String,
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
    pub total: f64,
}

impl LocalDay {
    /// 累加三项 token 并刷新 `total`。`pub(crate)` 供 delta 增量合并使用。
    pub(crate) fn add(&mut self, input: f64, cache_read: f64, output: f64) {
        self.input += input;
        self.cache_read += cache_read;
        self.output += output;
        self.total += input + cache_read + output;
    }
}

/// Per-model usage for a tool (aggregated across days + per-day series).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalModelUsage {
    pub model: String,
    pub total_tokens: f64,
    /// Provider-reported cost when the tool records it (0 if unknown).
    pub cost: f64,
    pub currency: String,
    /// True when `cost` was estimated from the pricing table (not provider-reported).
    pub cost_estimated: bool,
    /// Per-day token series (ascending by date).
    pub daily: Vec<LocalDay>,
}

/// Aggregated usage for one discovered local tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalToolReport {
    pub id: String,
    pub name: String,
    /// Per-day totals, ascending by date.
    pub daily: Vec<LocalDay>,
    pub total_tokens: f64,
    pub session_count: u64,
    pub project_count: u64,
    pub scanned_at: String,
    /// Per-model breakdown (drives the models view).
    pub models: Vec<LocalModelUsage>,
}

/// Payload returned to the UI: a list of discovered tools.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalToolsPayload {
    pub tools: Vec<LocalToolReport>,
    /// Number of session files parsed across all tools.
    pub sessions_parsed: u64,
}

/// In-memory cache so repeated `get_local_tools` calls don't re-parse hundreds
/// of log files on every UI poll. Held in `AppState`.
pub struct LocalCache {
    inner: Mutex<Option<(Instant, LocalToolsPayload)>>,
}

impl LocalCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Return the cached payload if one exists.
    ///
    /// No time-based expiry: the tool watcher continuously refreshes each tool's
    /// report whenever its logs change and emits `tools-updated`, so an existing
    /// cache is effectively "live". This keeps switching tabs from falling back
    /// to a slow tree-walk + full scan on every view. `force=true` in
    /// `get_local_tools` still bypasses the cache for an explicit rescan.
    pub async fn cached(&self) -> Option<LocalToolsPayload> {
        let guard = self.inner.lock().await;
        guard.as_ref().map(|(_, payload)| payload.clone())
    }

    pub async fn store(&self, payload: LocalToolsPayload) {
        *self.inner.lock().await = Some((Instant::now(), payload));
    }
}

/// Aggregate per-day maps into an ascending `Vec<LocalDay>` for the last up-to
/// `keep_days` days. Helper shared by tool scanners.
pub fn finalize_days(
    by_day: BTreeMap<String, (f64, f64, f64)>,
    keep_days: usize,
) -> Vec<LocalDay> {
    // Drop days trailing the window by removing the oldest until within scope.
    let mut keys: Vec<String> = by_day.keys().cloned().collect();
    keys.sort();
    if keys.len() > keep_days {
        keys = keys[keys.len() - keep_days..].to_vec();
    }
    keys.into_iter()
        .map(|date| {
            let (input, cache_read, output) = by_day[&date];
            let mut day = LocalDay {
                date,
                ..Default::default()
            };
            day.add(input, cache_read, output);
            day
        })
        .collect()
}

/// Shared reference type stored in AppState.
pub type SharedLocalCache = Arc<LocalCache>;

/// Persist the MiniMax Code token daily series into the app SQLite under the
/// synthetic key `minimax-code`, so the MiniMax provider card's calendar
/// heatmap reads it fast (DB) instead of re-scanning the big ~/.minimax db.
pub fn persist_minimax(storage: &crate::storage::Storage, tools: &[LocalToolReport]) {
    if let Some(tool) = tools.iter().find(|t| t.id == "minimax-code") {
        for d in &tool.daily {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") {
                let _ = storage.record_daily_on(
                    "minimax-code",
                    date,
                    d.total,
                    crate::providers::UsageUnit::Tokens,
                );
            }
        }
    }
}

/// Finalize per-model usage maps into an ascending, kept-window `Vec`.
/// `by_model`: model -> (day -> (input, cache_read, output)).
/// `cost_by_model`: model -> (provider-reported cost, currency).
pub fn finalize_models(
    by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
    cost_by_model: &BTreeMap<String, (f64, String)>,
    keep_days: usize,
) -> Vec<LocalModelUsage> {
    let mut out: Vec<LocalModelUsage> = Vec::new();
    for (model, days) in by_model {
        let series = finalize_days(days, keep_days);
        let total_tokens: f64 = series.iter().map(|d| d.total).sum();
        if total_tokens <= 0.0 {
            continue;
        }
        let (cost, currency) = cost_by_model.get(&model).cloned().unwrap_or((0.0, String::new()));
        out.push(LocalModelUsage {
            model,
            total_tokens,
            cost,
            currency,
            cost_estimated: false,
            daily: series,
        });
    }
    out.sort_by(|a, b| b.total_tokens.partial_cmp(&a.total_tokens).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Convert an epoch-millisecond timestamp (as stored by Cherry Studio's
/// `ai_usage_record.created_at` and MiniMax's `token_usage.ts`) to a local
/// `YYYY-MM-DD` day key. Out-of-range / impossible values map to an empty
/// string so callers can skip them.
pub fn day_from_epoch_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => dt.with_timezone(&chrono::Local).date_naive().to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::collections::HashMap;

    fn temp_db(tag: &str) -> (std::path::PathBuf, Storage) {
        let db = std::env::temp_dir().join(format!("tum_{}_{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&db);
        let storage = Storage::open(&db).expect("open temp storage");
        (db, storage)
    }

    fn day(date: &str, total: f64) -> LocalDay {
        LocalDay {
            date: date.to_string(),
            total,
            ..Default::default()
        }
    }

    #[test]
    fn persist_minimax_ignores_non_minimax_and_empty_input() {
        let (db, storage) = temp_db("empty");
        // 空工具列表：不写任何行。
        persist_minimax(&storage, &[]);
        assert!(storage.load_heatmap("minimax-code", 30).unwrap().is_empty());

        // 非 minimax 工具：即使有 daily，也不该写入 minimax-code 键。
        let cherry = LocalToolReport {
            id: "cherry-studio".into(),
            daily: vec![day("2026-01-02", 999.0)],
            ..Default::default()
        };
        persist_minimax(&storage, &[cherry]);
        assert!(storage.load_heatmap("minimax-code", 30).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn persist_minimax_writes_minimax_code_days() {
        let (db, storage) = temp_db("writes");
        let minimax = LocalToolReport {
            id: "minimax-code".into(),
            daily: vec![day("2026-01-02", 100.0), day("2026-01-03", 250.0)],
            ..Default::default()
        };
        let cherry = LocalToolReport {
            id: "cherry-studio".into(),
            daily: vec![day("2026-01-02", 999.0)],
            ..Default::default()
        };
        persist_minimax(&storage, &[minimax, cherry]);

        let cells = storage.load_heatmap("minimax-code", 30).unwrap();
        let by_date: HashMap<String, f64> = cells
            .iter()
            .map(|c| (c.date.to_string(), c.value))
            .collect();
        assert_eq!(by_date.get("2026-01-02"), Some(&100.0));
        assert_eq!(by_date.get("2026-01-03"), Some(&250.0));
        // minimax-code 里不应混入 cherry 的 999。
        assert_eq!(by_date.values().any(|v| *v == 999.0), false);

        // 单元应标记为 tokens（供卡片热力图按 token 显示）。
        assert!(cells.iter().all(|c| c.unit == crate::providers::UsageUnit::Tokens));

        // cherry-studio 自身键保持为空（persist_minimax 不写它）。
        assert!(storage.load_heatmap("cherry-studio", 30).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }
}