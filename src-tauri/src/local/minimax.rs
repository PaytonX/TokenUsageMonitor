//! MiniMax Code (opencode-based) local usage scanner.
//!
//! MiniMax Code keeps a per-turn token ledger in SQLite. Current data lives in
//! `~/.minimax/v2/sqlite/runtime-state.sqlite` (`local_runtime_token_usage`,
//! post-v2 migration), with a legacy copy in `~/.minimax/sqlite.db`
//! (`token_usage`). Both share columns `ts` (epoch-ms), `input_tokens`,
//! `output_tokens`, `reasoning_tokens`, `cache_read_tokens`,
//! `cache_write_tokens`, `cost_usd`, `session_id`. We aggregate from the v2 DB
//! first, then fold in the legacy DB (their date ranges are consecutive, so
//! merging doesn't double count).
//!
//! All DBs are opened read-only; missing/locked files simply yield an empty
//! report.

use super::{day_from_epoch_ms, finalize_days, finalize_models, LocalToolReport};
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

const TOOL_ID: &str = "minimax-code";
const TOOL_NAME: &str = "MiniMax Code";
const KEEP_DAYS: usize = 90;

const QRY_V2: &str =
    "SELECT ts, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, model, cost_usd, session_id \
     FROM local_runtime_token_usage";
const QRY_LEGACY: &str =
    "SELECT ts, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, model, cost_usd, session_id \
     FROM token_usage";

struct Agg {
    by_day: BTreeMap<String, (f64, f64, f64)>,
    by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
    model_cost: BTreeMap<String, (f64, String)>,
    sessions: HashSet<String>,
    rows: u64,
}

/// `.minimax` 数据目录。走 roots 解析：默认 `~`、用户声明的额外数据根目录、
/// 精确覆盖三者取**最近活跃**的那个——用户搬迁数据目录后旧位置常残留停更
/// 副本，只认 `~` 会让最近几天用量读成 0（见 `local::roots`）。
fn data_dir() -> Option<PathBuf> {
    super::roots::pick_freshest(&super::roots::windows_candidates(".minimax", ".minimax"))
}

fn v2_db() -> PathBuf {
    data_dir()
        .map(|d| d.join("v2").join("sqlite").join("runtime-state.sqlite"))
        .unwrap_or_default()
}
fn legacy_db() -> PathBuf {
    data_dir()
        .map(|d| d.join("sqlite.db"))
        .unwrap_or_default()
}

fn open_ro(path: &PathBuf) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    // A running MiniMax Code can hold the DB for a checkpoint, letting us
    // *open* read-only yet return Busy on the first query. Verify readability,
    // and fall back to an immutable view so a locked window never yields an
    // empty scan (which would silently drop the tool from the models view).
    if let Some(conn) = Connection::open_with_flags(path, flags).ok() {
        let _ = conn.busy_timeout(std::time::Duration::from_millis(250));
        let readable = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .is_ok();
        if readable {
            return Some(conn);
        }
        drop(conn);
    }
    let url = format!("file:{}?immutable=1", path.to_string_lossy());
    Connection::open_with_flags(&url, flags).ok()
}

