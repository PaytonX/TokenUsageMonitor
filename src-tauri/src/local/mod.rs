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
pub mod dsh;
pub mod hermes;
pub mod minimax;
pub mod roots;
pub mod watch;
pub mod wsl;
pub mod zcode;

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

/// 「经 TokenRouter」的哨兵模型名。**前端 `src/lib/router-attribution.ts` 有一份
/// 镜像常量**，改这里必须同步改那边。
///
/// 工具侧只知道自己请求了一个叫「路由」的东西，看不到实际落到哪个上游模型
/// （路由不改写客户端可见的模型名）。于是工具把这行打成哨兵，具体模型由
/// TokenRouter 自己的记账行（`kind='router'`，`model` 为实际承载模型）给出，
/// 前端按日做**替换**归因，而不是靠前缀匹配猜。
pub const ROUTED_MODEL: &str = "__via_router__";

/// 判定某个模型名是否为「经本地路由」的可验证占位符。
///
/// 判据必须是**具体的占位符**，不能拿"模型名为空"当路由信号：
/// - MiniMax Code 把自定义 Provider 记成 `custom_provider:<id>/<variant>`
/// - Codex 把指向本地路由的模型记成 `LocalRouter`
///
/// 空模型名是「上游没记模型」的未知态（见 [`UNCLASSIFIED_MODEL`]），与「走了
/// 路由」是两回事，混用会在用户把工具切回直连时把全部流量误标成经路由。
pub fn is_routed_placeholder(model: &str) -> bool {
    let m = model.trim();
    m.starts_with("custom_provider:") || m == "LocalRouter"
}

/// Scan every supported local tool in parallel (each tool owns independent I/O),
/// then drop tools with no usage. Used by `get_local_tools` and the startup
/// pre-warm so the first open of the tools view is instant.
pub fn scan_all(storage: &crate::storage::Storage) -> Vec<LocalToolReport> {
    let mut tools: Vec<LocalToolReport> = Vec::with_capacity(7);
    std::thread::scope(|s| {
        let claude = s.spawn(claude::scan);
        let cherry = s.spawn(cherry::scan);
        let minimax = s.spawn(minimax::scan);
        let codex = s.spawn(codex::scan);
        let dsh = s.spawn(dsh::scan);
        let hermes = s.spawn(move || hermes::scan(storage));
        let zcode = s.spawn(zcode::scan);
        tools.push(claude.join().unwrap_or_default());
        tools.push(cherry.join().unwrap_or_default());
        tools.push(minimax.join().unwrap_or_default());
        tools.push(codex.join().unwrap_or_default());
        tools.push(dsh.join().unwrap_or_default());
        tools.push(hermes.join().unwrap_or_default());
        tools.push(zcode.join().unwrap_or_default());
    });
    // 保留规则：有近 90 天数据的工具一律保留（它确实在用）；无数据的才要求
    // "已安装且近期仍活跃"——只看目录存在会把早已弃用的工具（如 Codex，其
    // .codex/sessions 永久留着历史会话）长期挂在面板上，毫无意义。
    tools.retain(|t| !t.daily.is_empty() || cache::tool_recently_active(&t.id));
    tools
}

/// 受支持工具 id 列表（供 watch 增量扫描与前端工具胶囊使用）。
pub const TOOL_IDS: [&str; 7] = [
    "claude-code",
    "cherry-studio",
    "minimax-code",
    "codex",
    "hermes",
    "deepseek-harness",
    "zcode",
];

