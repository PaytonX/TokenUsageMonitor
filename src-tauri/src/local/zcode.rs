//! ZCode (`~/.zcode`) usage scanner — per-message attribution.
//!
//! ZCode keeps its conversation store in `~/.zcode/cli/db/db.sqlite` (WAL,
//! actively written by the running CLI). Each assistant row in `message`
//! carries a **per-request increment** (not a running total): `tokens.input`,
//! `tokens.output`, `tokens.reasoning`, `tokens.cache.read/write`, plus the
//! `modelId` and its own `time_created` epoch-ms timestamp.
//!
//! Because rows are increments, a scan is a plain idempotent `GROUP BY` —
//! no delta accounting (unlike Hermes, whose DB only stores cumulative
//! per-session totals). Re-scanning the same DB twice yields the same report.
//!
//! All-zero rows exist (aborted attempts observed ~6% in real data); they are
//! skipped exactly like the other scanners. `cache.read + cache.write` fold
//! into `cache_read` and `reasoning` folds into `output`, matching the
//! Claude Code / Hermes display口径 so per-model views stay comparable.

use super::{day_from_epoch_ms, finalize_days, finalize_models, LocalToolReport};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::BTreeMap;

const TOOL_ID: &str = "zcode";
const TOOL_NAME: &str = "ZCode";
const KEEP_DAYS: usize = 90;

/// ZCode 的消息库文件（Windows + 各 WSL 发行版 home 下的 `.zcode/cli/db/db.sqlite`）。
fn zcode_dbs() -> Vec<std::path::PathBuf> {
    super::wsl::existing_dotdirs(".zcode/cli/db")
        .into_iter()
        .map(|d| d.join("db.sqlite"))
        .filter(|p| p.exists())
        .collect()
}

/// 与 hermes 相同的只读打开策略：WAL 活跃时先试 read-only（带 250ms busy
/// 超时），首查仍被写锁顶开则回退 immutable 快照（best-effort，可能少读
/// 未 checkpoint 的 WAL，但绝不阻塞、绝不报错）。
fn open_reader(db_path: &std::path::Path) -> Option<Connection> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    if let Ok(conn) = Connection::open_with_flags(db_path, flags) {
        let _ = conn.busy_timeout(std::time::Duration::from_millis(250));
        let readable = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .is_ok();
        if readable {
            return Some(conn);
        }
        drop(conn);
    }
    let url = format!("file:{}?immutable=1", db_path.to_string_lossy());
    Connection::open_with_flags(&url, flags).ok()
}

