//! Hermes Agent (running inside WSL) usage scanner — delta accounting.
//!
//! Hermes's `state.db` (in `~/.hermes`) only stores **cumulative per-session**
//! token totals with a fixed `started_at` (session creation). A long-lived
//! session started days ago keeps accumulating tokens, but there is **no
//! per-day breakdown** in the DB.
//!
//! If we naively attributed each session's whole total to `started_at`, all of
//! a multi-day session's usage would land on its first day, leaving recent days
//! at 0. Instead we do **delta accounting**: persist a per-session baseline,
//! and on each scan attribute only the newly-added tokens to the **current
//! local day**. The first observation of a session seeds its baseline by
//! attributing the then-total to its `started_at`; everything after that counts
//! as it grows.
//!
//! We open the DB **read-only** (WAL, possibly locked by a running gateway) and
//! fall back to an *immutable* view if the read-only connection still fights
//! the lock. Baselines live in our own SQLite, never in Hermes's DB.

use super::{finalize_days, finalize_models, LocalToolReport};
use crate::local::{day_from_epoch_ms, wsl};
use crate::storage::Storage;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// 串行化 Hermes 扫描：首次扫描同时被 watch 与 get_local_tools/预热触发时，
/// 若都读到空基线会各自"种子"整段累计而重复归属。此锁保证基线读改写原子。
static SCAN_LOCK: Mutex<()> = Mutex::new(());

const TOOL_ID: &str = "hermes";
const TOOL_NAME: &str = "Hermes Agent";
const KEEP_DAYS: usize = 90;
const DB_FILE: &str = "state.db";

fn open_reader(db_path: &std::path::Path) -> Option<Connection> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    // 1) Read-only view: stays fresh with a live WAL writer. A running Hermes
    //    gateway holds the DB under an exclusive lock during writes, so a
    //    read-only connection can *open* yet return `DatabaseBusy` on the very
    //    first query. Verify it's actually readable before trusting it.
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
    // 2) Writer held the lock for the whole window → immutable view reads the
    //    on-disk DB without locking (best-effort; may miss un-checkpointed WAL).
    let url = format!("file:{}?immutable=1", db_path.to_string_lossy());
    Connection::open_with_flags(&url, flags).ok()
}

/// 一条会话的原始读数（已按当前展示口径拆分，output 含 reasoning，cost 取 max(实际,估算)）。
#[derive(Clone, Debug)]
struct SessionRead {
    sid: String,
    started_at_secs: f64,
    input: f64,
    cache_read: f64,
    output: f64,
    model: String,
    usd: f64,
    estimated: bool,
}

/// 从一个已打开的 Hermes DB 读取所有 `started_at` 非空的会话（只读，load 全部列但不碰正文）。
fn read_sessions(conn: &Connection) -> Vec<SessionRead> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, started_at, model, input_tokens, output_tokens, cache_read_tokens, \
         reasoning_tokens, estimated_cost_usd, actual_cost_usd \
         FROM sessions WHERE started_at IS NOT NULL",
    ) else {
        return Vec::new();
    };
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<f64>>(7)?,
                r.get::<_, Option<f64>>(8)?,
            ))
        })
        .ok();
    let mut out = Vec::new();
    if let Some(rows) = rows {
        for row in rows.flatten() {
            let (sid, started_at, model, input, output, cache_read, reasoning, est, act) = row;
            let input_f = input.max(0) as f64;
            let cache_f = cache_read.max(0) as f64;
            let output_f = (output.max(0) + reasoning.max(0)) as f64;
            if input_f <= 0.0 && cache_f <= 0.0 && output_f <= 0.0 {
                continue;
            }
            let usd = act.unwrap_or(0.0).max(est.unwrap_or(0.0)).max(0.0);
            let estimated = act.is_none_or(|a| a <= 0.0);
            let model = model.unwrap_or_default();
            let model = if model.is_empty() {
                crate::local::UNCLASSIFIED_MODEL.to_string()
            } else {
                model
            };
            out.push(SessionRead {
                sid,
                started_at_secs: started_at,
                input: input_f,
                cache_read: cache_f,
                output: output_f,
                model,
                usd,
                estimated,
            });
        }
    }
    out
}

