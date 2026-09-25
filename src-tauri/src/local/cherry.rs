//! Cherry Studio local usage scanner.
//!
//! Cherry Studio keeps a per-request usage ledger in an SQLite DB:
//! `%APPDATA%\CherryStudio\Data\cherrystudio.sqlite`, table `ai_usage_record`
//! (columns `created_at` as epoch-ms, `input_tokens`, `output_tokens`,
//! `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens`,
//! `model_name`, ...). We aggregate tokens by local day into a
//! [`LocalToolReport`].
//!
//! The DB is opened read-only so an in-use Cherry Studio never locks/blocks us;
//! any open or schema error just yields an empty report.

use super::{day_from_epoch_ms, finalize_days, finalize_models, LocalToolReport};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::PathBuf;

const TOOL_ID: &str = "cherry-studio";
const TOOL_NAME: &str = "Cherry Studio";
const KEEP_DAYS: usize = 90;

const QRY: &str =
    "SELECT created_at, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, \
     model_name, cost, cost_currency \
     FROM ai_usage_record";

fn data_dir() -> PathBuf {
    // Prefer the roaming app-data location; `Local` varies by editor build.
    if let Ok(p) = std::env::var("APPDATA").map(PathBuf::from) {
        let p = p.join("CherryStudio").join("Data").join("cherrystudio.sqlite");
        if p.exists() {
            return p;
        }
    }
    std::env::var("LOCALAPPDATA")
        .map(|p| {
            PathBuf::from(p)
                .join("CherryStudio")
                .join("Data")
                .join("cherrystudio.sqlite")
        })
        .unwrap_or_default()
}

/// 从一个已打开的 Cherry Studio LEDGER 连接读取并聚合（供 `scan` 与测试复用）。
fn scan_conn(
    conn: &Connection,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
    model_cost: &mut BTreeMap<String, (f64, String)>,
    row_count: &mut u64,
) {
    if let Ok(mut stmt) = conn.prepare(QRY) {
        let mut rows = stmt.query([]);
        if let Ok(rows_mut) = rows.as_mut() {
            while let Ok(Some(row)) = rows_mut.next() {
                let ms: i64 = row.get(0).unwrap_or_default();
                let input: f64 = row.get(1).unwrap_or_default();
                let output: f64 = row.get(2).unwrap_or_default();
                let cache_read: f64 = row.get(3).unwrap_or_default();
                let cache_write: f64 = row.get(4).unwrap_or_default();
                let model: Option<String> = row.get(5).unwrap_or_default();
                let cost: f64 = row.get(6).unwrap_or_default();
                let currency: String = row.get(7).unwrap_or_default();
                let date = day_from_epoch_ms(ms);
                if date.is_empty() {
                    continue;
                }
                *row_count += 1;
                let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
                e.0 += input;
                e.1 += cache_read + cache_write;
                e.2 += output;
                // 与 claude/codex/minimax/hermes 同口径：无模型名的记录归入"未标记模型"
                // 桶，使模型页合计与趋势页(按日总量)对得上，而不伪造模型名。
                let model = model
                    .filter(|m| !m.trim().is_empty())
                    .map(|m| m.trim().to_string())
                    .unwrap_or_else(|| crate::local::UNCLASSIFIED_MODEL.to_string());
                let m = by_model.entry(model.clone()).or_default();
                let me = m.entry(date).or_insert((0.0, 0.0, 0.0));
                me.0 += input;
                me.1 += cache_read + cache_write;
                me.2 += output;
                if model != crate::local::UNCLASSIFIED_MODEL {
                    let c = model_cost.entry(model).or_insert((0.0, currency));
                    c.0 += cost;
                }
            }
        }
    }
}

/// Scan the Cherry Studio usage ledger, aggregating tokens by local day.
pub fn scan() -> LocalToolReport {
    let db = data_dir();
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut model_cost: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut row_count: u64 = 0;

    if let Ok(conn) = Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        scan_conn(&conn, &mut by_day, &mut by_model, &mut model_cost, &mut row_count);
    }

    let daily = finalize_days(by_day.clone(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    let models = finalize_models(by_model, &model_cost, KEEP_DAYS);
    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count: row_count,
        project_count: 0,
        scanned_at: chrono::Utc::now().to_rfc3339(),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_resolves_somewhere() {
        // On this machine it resolves to the real Cherry Studio DB; in CI it may
        // be empty, but it must never panic or return a poisoned path.
        let _ = data_dir();
    }

    #[test]
    fn rows_without_model_bucket_into_unclassified() {
        // NULL 与空字符串模型名的记录应归入"未标记模型"，使模型页与按日总量一致。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                created_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                model_name TEXT, cost REAL, cost_currency TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_usage_record
             (created_at, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, model_name, cost, cost_currency)
             VALUES (1776500585000, 100, 10, 0, 0, NULL, 0, 'USD'),
                    (1776500585000, 200, 20, 5, 1, '', 0, 'USD'),
                    (1776500585000, 400, 40, 0, 0, 'gpt-5', 0.5, 'USD')",
            [],
        )
        .unwrap();

        let mut by_day = BTreeMap::new();
        let mut by_model = BTreeMap::new();
        let mut model_cost = BTreeMap::new();
        let mut row_count = 0u64;
        scan_conn(&conn, &mut by_day, &mut by_model, &mut model_cost, &mut row_count);

        let unclassified = by_model
            .get(crate::local::UNCLASSIFIED_MODEL)
            .expect("unclassified bucket present");
        let day = unclassified.values().next().unwrap();
        // NULL + empty model 两行都归入未标记桶：input=300, cache=(5+1)=6, output=30
        assert_eq!(day.0, 300.0);
        assert_eq!(day.1, 6.0);
        assert_eq!(day.2, 30.0);
        assert!(by_model.contains_key("gpt-5"));
        // 命名模型记成本；未标记桶不记成本。
        assert_eq!(model_cost["gpt-5"].0, 0.5);
        assert!(!model_cost.contains_key(crate::local::UNCLASSIFIED_MODEL));
        // 趋势页按日总量 = 所有行之和（含未标记），与模型页合计一致。
        let total: f64 = by_day.values().map(|d| d.0 + d.1 + d.2).sum();
        assert_eq!(total, 100.0 + 10.0 + 200.0 + 20.0 + 5.0 + 1.0 + 400.0 + 40.0);
    }
}