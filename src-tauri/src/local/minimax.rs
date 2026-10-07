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
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// `local_runtime_sessions.extra_data_json` 里与模型归因相关的字段。上游运行时
/// 始终记录本会话生效的模型名，这里只取这一个字段，其余一律容忍。
#[derive(Debug, Clone, Deserialize, Default)]
struct SessionExtra {
    #[serde(default, rename = "effectiveModel")]
    effective_model: String,
}

/// 建立 `session_id → effectiveModel` 映射。
///
/// 存在的理由：上游 2026-08 起不再往 token 账本写 `model`（实测近 10 天
/// 13331/13516 行为空，`raw` 里也只有 token/cost、没有模型字段），但会话记录
/// 里的 `effectiveModel` 一直在——按 `session_id` 关联即可恢复出绝大多数行的
/// 模型名。旧版库没有 `local_runtime_sessions` 表（prepare 失败即返回空表），
/// 此时行为与修复前完全一致。
///
/// 口径与 Hermes 扫描器一致（`hermes.rs` 同样按会话级 `model` 归属）：粒度是
/// 会话而非单次请求，一个会话中途换模型时，其全部 token 归到会话最终的模型。
fn read_session_models(conn: &Connection) -> HashMap<String, String> {
    let Ok(mut stmt) = conn.prepare("SELECT session_id, extra_data_json FROM local_runtime_sessions")
    else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
    else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (sid, extra) in rows.flatten() {
        let Some(extra) = extra else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<SessionExtra>(&extra) else {
            continue;
        };
        let m = parsed.effective_model.trim();
        if !m.is_empty() {
            out.insert(sid, m.to_string());
        }
    }
    out
}