/// 计算每个会话本次应计入的"归属增量"，并得到新基线。
///
/// - `force_seed_all`（台账为空的首个真实种子）：对**所有**会话整段累计归到各自
///   `started_at` 一次——用于台账刚启用/迁移时把历史补齐，避免"基线已在、台账未种子"
///   导致历史全部归零；
/// - 否则：无基线（新会话）→ 种子到 `started_at`；有基线且增长 → 增量到 `today`；
///   有基线但回退/缩小 → 本次不归属，基线重置到当前。
///
/// 返回 `(台账写入列表, 新基线列表)`；台账写入 = `(date, model, input, cache_read, output, cost_usd, estimated)`。
/// 种子/增量的持久化由调用方写进 `hermes_daily`（幂等累加），从而历史永不消失。
fn plan_updates(
    sessions: &[SessionRead],
    baselines: &BTreeMap<String, (f64, f64, f64, f64)>,
    today: &str,
    force_seed_all: bool,
) -> (Vec<(String, String, f64, f64, f64, f64, bool)>, Vec<(String, f64, f64, f64, f64)>) {
    let mut writes = Vec::new();
    let mut new_base: Vec<(String, f64, f64, f64, f64)> = Vec::with_capacity(sessions.len());
    for s in sessions {
        if !force_seed_all {
            if let Some((bi, bcr, bo, bcost)) = baselines.get(&s.sid).copied() {
                // 已有基线：增量记账（历史种子早在台账中）。回退/缩小则不归属。
                if s.input >= bi && s.cache_read >= bcr && s.output >= bo {
                    let di = s.input - bi;
                    let dcr = s.cache_read - bcr;
                    let d = s.output - bo;
                    if di > 0.0 || dcr > 0.0 || d > 0.0 {
                        writes.push((today.to_string(), s.model.clone(), di, dcr, d, (s.usd - bcost).max(0.0), s.estimated));
                    }
                }
                new_base.push((s.sid.clone(), s.input, s.cache_read, s.output, s.usd));
                continue;
            }
        }
        // 新会话 或 强制全种子：整段累计作为种子归到其开始日。
        if s.started_at_secs > 0.0 {
            let day = day_from_epoch_ms((s.started_at_secs * 1000.0) as i64);
            if !day.is_empty() {
                writes.push((day, s.model.clone(), s.input, s.cache_read, s.output, s.usd, s.estimated));
            }
        }
        new_base.push((s.sid.clone(), s.input, s.cache_read, s.output, s.usd));
    }
    (writes, new_base)
}

/// 从逐日台账重建报告（每日 + 每模型 + 成本）。
fn build_report(ledger: &[(String, String, f64, f64, f64, f64, i64)], now: &str) -> LocalToolReport {
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut cost_by_model: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut est_by_model: BTreeMap<String, bool> = BTreeMap::new();
    for (date, model, input, cache_read, output, cost, estimated) in ledger {
        let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += input;
        e.1 += cache_read;
        e.2 += output;
        let m = by_model.entry(model.clone()).or_default();
        let me = m.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        me.0 += input;
        me.1 += cache_read;
        me.2 += output;
        if *cost > 0.0 {
            // 币种统一为 USD（Hermes 上报/估算字段 *_cost_usd）。cost_estimated 单独追踪：
            // 只要出现过实测成本（estimated=false）则整模型视为"上报"非估算。
            let cb = cost_by_model.entry(model.clone()).or_insert((0.0, "USD".to_string()));
            cb.0 += cost;
            let est = *estimated == 1;
            est_by_model
                .entry(model.clone())
                .and_modify(|e| {
                    if !est {
                        *e = false;
                    }
                })
                .or_insert(est);
        }
    }

    let mut models = finalize_models(by_model, &cost_by_model, KEEP_DAYS);
    let daily = finalize_days(by_day, KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
    for m in &mut models {
        m.cost_estimated = est_by_model.get(&m.model).copied().unwrap_or(false);
    }
    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count: 0,
        project_count: 0,
        scanned_at: now.to_string(),
        models,
    }
}