/// 单个工具的一次扫描（watch 用到：只重扫变化的那一个）。
pub fn scan_tool(tool_id: &str, storage: &crate::storage::Storage) -> Option<LocalToolReport> {
    let report = match tool_id {
        "claude-code" => claude::scan(),
        "cherry-studio" => cherry::scan(),
        "minimax-code" => minimax::scan(),
        "codex" => codex::scan(),
        "hermes" => hermes::scan(storage),
        "deepseek-harness" => dsh::scan(),
        "zcode" => zcode::scan(),
        _ => return None,
    };
    // 与 scan_all 同口径：有数据即保留；无数据则要求"已安装且近期仍活跃"，
    // 否则从缓存移除该工具（watch 增量路径据此把弃用工具清出面板）。
    if report.daily.is_empty()
        && report.total_tokens <= 0.0
        && !cache::tool_recently_active(tool_id)
    {
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

/// 把全部本机工具的逐日用量（含分模型）写入统一账本 `usage_daily`。
/// 全量回放语义（逐行 REPLACE 幂等）：重扫两次结果不变，滚动窗口外的过期日
/// 由 REPLACE 前的按源删除自然清掉。模型成本按日 total 占比分摊，使账本
/// 重聚合后的成本合计与工具上报一致；`cost_estimated` 随模型整体口径传递。
pub fn persist_all_tools(storage: &crate::storage::Storage, tools: &[LocalToolReport]) {
    let mut rows: Vec<crate::storage::UsageDailyRow> = Vec::new();
    for tool in tools {
        for d in &tool.daily {
            rows.push(crate::storage::UsageDailyRow {
                source: tool.id.clone(),
                kind: "tool".to_string(),
                date: d.date.clone(),
                model: String::new(),
                input: d.input,
                cache_read: d.cache_read,
                output: d.output,
                total: d.total,
                unit: crate::providers::UsageUnit::Tokens,
                cost: None,
                currency: None,
                cost_estimated: false,
            });
        }
        for m in &tool.models {
            // 分摊基数用模型自身的 total_tokens（与 cost 的统计窗口一致）。
            let per_token = if m.total_tokens > 0.0 { m.cost / m.total_tokens } else { 0.0 };
            let has_cost = m.cost > 0.0;
            for d in &m.daily {
                rows.push(crate::storage::UsageDailyRow {
                    source: tool.id.clone(),
                    kind: "tool".to_string(),
                    date: d.date.clone(),
                    model: m.model.clone(),
                    input: d.input,
                    cache_read: d.cache_read,
                    output: d.output,
                    total: d.total,
                    unit: crate::providers::UsageUnit::Tokens,
                    cost: has_cost.then(|| d.total * per_token),
                    currency: (!m.currency.is_empty()).then_some(m.currency.clone()),
                    cost_estimated: m.cost_estimated,
                });
            }
        }
    }
    if let Err(e) = storage.replace_usage_daily(&rows) {
        eprintln!("[local] persist_all_tools failed: {e}");
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

    /// 路由占位符只认**具体形态**，不认「模型名为空」——空名是「上游没记模型」
    /// 的未知态。用户把工具切回直连时，若靠空名判路由，全部直连流量都会被
    /// 误标成经路由。
    #[test]
    fn routed_placeholder_only_matches_known_forms() {
        assert!(is_routed_placeholder("custom_provider:localrouter/auto"));
        assert!(is_routed_placeholder("custom_provider:openrouter/stealth/x"));
        assert!(is_routed_placeholder("LocalRouter"));
        assert!(is_routed_placeholder("  LocalRouter  "));
        // 未知态，不是路由
        assert!(!is_routed_placeholder(""));
        assert!(!is_routed_placeholder(UNCLASSIFIED_MODEL));
        assert!(!is_routed_placeholder("minimax/MiniMax-M3"));
        assert!(!is_routed_placeholder("gpt-5"));
    }

    #[test]
    fn routed_sentinel_is_distinct_from_unclassified() {
        // 两者语义不同：前者「知道走了路由、模型待替换」，后者「不知道模型」。
        // 混用会让未知态被错误地拿去做替换归因。
        assert_ne!(ROUTED_MODEL, UNCLASSIFIED_MODEL);
        assert!(!ROUTED_MODEL.contains(UNCLASSIFIED_MODEL));
    }

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

    fn tool_report(id: &str, daily: Vec<LocalDay>) -> LocalToolReport {
        let total_tokens = daily.iter().map(|d| d.total).sum();
        LocalToolReport {
            id: id.into(),
            name: id.into(),
            daily,
            total_tokens,
            session_count: 0,
            project_count: 0,
            scanned_at: String::new(),
            models: Vec::new(),
        }
    }

    fn ledger_rows(storage: &Storage, source: &str) -> Vec<crate::storage::UsageDailyRow> {
        storage
            .load_usage_daily(None, 400)
            .unwrap()
            .into_iter()
            .filter(|r| r.source == source)
            .collect()
    }

    /// 空输入不写任何行。
    #[test]
    fn persist_all_tools_empty_input_writes_nothing() {
        let (db, storage) = temp_db("agg-empty");
        persist_all_tools(&storage, &[]);
        assert!(storage.load_usage_daily(None, 400).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    /// 全部工具入账本：来源总量行 + 分模型行，REPLACE 幂等（重扫两次值不变）。
    #[test]
    fn persist_all_tools_writes_every_tool_idempotently() {
        let (db, storage) = temp_db("agg-write");
        let mut minimax = tool_report(
            "minimax-code",
            vec![
                LocalDay { date: "2026-01-02".into(), input: 60.0, cache_read: 30.0, output: 10.0, total: 100.0 },
                LocalDay { date: "2026-01-03".into(), input: 150.0, cache_read: 75.0, output: 25.0, total: 250.0 },
            ],
        );
        minimax.models.push(LocalModelUsage {
            model: "MiniMax-M3".into(),
            total_tokens: 350.0,
            cost: 3.5,
            currency: "USD".into(),
            cost_estimated: true,
            daily: vec![
                LocalDay { date: "2026-01-02".into(), input: 60.0, cache_read: 30.0, output: 10.0, total: 100.0 },
                LocalDay { date: "2026-01-03".into(), input: 150.0, cache_read: 75.0, output: 25.0, total: 250.0 },
            ],
        });
        let cherry = tool_report("cherry-studio", vec![
            LocalDay { date: "2026-01-02".into(), input: 999.0, cache_read: 0.0, output: 0.0, total: 999.0 },
        ]);

        persist_all_tools(&storage, &[minimax.clone(), cherry.clone()]);
        let first = ledger_rows(&storage, "minimax-code");
        // 总量行 2 + 模型行 2
        assert_eq!(first.len(), 4);
        assert_eq!(first.iter().filter(|r| r.model.is_empty()).count(), 2);

        // 重扫幂等：行数与数值不变。
        persist_all_tools(&storage, &[minimax, cherry]);
        let second = ledger_rows(&storage, "minimax-code");
        assert_eq!(first.len(), second.len());
        assert_eq!(
            first.iter().map(|r| (r.date.clone(), r.total)).collect::<Vec<_>>(),
            second.iter().map(|r| (r.date.clone(), r.total)).collect::<Vec<_>>(),
        );

        // cherry 也入账本（旧 persist_minimax 不写非 minimax 工具）。
        assert_eq!(ledger_rows(&storage, "cherry-studio").len(), 1);
        let _ = std::fs::remove_file(&db);
    }

    /// 模型行的成本按日占比分摊：重聚合后与模型上报的成本合计一致。
    #[test]
    fn model_cost_is_distributed_proportionally() {
        let (db, storage) = temp_db("agg-cost");
        let mut m = tool_report("zcode", vec![
            LocalDay { date: "2026-01-02".into(), input: 0.0, cache_read: 0.0, output: 0.0, total: 100.0 },
            LocalDay { date: "2026-01-03".into(), input: 0.0, cache_read: 0.0, output: 0.0, total: 300.0 },
        ]);
        m.models.push(LocalModelUsage {
            model: "GLM-5.3".into(),
            total_tokens: 400.0,
            cost: 4.0,
            currency: "USD".into(),
            cost_estimated: true,
            daily: vec![
                LocalDay { date: "2026-01-02".into(), input: 0.0, cache_read: 0.0, output: 0.0, total: 100.0 },
                LocalDay { date: "2026-01-03".into(), input: 0.0, cache_read: 0.0, output: 0.0, total: 300.0 },
            ],
        });
        persist_all_tools(&storage, &[m]);
        let model_rows: Vec<_> = ledger_rows(&storage, "zcode")
            .into_iter()
            .filter(|r| r.model == "GLM-5.3")
            .collect();
        assert_eq!(model_rows.len(), 2);
        let cost_sum: f64 = model_rows.iter().filter_map(|r| r.cost).sum();
        assert!((cost_sum - 4.0).abs() < 1e-9, "cost must sum to reported total");
        assert!(model_rows.iter().all(|r| r.cost_estimated));
        let _ = std::fs::remove_file(&db);
    }
}