fn scan_table(conn: &Connection, qry: &str, sessions: &HashMap<String, String>, agg: &mut Agg) {
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
        // 解析模型名的优先级：账本自身的 `model` 列 > 会话的 `effectiveModel` >
        // 共享的"未标记模型"桶。账本列优先——它是逐请求的真实取值，优于会话
        // 级推断；两者都拿不到才落未标记桶，使按模型合计仍与按日总量对账。
        // 必须排在下面的 `sessions.insert` 之前：insert 会 move 掉 `session`。
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .or_else(|| sessions.get(&session).cloned())
            .unwrap_or_else(|| crate::local::UNCLASSIFIED_MODEL.to_string());
        // 走了本地路由的行打成哨兵：MiniMax Code 只能看到 `custom_provider:*`
        // 这个自定义 Provider 占位名，真实模型由路由自记账给出。
        let model = if crate::local::is_routed_placeholder(&model) {
            crate::local::ROUTED_MODEL.to_string()
        } else {
            model
        };
        if !session.is_empty() {
            agg.sessions.insert(session);
        }
        let e = agg.by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += input;
        e.1 += cache_read + cache_write;
        e.2 += output;
        let m = agg.by_model.entry(model.clone()).or_default();
        let me = m.entry(date).or_insert((0.0, 0.0, 0.0));
        me.0 += input;
        me.1 += cache_read + cache_write;
        me.2 += output;
        // 哨兵与未标记都记不了成本（前者真实模型在路由侧，后者本就未知），
        // 不能拿价目表按占位名估算——那会造出一个不存在的价格。
        if model != crate::local::UNCLASSIFIED_MODEL && model != crate::local::ROUTED_MODEL {
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
        // 会话表与账本表同库，一次性建好映射再扫账本。
        let sessions = read_session_models(&conn);
        scan_table(&conn, QRY_V2, &sessions, &mut agg);
    }
    if let Some(conn) = open_ro(&legacy_db()) {
        // 旧版库无会话表，模型只能取自账本自身的 `model` 列。
        scan_table(&conn, QRY_LEGACY, &HashMap::new(), &mut agg);
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
        scan_table(&conn, QRY_V2, &HashMap::new(), &mut agg);

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

    #[test]
    fn rows_without_ledger_model_recover_from_session() {
        // 上游 2026-08 起账本不再写 model，但会话的 effectiveModel 仍在，
        // 按 session_id 关联即可恢复；只有关联不上的才落未标记桶。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE local_runtime_sessions (
                session_id TEXT, extra_data_json TEXT
            );
            CREATE TABLE local_runtime_token_usage (
                id INTEGER, session_id TEXT, agent_name TEXT, framework_type TEXT, turn_id TEXT,
                model TEXT, ts INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                reasoning_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                cost_usd REAL, raw TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_sessions (session_id, extra_data_json)
             VALUES ('s1', '{\"effectiveModel\":\"minimax/MiniMax-M3.1-Flash-Preview\"}'),
                    ('s2', '{\"appMode\":\"coding\"}'),
                    ('s4', 'not json at all')",
            [],
        )
        .unwrap();
        // s1 两行模型列为空但可从会话恢复；s2 无 effectiveModel；s3 会话已消失；
        // s9 账本自带模型名。
        conn.execute(
            "INSERT INTO local_runtime_token_usage
             (session_id, model, ts, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd)
             VALUES ('s1', NULL, 1776500585000, 100, 10, 0, 0, 0.4),
                    ('s1', '', 1776500585000, 200, 20, 0, 0, 0.6),
                    ('s2', NULL, 1776500585000, 300, 30, 0, 0, 0.1),
                    ('s3', NULL, 1776500585000, 400, 40, 0, 0, 0.2),
                    ('s9', 'minimax/MiniMax-M3', 1776500585000, 500, 50, 0, 0, 0.5)",
            [],
        )
        .unwrap();

        let sessions = read_session_models(&conn);
        assert_eq!(
            sessions.get("s1").map(String::as_str),
            Some("minimax/MiniMax-M3.1-Flash-Preview")
        );
        // 无 effectiveModel / JSON 损坏的会话都不入表
        assert!(!sessions.contains_key("s2"));
        assert!(!sessions.contains_key("s4"));

        let mut agg = Agg {
            by_day: BTreeMap::new(),
            by_model: BTreeMap::new(),
            model_cost: BTreeMap::new(),
            sessions: HashSet::new(),
            rows: 0,
        };
        scan_table(&conn, QRY_V2, &sessions, &mut agg);

        // s1 的两行（NULL + 空串）都恢复成会话的模型名。
        let recovered = agg
            .by_model
            .get("minimax/MiniMax-M3.1-Flash-Preview")
            .expect("session model recovered");
        let day = recovered.values().next().unwrap();
        assert_eq!(day.0, 300.0);
        assert_eq!(day.2, 30.0);
        // 账本自带模型名的行不受会话覆盖。
        assert!(agg.by_model.contains_key("minimax/MiniMax-M3"));
        // 只有 s2（无 effectiveModel）+ s3（会话消失）落未标记桶。
        let un = agg
            .by_model
            .get(crate::local::UNCLASSIFIED_MODEL)
            .expect("unclassified bucket present");
        assert_eq!(un.values().next().unwrap().0, 700.0);
        // 恢复出的模型要记成本（s1 两行 0.4+0.6），未标记桶仍不记。
        assert_eq!(agg.model_cost["minimax/MiniMax-M3.1-Flash-Preview"].0, 1.0);
        assert!(!agg.model_cost.contains_key(crate::local::UNCLASSIFIED_MODEL));
        // 按日总量与按模型合计仍然对账：input 1500 + output 150。
        let total: f64 = agg.by_day.values().map(|d| d.0 + d.1 + d.2).sum();
        assert_eq!(total, 1650.0);
    }

    #[test]
    fn custom_provider_model_becomes_routed_sentinel() {
        // MiniMax Code 走 TokenRouter 时，会话 effectiveModel 记的是自定义
        // Provider 占位名；这类行须打成哨兵，交由路由自记账归因真实模型，
        // 而不是当成一个叫 "custom_provider:..." 的模型。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE local_runtime_sessions (session_id TEXT, extra_data_json TEXT);
             CREATE TABLE local_runtime_token_usage (
                id INTEGER, session_id TEXT, agent_name TEXT, framework_type TEXT, turn_id TEXT,
                model TEXT, ts INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                reasoning_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                cost_usd REAL, raw TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_sessions (session_id, extra_data_json)
             VALUES ('s1', '{\"effectiveModel\":\"custom_provider:localrouter/auto\"}'),
                    ('s2', '{\"effectiveModel\":\"minimax/MiniMax-M3\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_token_usage
             (session_id, model, ts, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd)
             VALUES ('s1', NULL, 1776500585000, 100, 10, 0, 0, 1.5),
                    ('s2', NULL, 1776500585000, 200, 20, 0, 0, 0.5)",
            [],
        )
        .unwrap();

        let sessions = read_session_models(&conn);
        let mut agg = Agg {
            by_day: BTreeMap::new(),
            by_model: BTreeMap::new(),
            model_cost: BTreeMap::new(),
            sessions: HashSet::new(),
            rows: 0,
        };
        scan_table(&conn, QRY_V2, &sessions, &mut agg);

        assert!(
            agg.by_model.contains_key(crate::local::ROUTED_MODEL),
            "custom_provider 行须打成哨兵，实际键: {:?}",
            agg.by_model.keys().collect::<Vec<_>>()
        );
        assert!(agg.by_model.contains_key("minimax/MiniMax-M3"));
        // 哨兵记不了成本：按占位名估算会造出一个不存在的价格。
        assert!(!agg.model_cost.contains_key(crate::local::ROUTED_MODEL));
        assert_eq!(agg.model_cost["minimax/MiniMax-M3"].0, 0.5);
    }
}