/// 扫描所有 Hermes `state.db`（Windows + 各 WSL 发行版 home），做增量记账到台账后归并成报告。
pub fn scan(storage: &Storage) -> LocalToolReport {
    // 串行化：避免多线程首次扫描同时以空基线"种子"导致历史重复归属。
    let _guard = SCAN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let now = chrono::Utc::now().to_rfc3339();
    let today = chrono::Local::now().date_naive().to_string();
    let cutoff = (chrono::Local::now().date_naive() - chrono::Duration::days(KEEP_DAYS as i64)).to_string();

    let mut sessions: Vec<SessionRead> = Vec::new();
    for hermes in wsl::existing_dotdirs(".hermes") {
        let db_path = hermes.join(DB_FILE);
        if !db_path.exists() {
            continue;
        }
        if let Some(conn) = open_reader(&db_path) {
            sessions.extend(read_sessions(&conn));
        }
    }

    let mut baselines: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
    if let Ok(rows) = storage.get_hermes_baselines() {
        for (sid, i, cr, o, c) in rows {
            baselines.insert(sid, (i, cr, o, c));
        }
    }

    // 台账为空（迁移/首次）时，对**所有**会话做一次整段种子把历史补齐——否则已有的会话
    // 基线会让历史永久归零。台账非空后才走纯增量。
    let force_seed = storage
        .get_hermes_daily()
        .map(|l| l.is_empty())
        .unwrap_or(true);

    // 以"本次读到的会话全集"为标准，把种子/增量写进台账，然后保存新基线（天然清理消失会话）。
    let (writes, new_base) = plan_updates(&sessions, &baselines, &today, force_seed);
    for (date, model, input, cache_read, output, cost, est) in writes {
        let _ = storage.add_hermes_daily(&date, &model, input, cache_read, output, cost, est);
    }
    let _ = storage.save_hermes_baselines(&new_base);
    let _ = storage.prune_hermes_daily(&cutoff);

    let mut report = match storage.get_hermes_daily() {
        Ok(ledger) => build_report(&ledger, &now),
        Err(_) => build_report(&[], &now),
    };
    report.session_count = sessions.len() as u64;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(sid: &str, started: f64, i: f64, cr: f64, o: f64, usd: f64) -> SessionRead {
        SessionRead {
            sid: sid.to_string(),
            started_at_secs: started,
            input: i,
            cache_read: cr,
            output: o,
            model: "m".to_string(),
            usd,
            estimated: true,
        }
    }

    #[test]
    fn seeds_new_session_to_start_day_then_deltas_to_today() {
        // 一个会话在 2026-01-02 开始，第一次扫描时累计 (100,50,20)。
        let started = 1767312000.0; // 2026-01-02T00:00:00Z-ish
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.0)];

        // 首次：无基线 → 种子归到 start 日，基线=当前。
        let base0 = BTreeMap::new();
        let (attrs0, nb0) = plan_updates(&sessions, &base0, "2026-09-24", false);
        assert_eq!(attrs0.len(), 1);
        assert_eq!(attrs0[0].0, day_from_epoch_ms((started * 1000.0) as i64)); // seed day
        assert_eq!(attrs0[0].3, 50.0); // cache_read
        assert_eq!(nb0[0].1, 100.0); // 基线 input

        // 第二次：会话增长到 (130,60,30) → 增量 (30,10,10) 归到今天，新基线=当前。
        let grown = vec![s("a", started, 130.0, 60.0, 30.0, 0.1)];
        let mut base1 = BTreeMap::new();
        base1.insert("a".to_string(), (nb0[0].1, nb0[0].2, nb0[0].3, nb0[0].4));
        let (attrs1, nb1) = plan_updates(&grown, &base1, "2026-09-24", false);
        assert_eq!(attrs1.len(), 1);
        assert_eq!(attrs1[0].0, "2026-09-24"); // 增量归到今天
        assert_eq!(attrs1[0].2, 30.0); // input 增量
        assert_eq!(attrs1[0].3, 10.0); // cache_read 增量
        assert_eq!(attrs1[0].4, 10.0); // output 增量
        assert_eq!(nb1[0].1, 130.0); // 基线前移
    }

    #[test]
    fn shrinking_session_resets_baseline_without_attribution() {
        // 基线 (100,50,20)，当前回退到 (80,40,15)（Hermes 压缩/重算）→ 不归属，基线重置。
        let sessions = vec![s("a", 0.0, 80.0, 40.0, 15.0, 0.0)];
        let mut base = BTreeMap::new();
        base.insert("a".to_string(), (100.0, 50.0, 20.0, 0.0));
        let (attrs, _nb) = plan_updates(&sessions, &base, "2026-09-24", false);
        assert_eq!(attrs.len(), 0);
    }

    #[test]
    fn force_seed_restores_history_even_when_baselines_exist() {
        // 迁移场景：有基线（历史未被台账记过）但强制全种子 → 整段累计仍归到 start 日。
        // 这修复"hermes 历史归零"：台账为空时 force_seed_all=true，忽略已有基线补种子。
        let started = 1767312000.0; // 2026-01-02
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.3)];
        let mut base = BTreeMap::new();
        base.insert("a".to_string(), (100.0, 50.0, 20.0, 0.3)); // 基线已存在
        let (attrs, nb) = plan_updates(&sessions, &base, "2026-09-25", true);
        assert_eq!(attrs.len(), 1); // 即便如此也要种子历史
        assert_eq!(attrs[0].0, day_from_epoch_ms((started * 1000.0) as i64));
        assert_eq!(nb[0].1, 100.0); // 基线仍记录当前累计，防下一次重复种子
    }

    #[test]
    fn duplicate_scan_second_time_counts_nothing() {
        // 同一会话值不变、再次扫描（常见：watch 多轮）→ 无增量，不出新归属。
        let started = 1767312000.0;
        let s1 = vec![s("a", started, 100.0, 50.0, 20.0, 0.0)];
        let (attrs0, nb0) = plan_updates(&s1, &BTreeMap::new(), "2026-09-24", false);
        assert_eq!(attrs0.len(), 1);
        let mut base = BTreeMap::new();
        base.insert("a".to_string(), (nb0[0].1, nb0[0].2, nb0[0].3, nb0[0].4));
        let (attrs1, _nb1) = plan_updates(&s1, &base, "2026-09-24", false);
        assert_eq!(attrs1.len(), 0);
    }

    #[test]
    fn build_report_merges_ledger_into_day_model_cost() {
        let report = build_report(
            &[
                ("2026-09-24".to_string(), "m".to_string(), 100.0, 50.0, 20.0, 0.3, 0),
                ("2026-09-25".to_string(), "m".to_string(), 5.0, 1.0, 2.0, 0.0, 0),
            ],
            "2026-09-25T00:00:00Z",
        );
        assert!(report.daily.iter().any(|d| d.date == "2026-09-25" && d.total == 8.0));
        let usage = report.models.iter().find(|u| u.model == "m").unwrap();
        assert_eq!(usage.total_tokens, 178.0); // 100+50+20+5+1+2
        assert_eq!(usage.currency, "USD"); // 币种归一到 USD，不再是 "estimated"/"reported"
        assert!((usage.cost - 0.3).abs() < 1e-9);
        assert!(!usage.cost_estimated);
    }
}