fn scan_table(conn: &Connection, qry: &str, agg: &mut Agg) {
    let Ok(mut stmt) = conn.prepare(qry) else {
        return;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return;
    };
    while let Ok(Some(row)) = rows.next() {
        let ms: i64 = row.get(0).unwrap_or_default();
        let input: f64 = row.get(1).unwrap_or_default();
        let output: f64 = row.get(2).unwrap_or_default();
        let cache_read: f64 = row.get(3).unwrap_or_default();
        let cache_write: f64 = row.get(4).unwrap_or_default();
        let model: Option<String> = row.get(5).unwrap_or_default();
        let cost: f64 = row.get(6).unwrap_or_default();
        let session: String = row.get(7).unwrap_or_default();
        let date = day_from_epoch_ms(ms);
        if date.is_empty() {
            continue;
        }
        agg.rows += 1;
        if !session.is_empty() {
            agg.sessions.insert(session);
        }
        let e = agg.by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += input;
        e.1 += cache_read + cache_write;
        e.2 += output;
        // 个别行源里没有模型（`model`/`raw` 均为空）——仍把 token 计入共享的
        // "未标记模型" 桶，使模型页合计与趋势页(按日总量)对得上，且不伪造模型名。
        let model = model
            .filter(|m| !m.trim().is_empty())
            .map(|m| m.trim().to_string())
            .unwrap_or_else(|| crate::local::UNCLASSIFIED_MODEL.to_string());
        let m = agg.by_model.entry(model.clone()).or_default();
        let me = m.entry(date).or_insert((0.0, 0.0, 0.0));
        me.0 += input;
        me.1 += cache_read + cache_write;
        me.2 += output;
        if model != crate::local::UNCLASSIFIED_MODEL {
            let c = agg.model_cost.entry(model).or_insert((0.0, "USD".to_string()));
            c.0 += cost;
        }
    }
}

/// Scan MiniMax Code's token ledger, merging v2 (primary) with legacy data.
pub fn scan() -> LocalToolReport {
    let mut agg = Agg {
        by_day: BTreeMap::new(),
        by_model: BTreeMap::new(),
        model_cost: BTreeMap::new(),
        sessions: HashSet::new(),
        rows: 0,
    };

    if let Some(conn) = open_ro(&v2_db()) {
        scan_table(&conn, QRY_V2, &mut agg);
    }
    if let Some(conn) = open_ro(&legacy_db()) {
        scan_table(&conn, QRY_LEGACY, &mut agg);
    }

    let daily = finalize_days(agg.by_day.clone(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    let models = finalize_models(agg.by_model, &agg.model_cost, KEEP_DAYS);
    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count: agg.sessions.len().max(1) as u64,
        project_count: 0,
        scanned_at: chrono::Utc::now().to_rfc3339(),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn rows_without_model_bucket_into_unclassified() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE local_runtime_token_usage (
                id INTEGER, session_id TEXT, agent_name TEXT, framework_type TEXT, turn_id TEXT,
                model TEXT, ts INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                reasoning_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                cost_usd REAL, raw TEXT
            );",
        )
        .unwrap();
        // 3 rows: NULL model, empty-string model, and a named model.
        conn.execute(
            "INSERT INTO local_runtime_token_usage
             (model, ts, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd)
             VALUES (?1, 1776500585000, 100, 10, 0, 0, 0), (?2, 1776500585000, 200, 20, 0, 0, 0),
             ('minimax/MiniMax-M3', 1776500585000, 400, 40, 0, 0, 0.5)",
            rusqlite::params![rusqlite::types::Null, ""],
        )
        .unwrap();

        let mut agg = Agg {
            by_day: BTreeMap::new(),
            by_model: BTreeMap::new(),
            model_cost: BTreeMap::new(),
            sessions: HashSet::new(),
            rows: 0,
        };
        scan_table(&conn, QRY_V2, &mut agg);

        let unclassified = agg.by_model.get(crate::local::UNCLASSIFIED_MODEL).unwrap();
        let day = unclassified.values().next().unwrap();
        // NULL + empty model rows both fold into the unclassified bucket.
        assert_eq!(day.0, 300.0);
        assert_eq!(day.2, 30.0);
        assert!(agg.by_model.contains_key("minimax/MiniMax-M3"));
        // Named row keeps its cost; unclassified carries none in model_cost.
        assert_eq!(agg.model_cost["minimax/MiniMax-M3"].0, 0.5);
        assert!(!agg.model_cost.contains_key(crate::local::UNCLASSIFIED_MODEL));
        // Daily total reconciles all three rows.
        let total: f64 = agg.by_day.values().map(|d| d.0 + d.1 + d.2).sum();
        assert_eq!(total, 100.0 + 10.0 + 200.0 + 20.0 + 400.0 + 40.0);
    }
}