/// `message.data` JSON 中与本扫描相关的字段。ZCode 的列名是 camelCase
/// （`modelId`），serde 逐字段改名；缺失字段一律容忍（版本漂移安全）。
#[derive(Debug, Clone, Deserialize, Default)]
struct MessageData {
    #[serde(default, rename = "modelId")]
    model_id: String,
    #[serde(default)]
    tokens: Tokens,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Tokens {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    reasoning: f64,
    #[serde(default)]
    cache: Cache,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Cache {
    #[serde(default)]
    read: f64,
    #[serde(default)]
    write: f64,
}

/// 一条 assistant 消息的已拆分读数（output 含 reasoning，cache_read 含写）。
#[derive(Debug, Clone)]
struct MessageRead {
    day: String,
    model: String,
    input: f64,
    cache_read: f64,
    output: f64,
}

/// 从一个已打开的 ZCode DB 读出所有非零 assistant 消息（只读）。
fn read_messages(conn: &Connection) -> Vec<MessageRead> {
    let Ok(mut stmt) =
        conn.prepare("SELECT time_created, data FROM message ORDER BY time_created")
    else {
        return Vec::new();
    };
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .ok();
    let mut out = Vec::new();
    if let Some(rows) = rows {
        for (created_ms, data) in rows.flatten() {
            let Ok(parsed) = serde_json::from_str::<MessageData>(&data) else {
                continue;
            };
            // role 过滤放在解析后：`WHERE json_extract(...)` 依赖 SQL JSON1，
            // 而这份 data 里 role 就一个字符串字段，解析成本可忽略。
            if !data.contains("\"role\":\"assistant\"") {
                continue;
            }
            let input = parsed.tokens.input.max(0.0);
            let cache_read = (parsed.tokens.cache.read + parsed.tokens.cache.write).max(0.0);
            let output = (parsed.tokens.output + parsed.tokens.reasoning).max(0.0);
            if input <= 0.0 && cache_read <= 0.0 && output <= 0.0 {
                continue; // 中止的请求等零值行，与其他扫描器同口径跳过
            }
            let day = day_from_epoch_ms(created_ms);
            if day.is_empty() {
                continue;
            }
            let model = if parsed.model_id.trim().is_empty() {
                super::UNCLASSIFIED_MODEL.to_string()
            } else {
                parsed.model_id.trim().to_string()
            };
            out.push(MessageRead {
                day,
                model,
                input,
                cache_read,
                output,
            });
        }
    }
    out
}

/// 把逐消息读数聚合成报告（幂等：同输入同输出，无台账无基线）。
fn build_report(
    messages: &[MessageRead],
    session_count: u64,
    now: &str,
) -> LocalToolReport {
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    for m in messages {
        let e = by_day.entry(m.day.clone()).or_default();
        e.0 += m.input;
        e.1 += m.cache_read;
        e.2 += m.output;
        let me = by_model
            .entry(m.model.clone())
            .or_default()
            .entry(m.day.clone())
            .or_default();
        me.0 += m.input;
        me.1 += m.cache_read;
        me.2 += m.output;
    }
    let daily = finalize_days(by_day, KEEP_DAYS);
    let models = finalize_models(by_model, &BTreeMap::new(), KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count,
        project_count: 0,
        scanned_at: now.to_string(),
        models,
    }
}

/// 扫描所有 ZCode DB（Windows + WSL），归并成一份报告。
pub fn scan() -> LocalToolReport {
    let now = chrono::Utc::now().to_rfc3339();
    let mut all: Vec<MessageRead> = Vec::new();
    let mut sessions: u64 = 0;
    for db in zcode_dbs() {
        if let Some(conn) = open_reader(&db) {
            all.extend(read_messages(&conn));
            sessions += count_sessions(&conn);
        }
    }
    build_report(&all, sessions, &now)
}

/// 不同 session_id 计数（assistant 行）。
fn count_sessions(conn: &Connection) -> u64 {
    let Ok(mut stmt) = conn.prepare(
        "SELECT COUNT(DISTINCT session_id) FROM message \
         WHERE data LIKE '%\"role\":\"assistant\"%'",
    ) else {
        return 0;
    };
    stmt.query_row([], |r| r.get::<_, i64>(0))
        .map(|n| n.max(0) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(data: &str, created_ms: i64) -> (i64, String) {
        (created_ms, data.to_string())
    }

    fn read_all(rows: &[(i64, String)]) -> Vec<MessageRead> {
        // read_messages 吃 Connection；为可测性把解析内核抽成对行切片的纯逻辑
        // 是下一个重构——这里直接走 SQL 路径，用内存库造真实 schema。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        for (i, (tc, data)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![format!("m{i}"), format!("s{}", i % 2), tc, data],
            )
            .unwrap();
        }
        read_messages(&conn)
    }

    const ASSISTANT_ROW: &str = r#"{"role":"assistant","modelId":"GLM-5.3-Flash","tokens":{"input":1000,"output":200,"reasoning":50,"cache":{"read":3000,"write":100}},"time":{"created":0}}"#;

    #[test]
    fn parses_and_splits_token_buckets() {
        // 2026-09-29 12:00 +08:00 = 1790673600000ms 附近的某个时刻。
        let rows = vec![msg(ASSISTANT_ROW, 1_794_067_200_000)];
        let out = read_all(&rows);
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m.input, 1000.0);
        // cache.read + cache.write 折入 cache_read，口径与 claude.rs 一致。
        assert_eq!(m.cache_read, 3100.0);
        // output + reasoning 折入 output，口径与 hermes 一致。
        assert_eq!(m.output, 250.0);
        assert_eq!(m.model, "GLM-5.3-Flash");
    }

    #[test]
    fn skips_user_rows_and_zero_token_assistant_rows() {
        let rows = vec![
            msg(r#"{"role":"user","tokens":null}"#, 1_794_067_200_000),
            msg(
                r#"{"role":"assistant","modelId":"m","tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}"#,
                1_794_067_200_001,
            ),
        ];
        assert!(read_all(&rows).is_empty());
    }

    #[test]
    fn malformed_data_and_missing_model_are_tolerated() {
        let rows = vec![
            msg("not json at all", 1_794_067_200_000),
            msg(
                r#"{"role":"assistant","tokens":{"input":10,"output":5}}"#,
                1_794_067_200_001,
            ),
        ];
        let out = read_all(&rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].model, super::super::UNCLASSIFIED_MODEL);
        assert_eq!(out[0].input, 10.0);
    }

    #[test]
    fn build_report_is_idempotent_and_merges_days() {
        let messages = vec![
            MessageRead {
                day: "2026-09-28".into(),
                model: "m".into(),
                input: 100.0,
                cache_read: 50.0,
                output: 20.0,
            },
            MessageRead {
                day: "2026-09-28".into(),
                model: "m".into(),
                input: 10.0,
                cache_read: 0.0,
                output: 5.0,
            },
            MessageRead {
                day: "2026-09-29".into(),
                model: "m".into(),
                input: 7.0,
                cache_read: 0.0,
                output: 3.0,
            },
        ];
        let r1 = build_report(&messages, 0, "now");
        let r2 = build_report(&messages, 0, "now");
        assert_eq!(r1.total_tokens, r2.total_tokens); // 幂等
        assert_eq!(r1.total_tokens, 195.0);
        let d28 = r1.daily.iter().find(|d| d.date == "2026-09-28").unwrap();
        assert_eq!(d28.total, 185.0);
        assert_eq!(r1.models.len(), 1);
        assert_eq!(r1.id, "zcode");
    }

    #[test]
    fn count_sessions_dedupes_by_session_id() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        for i in 0..4 {
            conn.execute(
                "INSERT INTO message VALUES (?1, ?2, 0, ?3)",
                rusqlite::params![format!("m{i}"), format!("s{}", i % 2), ASSISTANT_ROW],
            )
            .unwrap();
        }
        assert_eq!(count_sessions(&conn), 2);
    }
}
