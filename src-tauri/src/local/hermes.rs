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
//! 记账粒度是 `(session_id, billing_provider, model)`，而不是只有会话：Hermes
//! 的 `session_model_usage` 把每个会话拆到多行（按模型 + 计费账户 + base_url +
//! task 的联合主键），同一个 `deepseek-v4.1-flash` 既可能走 DeepSeek 官方、也
//! 可能走 OpenCode Go。只按会话记账会把不同账户的账单合并成一行，趋势页的
//! Provider 归因就错了（用户实际观测到的现象）。旧库没有这张表时退回按会话读。
//!
//! We open the DB **read-only** (WAL, possibly locked by a running gateway). When
//! the data dir has moved into WSL (`\\wsl$\...`, a 9p share) the usual read-only
//! open fails with "database is locked" because 9p cannot negotiate WAL's `-shm`
//! locks; `sqlite_ro::open_readonly` then copies `db`/`-wal`/`-shm` to a temp dir
//! and reads the copy. Baselines live in our own SQLite, never in Hermes's DB.
//!
//! 基线表与台账是两代数据，旧版本在 DB 不可读时会把空基线落盘（"整表删除 +
//! 重插本轮会话"），导致下次读得到库时所有会话都被当新会话重新整段种子，历史
//! 用量翻倍。现在 DB 不可读的一轮**既不写台账也不动基线**；万一基线丢了而台账
//! 还有历史，本轮只重建基线、不重种（宁可少记一次，也不能翻倍）。

use super::{finalize_days, sqlite_ro, LocalModelUsage, LocalToolReport};
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

/// 模型键里 provider 与模型名的分隔符。用 ASCII 单元分隔符（U+001F），模型名
/// 与账户 id 里不可能出现，因此老台账里没有分隔符的裸模型名可以无歧义兼容。
const KEY_SEP: char = '\u{1f}';

fn open_reader(db_path: &std::path::Path) -> Option<sqlite_ro::ReadOnlyDb> {
    // 只读连接能看见 `-wal`（Hermes 的 gateway 常驻在线），读到的是最近一次提交
    // 的数据。写者长时间持锁时宁可本轮不出数——账本按来源批量删除，缺源等于保留
    // 上一轮的旧行——也不要退回 `immutable=1`：那会忽略 WAL、拿到旧快照并覆盖
    // 账本里已正确的数字（见 `local::open_sqlite_readonly`）。
    //
    // 数据目录迁进 WSL 后直接打开会因 9p 的 shm 锁协商失败而 busy；
    // `sqlite_ro::open_readonly` 负责退回到本地临时副本，仍然失败才返回 None。
    sqlite_ro::open_readonly(db_path)
}

/// 把 `(计费账户, 模型)` 编码成台账里的模型键。账户为空时保持裸模型名，兼容
/// 旧台账行（旧版本只记模型）。
fn model_key(provider: &str, model: &str) -> String {
    if provider.is_empty() {
        model.to_string()
    } else {
        format!("{provider}{KEY_SEP}{model}")
    }
}

/// [`model_key`] 的逆运算：拆回 `(计费账户, 模型)`；旧台账行没有分隔符时账户为空。
fn split_model_key(key: &str) -> (&str, &str) {
    match key.split_once(KEY_SEP) {
        Some((provider, model)) => (provider, model),
        None => ("", key),
    }
}

/// 一条桶的累计 token（用于合并重复读取时判断哪份更新）。
fn token_total(s: &SessionRead) -> f64 {
    s.input + s.cache_read + s.output
}

/// 合并重复的桶：同一会话可能同时存在于 Windows 与 WSL 的 `.hermes` 副本里
/// （数据迁移后旧副本没删）。同一 `(sid, provider, model)` 只保留累计最大的
/// 那条——那是更新的副本。保留两条会让基线表主键冲突（整批保存回滚），而每轮
/// 又按旧基线重复记同一段增量。
fn dedupe_sessions(rows: Vec<SessionRead>) -> Vec<SessionRead> {
    let mut by_key: BTreeMap<String, SessionRead> = BTreeMap::new();
    for s in rows {
        let key = baseline_key(&s);
        let replace = by_key
            .get(&key)
            .map(|prev| token_total(&s) > token_total(prev))
            .unwrap_or(true);
        if replace {
            by_key.insert(key, s);
        }
    }
    by_key.into_values().collect()
}

/// 基线键：会话 + 计费账户 + 模型。同一会话可以同时命中多个账户/模型，各自
/// 独立做增量记账，账本里才能分开归因。
fn baseline_key(s: &SessionRead) -> String {
    format!("{}{KEY_SEP}{}{KEY_SEP}{}", s.sid, s.provider, s.model)
}

/// 一条会话（或会话 × 模型 × 账户）的原始读数（已按当前展示口径拆分，output
/// 含 reasoning，cost 取 max(实际,估算)）。
#[derive(Clone, Debug)]
struct SessionRead {
    sid: String,
    started_at_secs: f64,
    /// 这个 `(会话, 账户, 模型)` 桶第一次被计费的时刻（`session_model_usage.first_seen`）。
    /// 比会话开始时间更准：一个会话可以持续几天，`started_at` 会把这些天的用量
    /// 全压回开头。旧库没有这张表时为 None，回退到 `started_at`。
    first_seen_secs: Option<f64>,
    input: f64,
    cache_read: f64,
    output: f64,
    model: String,
    /// 实际上游计费账户（`session_model_usage.billing_provider`），空串 = 没记。
    provider: String,
    usd: f64,
    estimated: bool,
}

/// 表里是否有这一列（旧版本 Hermes 的 `session_model_usage` 没有 `first_seen`）。
fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let found = rows.flatten().any(|name| name == column);
    found
}
/// `sqlite_master` 里是否有某张表。查询失败返回 None（库不可用，调用方据此
/// 判定本轮不出数）。
fn has_table(conn: &Connection, name: &str) -> Option<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .map(|n| n > 0)
}

fn normalized_model(model: Option<String>) -> String {
    match model.unwrap_or_default() {
        m if m.is_empty() => crate::local::UNCLASSIFIED_MODEL.to_string(),
        m => m,
    }
}

/// 按 `(session_id, model, billing_provider)` 聚合 `session_model_usage`：Hermes
/// 的联合主键还包含 base_url/task，先在这里合并，再交给增量记账。
fn read_bucketed_usage(conn: &Connection) -> Option<Vec<SessionRead>> {
    // `first_seen` 是较新版本才有的列（旧库没有）：有的库按它取归属日，没有的库
    // 用字面量 NULL 占位，保持元组形状一致——绝不能因为缺一列就整库放弃。
    let first_seen_expr = if has_column(conn, "session_model_usage", "first_seen") {
        "MIN(m.first_seen)"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT m.session_id, s.started_at, m.model, m.billing_provider, \
                SUM(m.input_tokens), SUM(m.cache_read_tokens), \
                SUM(m.output_tokens), SUM(m.reasoning_tokens), \
                SUM(m.estimated_cost_usd), SUM(m.actual_cost_usd), \
                SUM(CASE WHEN m.actual_cost_usd > 0 THEN 1 ELSE 0 END), \
                {first_seen_expr} \
         FROM session_model_usage m JOIN sessions s ON s.id = m.session_id \
         WHERE s.started_at IS NOT NULL \
         GROUP BY m.session_id, m.model, m.billing_provider"
    );
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, f64>(8)?,
                r.get::<_, f64>(9)?,
                r.get::<_, i64>(10)?,
                r.get::<_, Option<f64>>(11)?,
            ))
        })
        .ok()?;
    Some(
        rows.flatten()
            .filter_map(|(sid, started_at, model, provider, input, cache_read, output, reasoning, est, act, actual_calls, first_seen)| {
                let input_f = input.max(0) as f64;
                let cache_f = cache_read.max(0) as f64;
                let output_f = (output.max(0) + reasoning.max(0)) as f64;
                if input_f <= 0.0 && cache_f <= 0.0 && output_f <= 0.0 {
                    return None;
                }
                Some(SessionRead {
                    sid,
                    started_at_secs: started_at,
                    first_seen_secs: first_seen,
                    input: input_f,
                    cache_read: cache_f,
                    output: output_f,
                    model: normalized_model(model),
                    provider: provider.unwrap_or_default(),
                    usd: act.max(est).max(0.0),
                    estimated: actual_calls <= 0,
                })
            })
            .collect(),
    )
}

/// 读取没有 `session_model_usage` 行的会话（旧会话 / 旧库）：整段会话累计 +
/// `sessions.billing_provider`。`only_without_smu=false` 时读全部会话（旧库没有
/// 那张表）。
fn read_session_rows(conn: &Connection, only_without_smu: bool) -> Option<Vec<SessionRead>> {
    let sql = if only_without_smu {
        "SELECT s.id, s.started_at, s.model, s.billing_provider, s.input_tokens, \
                s.output_tokens, s.cache_read_tokens, s.reasoning_tokens, \
                s.estimated_cost_usd, s.actual_cost_usd \
         FROM sessions s WHERE s.started_at IS NOT NULL AND NOT EXISTS \
           (SELECT 1 FROM session_model_usage m WHERE m.session_id = s.id)"
    } else {
        "SELECT s.id, s.started_at, s.model, s.billing_provider, s.input_tokens, \
                s.output_tokens, s.cache_read_tokens, s.reasoning_tokens, \
                s.estimated_cost_usd, s.actual_cost_usd \
         FROM sessions s WHERE s.started_at IS NOT NULL"
    };
    let mut stmt = conn.prepare(sql).ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<f64>>(8)?,
                r.get::<_, Option<f64>>(9)?,
            ))
        })
        .ok()?;
    Some(
        rows.flatten()
            .filter_map(|(sid, started_at, model, provider, input, output, cache_read, reasoning, est, act)| {
                let input_f = input.max(0) as f64;
                let cache_f = cache_read.max(0) as f64;
                let output_f = (output.max(0) + reasoning.max(0)) as f64;
                if input_f <= 0.0 && cache_f <= 0.0 && output_f <= 0.0 {
                    return None;
                }
                let usd = act.unwrap_or(0.0).max(est.unwrap_or(0.0)).max(0.0);
                Some(SessionRead {
                    sid,
                    started_at_secs: started_at,
                    first_seen_secs: None,
                    input: input_f,
                    cache_read: cache_f,
                    output: output_f,
                    model: normalized_model(model),
                    provider: provider.unwrap_or_default(),
                    usd,
                    estimated: act.is_none_or(|a| a <= 0.0),
                })
            })
            .collect(),
    )
}

/// 从一个已打开的 Hermes DB 读取全部用量行。返回 None 表示**库不可用/结构
/// 读不了**（调用方必须据此放弃本轮记账，否则空基线会覆盖真实基线）。
fn read_usage_rows(conn: &Connection) -> Option<Vec<SessionRead>> {
    // 有没有 per-model 表决定记账粒度；查不到表（旧库/损坏）退回会话级。
    let has_smu = has_table(conn, "session_model_usage")?;
    let mut rows = if has_smu {
        read_bucketed_usage(conn)?
    } else {
        Vec::new()
    };
    let mut legacy = read_session_rows(conn, has_smu)?;
    rows.append(&mut legacy);
    Some(rows)
}

/// 这个桶该记在哪一天：优先桶的首次计费日（`first_seen`），回退会话开始日。
/// 空串/0 一律当"没有时间信息"。
fn bucket_day(s: &SessionRead) -> Option<String> {
    let epoch = s
        .first_seen_secs
        .filter(|v| *v > 0.0)
        .or_else(|| (s.started_at_secs > 0.0).then_some(s.started_at_secs))?;
    let day = day_from_epoch_ms((epoch * 1000.0) as i64);
    (!day.is_empty()).then_some(day)
}
/// 补齐台账尾部：把"桶首次计费日晚于台账最后一天"的桶整段补进它自己的首次计费日。
///
/// 为什么需要：数据源换了位置（Windows `~/.hermes` → WSL `~/.hermes`）期间扫描器
/// 读不到库，台账就停在旧位置最后一次能读的那天；后续某轮读通之后，基线又按"当前
/// 累计"重建过，于是每次增量都是 0，缺口永远不会自己长回来。这类桶的首次计费日
/// 在台账里没有任何行，补它不可能重复；写进去之后台账尾部前移，下一轮自然跳过。
///
/// **只处理本轮之前已有基线的桶**：全新桶由 [`plan_updates`] 负责种子。两边都写
/// 会把同一段累计记两遍（新会话的 `started_at` 与 `first_seen` 常常就是同一天）。
/// `pending` 是 `plan_updates` 本轮已决定写入的行，一并纳入去重，防止同一
/// (day, 模型键) 被两处各写一次。
///
/// 跨越缺口的老桶（首次计费日 ≤ 台账尾部）不碰：把整段压到开头会和已有历史重复。
/// 返回 `(补写行, 因同日同模型已被占用而跳过的桶数)`。
fn backfill_tail(
    sessions: &[SessionRead],
    ledger: &[(String, String, f64, f64, f64, f64, i64)],
    ledger_last_day: Option<&str>,
    baselines: &BTreeMap<String, (f64, f64, f64, f64)>,
    pending: &[(String, String, f64, f64, f64, f64, bool)],
) -> (Vec<(String, String, f64, f64, f64, f64, bool)>, usize) {
    let Some(last) = ledger_last_day else {
        return (Vec::new(), 0);
    };
    let mut recorded: std::collections::BTreeSet<(String, String)> =
        ledger.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
    for (date, key, ..) in pending {
        recorded.insert((date.clone(), key.clone()));
    }
    let mut writes = Vec::new();
    let mut skipped = 0usize;
    for s in sessions {
        let key = baseline_key(s);
        // 旧版基线只按会话记（裸 sid）。命中它同样算“已有基线”，否则升级首轮
        // 会把老会话当成新会话再补一遍。
        if !baselines.contains_key(&key) && !baselines.contains_key(&s.sid) {
            continue;
        }
        let Some(day) = bucket_day(s) else { continue };
        if day.as_str() <= last {
            continue;
        }
        let model = model_key(&s.provider, &s.model);
        if !recorded.insert((day.clone(), model.clone())) {
            skipped += 1;
            continue;
        }
        writes.push((day, model, s.input, s.cache_read, s.output, s.usd, s.estimated));
    }
    (writes, skipped)
}
/// 计算每个基线键本次应计入的"归属增量"，并得到新基线。
///
/// - `force_seed_all`（台账为空的首个真实种子）：对**所有**会话整段累计归到各自
///   `started_at` 一次——用于台账刚启用/迁移时把历史补齐，避免"基线已在、台账未种子"
///   导致历史全部归零；
/// - `seed_suppressed`（基线丢光但台账已有历史）：**不重种任何会话**，只按当前
///   累计重建基线。任何"整段种子"都可能与台账里的历史行重复（同一段累计已经被
///   记过），宁可少记一轮，也不让历史翻倍；之后的新增长量照常做增量归属。
/// - 否则：无基线（新会话）→ 种子到 `started_at`；有基线且增长 → 增量到 `today`；
///   有基线但回退/缩小 → 本次不归属，基线重置到当前。
///
/// 返回 `(台账写入列表, 新基线列表)`；台账写入 = `(date, 模型键, input, cache_read, output, cost_usd, estimated)`，
/// 模型键由 [`model_key`] 编码了计费账户。种子/增量的持久化由调用方写进
/// `hermes_daily`（幂等累加），从而历史永不消失。
fn plan_updates(
    sessions: &[SessionRead],
    baselines: &BTreeMap<String, (f64, f64, f64, f64)>,
    today: &str,
    force_seed_all: bool,
    seed_suppressed: bool,
    ledger_last_day: Option<&str>,
) -> (Vec<(String, String, f64, f64, f64, f64, bool)>, Vec<(String, f64, f64, f64, f64)>) {
    let mut writes = Vec::new();
    let mut new_base: Vec<(String, f64, f64, f64, f64)> = Vec::with_capacity(sessions.len());
    for s in sessions {
        let key = baseline_key(s);
        if !force_seed_all {
            if let Some((bi, bcr, bo, bcost)) = baselines.get(&key).copied() {
                // 已有基线：增量记账（历史种子早在台账中）。回退/缩小则不归属。
                if s.input >= bi && s.cache_read >= bcr && s.output >= bo {
                    let di = s.input - bi;
                    let dcr = s.cache_read - bcr;
                    let d = s.output - bo;
                    if di > 0.0 || dcr > 0.0 || d > 0.0 {
                        writes.push((
                            today.to_string(),
                            model_key(&s.provider, &s.model),
                            di,
                            dcr,
                            d,
                            (s.usd - bcost).max(0.0),
                            s.estimated,
                        ));
                    }
                }
                new_base.push((key, s.input, s.cache_read, s.output, s.usd));
                continue;
            }
            // 旧版基线只按会话记（裸 sid）。新键是 `sid␟provider␟model`，升级后
            // 首次运行必然查不到新键；若把它当“新会话”整段重种，会把台账里已有的
            // 历史再记一遍。命中旧键时本轮只重建键、不写台账，增量从下一轮开始。
            if baselines.contains_key(&s.sid) {
                new_base.push((key, s.input, s.cache_read, s.output, s.usd));
                continue;
            }
        }
        // 新会话 或 强制全种子：整段累计作为种子归到其开始日。
        //
        // `seed_suppressed`（基线丢光但台账有历史）时**只允许**给"开始日严格晚于台账
        // 最后一天"的会话补种子：旧规则把整段累计记在开始日，这类会话在台账里不可能
        // 有历史行，补它不会重复；更早开始的会话仍然只重建基线，宁可留一段空洞，也不
        // 让历史翻倍（P0 的两条腿都要站住）。
        let day = if s.started_at_secs > 0.0 {
            day_from_epoch_ms((s.started_at_secs * 1000.0) as i64)
        } else {
            String::new()
        };
        let seed_allowed = !seed_suppressed
            || match (ledger_last_day, day.as_str()) {
                (Some(last), d) => !d.is_empty() && d > last,
                _ => false,
            };
        if seed_allowed && !day.is_empty() {
            writes.push((
                day,
                model_key(&s.provider, &s.model),
                s.input,
                s.cache_read,
                s.output,
                s.usd,
                s.estimated,
            ));
        }
        new_base.push((key, s.input, s.cache_read, s.output, s.usd));
    }
    (writes, new_base)
}

/// 从逐日台账重建报告（每日 + 每 (账户, 模型) + 成本）。
fn build_report(ledger: &[(String, String, f64, f64, f64, f64, i64)], now: &str) -> LocalToolReport {
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_account: BTreeMap<(String, String), BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut cost_by_account: BTreeMap<(String, String), (f64, String)> = BTreeMap::new();
    let mut est_by_account: BTreeMap<(String, String), bool> = BTreeMap::new();
    for (date, key, input, cache_read, output, cost, estimated) in ledger {
        let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += input;
        e.1 += cache_read;
        e.2 += output;
        let (provider, model) = split_model_key(key);
        let account = (provider.to_string(), model.to_string());
        let days = by_account.entry(account.clone()).or_default();
        let me = days.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
        me.0 += input;
        me.1 += cache_read;
        me.2 += output;
        if *cost > 0.0 {
            // 币种统一为 USD（Hermes 上报/估算字段 *_cost_usd）。cost_estimated 单独追踪：
            // 只要出现过实测成本（estimated=false）则该行视为"上报"非估算。
            let cb = cost_by_account.entry(account.clone()).or_insert((0.0, "USD".to_string()));
            cb.0 += cost;
            let est = *estimated == 1;
            est_by_account
                .entry(account)
                .and_modify(|e| {
                    if !est {
                        *e = false;
                    }
                })
                .or_insert(est);
        }
    }

    let mut models: Vec<LocalModelUsage> = Vec::new();
    for ((provider, model), days) in by_account {
        let series = finalize_days(days, KEEP_DAYS);
        let total_tokens: f64 = series.iter().map(|d| d.total).sum();
        if total_tokens <= 0.0 {
            continue;
        }
        let (cost, currency) = cost_by_account
            .get(&(provider.clone(), model.clone()))
            .cloned()
            .unwrap_or((0.0, String::new()));
        let cost_estimated = est_by_account
            .get(&(provider.clone(), model.clone()))
            .copied()
            .unwrap_or(false);
        models.push(LocalModelUsage {
            model,
            provider,
            total_tokens,
            cost,
            currency,
            cost_estimated,
            daily: series,
        });
    }
    models.sort_by(|a, b| {
        b.total_tokens
            .partial_cmp(&a.total_tokens)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let daily = finalize_days(by_day, KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();
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
    let mut found_any_db = false;
    let mut read_failed = false;
    for hermes in wsl::existing_dotdirs(".hermes") {
        let db_path = hermes.join(DB_FILE);
        if !db_path.exists() {
            continue;
        }
        found_any_db = true;
        match open_reader(&db_path) {
            Some(conn) => match read_usage_rows(&conn) {
                Some(rows) => sessions.extend(rows),
                None => read_failed = true,
            },
            None => read_failed = true,
        }
    }

    let mut ledger = storage.get_hermes_daily().unwrap_or_default();
    let sessions = dedupe_sessions(sessions);
    // 台账里最新的一天。seed_suppressed 时用它判断哪些会话"晚于历史、补种安全"。
    let ledger_last_day: Option<String> = ledger.iter().map(|r| r.0.clone()).max();
    // 库不可读（9p/WAL 锁 + 副本回退也失败）时**本轮什么都不写**：既不清基线也不
    // 记台账，直接沿用既有台账出报表。旧实现会落盘空基线，下一次读得到库时所有
    // 会话都成了"新会话"被整段重种，历史用量翻倍（bug3）。
    if read_failed {
        tracing::warn!(
            target: "tum.local",
            "hermes state.db 本轮不可读（WAL/9p 锁或临时副本打开失败），跳过本轮记账以保留既有基线"
        );
    } else if found_any_db {
        let mut baselines: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
        if let Ok(rows) = storage.get_hermes_baselines() {
            for (sid, i, cr, o, c) in rows {
                baselines.insert(sid, (i, cr, o, c));
            }
        }

        // 台账为空（迁移/首次）时，对**所有**会话做一次整段种子把历史补齐——否则已有的会话
        // 基线会让历史永久归零。台账非空后才走纯增量。
        let force_seed = ledger.is_empty();
        // 基线全丢但台账仍有历史（旧版本 bug 的残留 / 手工清表）：禁止重种任何
        // 会话，只重建基线；否则同一段累计会被再记一遍，历史翻倍。
        let seed_suppressed = baselines.is_empty() && !ledger.is_empty();
        if seed_suppressed {
            tracing::warn!(
                target: "tum.local",
                "hermes 基线为空但台账已有 {} 行历史：本轮只重建基线、不重种，避免历史用量重复归属",
                ledger.len()
            );
        }

        let (mut writes, new_base) =
            plan_updates(&sessions, &baselines, &today, force_seed, seed_suppressed, ledger_last_day.as_deref());

        // 补齐台账尾部：数据源换位置（Windows → WSL）那段时间里扫描器根本读不到库，
        // 台账就停在旧位置最后一次能读的那天，而基线又被后续成功的一轮按"当前累计"
        // 重建过——此后每次增量都是 0，缺口永远不会自己长回来。这里按"桶首次计费日"
        // 把**晚于台账尾部**的桶整段补进它自己的首次计费日：该日该模型在台账里没有
        // 行，补它不可能与历史重复（幂等：写进去后台账尾部前移，下一轮自然跳过）。
        // 跨越缺口的老桶（首次计费日早于尾部）仍旧不碰：把整段压到开头会和已有
        // 历史重复，宁可少记也不翻倍。
        let (tail_writes, backfill_skipped) = backfill_tail(
            &sessions,
            &ledger,
            ledger_last_day.as_deref(),
            &baselines,
            &writes,
        );
        if !tail_writes.is_empty() || backfill_skipped > 0 {
            tracing::info!(
                target: "tum.local.hermes",
                backfilled = tail_writes.len(),
                backfill_skipped,
                ledger_last_day = ledger_last_day.as_deref().unwrap_or("-"),
                "hermes 补齐台账尾部（数据源迁移期间漏记的日期）"
            );
        }
        writes.extend(tail_writes);
        tracing::debug!(
            target: "tum.local.hermes",
            sessions = sessions.len(),
            baselines = baselines.len(),
            ledger_rows = ledger.len(),
            seed_suppressed,
            writes = writes.len(),
            "hermes 本轮记账决策"
        );
        for (date, model, input, cache_read, output, cost, est) in writes {
            let _ = storage.add_hermes_daily(&date, &model, input, cache_read, output, cost, est);
        }
        let _ = storage.save_hermes_baselines(&new_base);
        let _ = storage.prune_hermes_daily(&cutoff);
        ledger = storage.get_hermes_daily().unwrap_or(ledger);
    }

    let mut report = build_report(&ledger, &now);
    // session_count 按去重后的会话 id 计（记账粒度更细，不能直接拿行数）。
    let mut sids: Vec<&str> = sessions.iter().map(|s| s.sid.as_str()).collect();
    sids.sort_unstable();
    sids.dedup();
    report.session_count = sids.len() as u64;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(sid: &str, started: f64, i: f64, cr: f64, o: f64, usd: f64) -> SessionRead {
        SessionRead {
            sid: sid.to_string(),
            started_at_secs: started,
            first_seen_secs: None,
            input: i,
            cache_read: cr,
            output: o,
            model: "m".to_string(),
            provider: String::new(),
            usd,
            estimated: true,
        }
    }

    fn with_provider(mut read: SessionRead, provider: &str, model: &str) -> SessionRead {
        read.provider = provider.to_string();
        read.model = model.to_string();
        read
    }

    /// 本地时区正午的时间戳：避开夏令时边界，也避开"当天零点算前一天"的歧义。
    fn local_noon(y: i32, m: u32, d: u32) -> f64 {
        use chrono::{Local, TimeZone};
        Local
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .single()
            .expect("valid local time")
            .timestamp() as f64
    }

    fn ledger_row(date: &str, key: &str, total: f64) -> (String, String, f64, f64, f64, f64, i64) {
        (date.to_string(), key.to_string(), total, 0.0, 0.0, 0.0, 1)
    }

    #[test]
    fn bucket_day_prefers_first_seen_over_session_start() {
        let mut b = with_provider(s("s1", local_noon(2026, 9, 13), 10.0, 0.0, 0.0, 0.0), "opencode-go", "deepseek-v4.1-flash");
        b.first_seen_secs = Some(local_noon(2026, 10, 20));
        assert_eq!(bucket_day(&b).as_deref(), Some("2026-10-20"));
        // 没有 first_seen（旧库）时退回会话开始日。
        b.first_seen_secs = None;
        assert_eq!(bucket_day(&b).as_deref(), Some("2026-09-13"));
        // 两个时间都没有 → 没有归属日。
        b.started_at_secs = 0.0;
        assert_eq!(bucket_day(&b), None);
    }

    #[test]
    fn backfills_buckets_billed_after_the_ledger_tail() {
        let ledger = vec![ledger_row("2026-10-08", "MiniMax-M3", 100.0)];
        let mut fresh = with_provider(
            s("s-new", local_noon(2026, 10, 19), 40.0, 5.0, 10.0, 0.5),
            "opencode-go",
            "deepseek-v4.1-flash",
        );
        fresh.first_seen_secs = Some(local_noon(2026, 10, 20));
        // 跨越缺口的老桶：首次计费日早于台账尾部，整段压过去会与历史重复 → 不补。
        let mut old = with_provider(
            s("s-old", local_noon(2026, 9, 13), 70.0, 0.0, 0.0, 0.0),
            "opencode-go",
            "deepseek-v4.1-flash",
        );
        old.first_seen_secs = Some(local_noon(2026, 10, 1));
        // 同一 (日, 模型) 出现两次：只补一次，另一条记为跳过。
        let dup = fresh.clone();
        // 尾部补齐只处理「本轮之前已有基线」的桶（新桶由 plan_updates 种子）。
        let mut baselines = BTreeMap::new();
        baselines.insert(baseline_key(&fresh), (0.0, 0.0, 0.0, 0.0));
        baselines.insert(baseline_key(&old), (0.0, 0.0, 0.0, 0.0));

        let (writes, skipped) = backfill_tail(
            &[fresh, old, dup],
            &ledger,
            Some("2026-10-08"),
            &baselines,
            &[],
        );
        assert_eq!(writes.len(), 1, "同一 (日, 模型) 只能补一行");
        assert_eq!(writes[0].0, "2026-10-20");
        assert_eq!(writes[0].1, "opencode-go\u{1f}deepseek-v4.1-flash");
        assert_eq!(writes[0].2, 40.0);
        assert_eq!(skipped, 1);
    }

    /// 新桶没有基线：种子由 plan_updates 负责，backfill 不能再补一遍
    /// （`started_at` 与 `first_seen` 常常同一天，两边都写就是整段翻倍）。
    #[test]
    fn backfill_skips_new_buckets_without_a_baseline() {
        let mut fresh = with_provider(
            s("s-new", local_noon(2026, 10, 19), 40.0, 5.0, 10.0, 0.5),
            "opencode-go",
            "deepseek-v4.1-flash",
        );
        fresh.first_seen_secs = Some(local_noon(2026, 10, 20));
        let ledger = vec![ledger_row("2026-10-08", "MiniMax-M3", 100.0)];
        let (writes, _) = backfill_tail(
            std::slice::from_ref(&fresh),
            &ledger,
            Some("2026-10-08"),
            &BTreeMap::new(),
            &[],
        );
        assert!(writes.is_empty(), "无基线的桶必须留给 plan_updates 种子");
    }

    /// 旧版基线只按裸 `sid` 记：升级后新键查不到，但命中旧键时只能重建键，
    /// 不能把整段累计再种子一遍——否则升级首轮历史直接翻倍。
    #[test]
    fn legacy_session_baseline_migrates_without_reseeding() {
        let started = 1767312000.0;
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.3)];
        let mut base = BTreeMap::new();
        base.insert("a".to_string(), (100.0, 50.0, 20.0, 0.3));
        let (writes, nb) = plan_updates(&sessions, &base, "2026-09-25", false, false, None);
        assert!(writes.is_empty(), "旧 sid 基线命中时不得整段重种");
        assert_eq!(nb.len(), 1);
        assert_eq!(nb[0].0, baseline_key(&sessions[0]), "要重建新键");
    }

    /// 同一会话在 Windows / WSL 两份副本里各出现一次：合并成一条，保留更新
    /// （累计更大）的那份。否则基线表主键冲突会让整批基线保存回滚，之后每轮
    /// 都按旧基线重复记同一段增量。
    #[test]
    fn duplicate_session_rows_keep_the_fresher_copy() {
        let stale = with_provider(s("dup", 100.0, 10.0, 1.0, 2.0, 0.0), "opencode-go", "m");
        let fresh = with_provider(s("dup", 100.0, 30.0, 2.0, 4.0, 0.0), "opencode-go", "m");
        let rows = dedupe_sessions(vec![stale, fresh]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input, 30.0);
        assert_eq!(rows[0].output, 4.0);

        // 不同账户/模型是不同桶，不能合并。
        let other = with_provider(s("dup", 100.0, 30.0, 2.0, 4.0, 0.0), "deepseek", "m");
        assert_eq!(dedupe_sessions(vec![rows[0].clone(), other]).len(), 2);
    }

    #[test]
    fn backfill_is_idempotent_once_the_tail_moves_forward() {
        let mut fresh = with_provider(
            s("s-new", local_noon(2026, 10, 19), 40.0, 5.0, 10.0, 0.5),
            "opencode-go",
            "deepseek-v4.1-flash",
        );
        fresh.first_seen_secs = Some(local_noon(2026, 10, 20));
        // 第一轮：尾部是 10-08 → 补一行。
        let ledger = vec![ledger_row("2026-10-08", "MiniMax-M3", 100.0)];
        let mut baselines = BTreeMap::new();
        baselines.insert(baseline_key(&fresh), (0.0, 0.0, 0.0, 0.0));
        let (first, _) = backfill_tail(
            std::slice::from_ref(&fresh),
            &ledger,
            Some("2026-10-08"),
            &baselines,
            &[],
        );
        assert_eq!(first.len(), 1);
        // 第二轮：那行已经落账，尾部前移到 10-20 → 同一桶不再补。
        let mut ledger2 = ledger;
        ledger2.push(ledger_row(&first[0].0, &first[0].1, first[0].2));
        let last = ledger2.iter().map(|r| r.0.clone()).max();
        let (second, _) =
            backfill_tail(&[fresh], &ledger2, last.as_deref(), &baselines, &[]);
        assert!(second.is_empty(), "已补过的日期不得重复补");
    }

    #[test]
    fn backfill_does_nothing_without_a_ledger_tail() {
        let s1 = with_provider(s("s1", local_noon(2026, 10, 20), 1.0, 0.0, 0.0, 0.0), "opencode-go", "m1");
        let (writes, skipped) = backfill_tail(&[s1], &[], None, &BTreeMap::new(), &[]);
        assert!(writes.is_empty());
        assert_eq!(skipped, 0);
    }
    #[test]
    fn first_seen_is_read_when_the_column_exists() {
        let conn = sample_db();
        // 补上新版本才有的列，并给一个可辨识的首次计费时刻。
        conn.execute_batch(
            "ALTER TABLE session_model_usage ADD COLUMN first_seen REAL;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, started_at, model) VALUES ('s1', 100.0, 'deepseek-v4.1-flash')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_model_usage
               (session_id, model, billing_provider, input_tokens, output_tokens, first_seen)
             VALUES ('s1', 'deepseek-v4.1-flash', 'opencode-go', 10, 1, 111.0)",
            [],
        )
        .unwrap();

        let rows = read_usage_rows(&conn).expect("readable");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].first_seen_secs, Some(111.0));
    }

    #[test]
    fn missing_first_seen_column_does_not_break_the_read() {
        // sample_db 本身就没有 first_seen：整库读数不得因此返回 None（那会让
        // 扫描器以为库不可读，从而整轮不记账）。
        let conn = sample_db();
        conn.execute(
            "INSERT INTO sessions (id, started_at, model) VALUES ('s1', 100.0, 'deepseek-v4.1-flash')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens)
             VALUES ('s1', 'deepseek-v4.1-flash', 'opencode-go', 10)",
            [],
        )
        .unwrap();
        let rows = read_usage_rows(&conn).expect("旧库也要能读");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].first_seen_secs, None);
    }
    fn base_of(nb: &[(String, f64, f64, f64, f64)]) -> BTreeMap<String, (f64, f64, f64, f64)> {
        nb.iter()
            .map(|(k, i, cr, o, c)| (k.clone(), (*i, *cr, *o, *c)))
            .collect()
    }

    /// 建一个最小的 Hermes 库（sessions + session_model_usage）用于读数测试。
    fn sample_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY, started_at REAL, model TEXT, billing_provider TEXT,
                input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
                cache_read_tokens INTEGER DEFAULT 0, reasoning_tokens INTEGER DEFAULT 0,
                estimated_cost_usd REAL, actual_cost_usd REAL);
             CREATE TABLE session_model_usage (
                session_id TEXT NOT NULL, model TEXT NOT NULL,
                billing_provider TEXT NOT NULL DEFAULT '',
                billing_base_url TEXT NOT NULL DEFAULT '',
                billing_mode TEXT NOT NULL DEFAULT '', task TEXT NOT NULL DEFAULT '',
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0,
                actual_cost_usd REAL NOT NULL DEFAULT 0);",
        )
        .expect("schema");
        conn
    }

    #[test]
    fn model_key_roundtrip_keeps_legacy_plain_names() {
        let key = model_key("opencode-go", "deepseek-v4.1-flash");
        assert_eq!(split_model_key(&key), ("opencode-go", "deepseek-v4.1-flash"));
        // 旧台账没有账户：裸模型名拆出来是空账户，不会被误读。
        assert_eq!(split_model_key("MiniMax-M3"), ("", "MiniMax-M3"));
    }

    #[test]
    fn read_usage_rows_reads_bucketed_accounts_and_legacy_sessions() {
        let conn = sample_db();
        conn.execute(
            "INSERT INTO sessions (id, started_at, model, billing_provider, input_tokens, output_tokens, cache_read_tokens)
             VALUES ('s1', 100.0, 'deepseek-v4.1-flash', 'deepseek', 10, 1, 2)",
            [],
        )
        .unwrap();
        // 同一会话同一模型，两个计费账户 → 必须读成两行。
        for provider in ["deepseek", "opencode-go"] {
            conn.execute(
                "INSERT INTO session_model_usage
                   (session_id, model, billing_provider, input_tokens, output_tokens, cache_read_tokens, actual_cost_usd)
                 VALUES (?1, 'deepseek-v4.1-flash', ?2, 10, 1, 2, 0)",
                rusqlite::params!["s1", provider],
            )
            .unwrap();
        }
        // 没有 per-model 行的旧会话走 sessions 兜底。
        conn.execute(
            "INSERT INTO sessions (id, started_at, model, billing_provider, input_tokens, output_tokens, cache_read_tokens)
             VALUES ('s2', 200.0, 'MiniMax-M3', 'minimax-cn', 7, 3, 0)",
            [],
        )
        .unwrap();

        let rows = read_usage_rows(&conn).expect("readable");
        assert_eq!(rows.len(), 3, "两行账户分桶 + 一行旧会话兜底");
        let mut providers: Vec<&str> = rows.iter().map(|r| r.provider.as_str()).collect();
        providers.sort_unstable();
        assert_eq!(providers, vec!["deepseek", "minimax-cn", "opencode-go"]);
        assert!(rows.iter().all(|r| r.sid != "s1" || r.provider != ""));
    }

    #[test]
    fn read_usage_rows_returns_none_when_schema_missing() {
        // 库能打开但没有 sessions/session_model_usage（不是 Hermes 库 / 损坏）：
        // 必须返回 None，让调用方跳过本轮记账，而不是当成"空库"清空基线。
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_usage_rows(&conn).is_none());
    }

    #[test]
    fn seeds_new_session_to_start_day_then_deltas_to_today() {
        // 一个会话在 2026-01-02 开始，第一次扫描时累计 (100,50,20)。
        let started = 1767312000.0; // 2026-01-02T00:00:00Z-ish
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.0)];

        // 首次：无基线 → 种子归到 start 日，基线=当前。
        let base0 = BTreeMap::new();
        let (attrs0, nb0) = plan_updates(&sessions, &base0, "2026-09-24", false, false, None);
        assert_eq!(attrs0.len(), 1);
        assert_eq!(attrs0[0].0, day_from_epoch_ms((started * 1000.0) as i64)); // seed day
        assert_eq!(attrs0[0].3, 50.0); // cache_read
        assert_eq!(nb0[0].1, 100.0); // 基线 input

        // 第二次：会话增长到 (130,60,30) → 增量 (30,10,10) 归到今天，新基线=当前。
        let grown = vec![s("a", started, 130.0, 60.0, 30.0, 0.1)];
        let base1 = base_of(&nb0);
        let (attrs1, nb1) = plan_updates(&grown, &base1, "2026-09-24", false, false, None);
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
        base.insert(baseline_key(&s("a", 0.0, 100.0, 50.0, 20.0, 0.0)), (100.0, 50.0, 20.0, 0.0));
        let (attrs, _nb) = plan_updates(&sessions, &base, "2026-09-24", false, false, None);
        assert_eq!(attrs.len(), 0);
    }

    #[test]
    fn force_seed_restores_history_even_when_baselines_exist() {
        // 迁移场景：有基线（历史未被台账记过）但强制全种子 → 整段累计仍归到 start 日。
        // 这修复"hermes 历史归零"：台账为空时 force_seed_all=true，忽略已有基线补种子。
        let started = 1767312000.0; // 2026-01-02
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.3)];
        let mut base = BTreeMap::new();
        base.insert(baseline_key(&sessions[0]), (100.0, 50.0, 20.0, 0.3)); // 基线已存在
        let (attrs, nb) = plan_updates(&sessions, &base, "2026-09-25", true, false, None);
        assert_eq!(attrs.len(), 1); // 即便如此也要种子历史
        assert_eq!(attrs[0].0, day_from_epoch_ms((started * 1000.0) as i64));
        assert_eq!(nb[0].1, 100.0); // 基线仍记录当前累计，防下一次重复种子
    }

    #[test]
    fn duplicate_scan_second_time_counts_nothing() {
        // 同一会话值不变、再次扫描（常见：watch 多轮）→ 无增量，不出新归属。
        let started = 1767312000.0;
        let s1 = vec![s("a", started, 100.0, 50.0, 20.0, 0.0)];
        let (attrs0, nb0) = plan_updates(&s1, &BTreeMap::new(), "2026-09-24", false, false, None);
        assert_eq!(attrs0.len(), 1);
        let base = base_of(&nb0);
        let (attrs1, _nb1) = plan_updates(&s1, &base, "2026-09-24", false, false, None);
        assert_eq!(attrs1.len(), 0);
    }

    #[test]
    fn lost_baseline_does_not_reseed_existing_history() {
        // P0 回归：基线表丢光（旧版本在库不可读时清空过）而台账仍有历史 →
        // 本轮不得把整段累计重新种子，否则同一段用量被记两遍、历史翻倍。
        let started = 1767312000.0;
        let sessions = vec![s("a", started, 100.0, 50.0, 20.0, 0.3)];
        let (writes, nb) = plan_updates(&sessions, &BTreeMap::new(), "2026-09-24", false, true, None);
        assert!(writes.is_empty(), "台账已有历史时禁止重种");
        assert_eq!(nb.len(), 1, "基线仍然要重建，后续增量才能继续");

        // 下一轮会话增长 → 只记真实增量，历史不重复。
        let grown = vec![s("a", started, 130.0, 60.0, 30.0, 0.3)];
        let base = base_of(&nb);
        let (writes2, _) = plan_updates(&grown, &base, "2026-09-24", false, false, None);
        assert_eq!(writes2.len(), 1);
        assert_eq!(writes2[0].0, "2026-09-24");
        assert_eq!(writes2[0].2, 30.0);
    }

    #[test]
    fn lost_baseline_still_seeds_sessions_started_after_the_ledger() {
        // P0 的另一半：基线丢光、台账止于 2026-10-08 时，10-09 之后才开始的新会话在
        // 台账里不可能有历史行，必须补种（否则断流那段时间永久缺失）；更早开始的会话
        // 仍然只重建基线，宁可留空洞也不让同一段累计被记两遍。
        let older = 1791129600.0; // 2026-10-05，早于台账最后一天
        let newer = 1792425600.0; // 2026-10-20，晚于台账最后一天
        let sessions = vec![
            s("old", older, 100.0, 50.0, 20.0, 0.3),
            s("new", newer, 40.0, 10.0, 5.0, 0.1),
        ];
        let (writes, nb) = plan_updates(
            &sessions,
            &BTreeMap::new(),
            "2026-10-24",
            false,
            true,
            Some("2026-10-08"),
        );
        assert_eq!(writes.len(), 1, "只补种晚于台账最后一天的那一个会话");
        assert_eq!(writes[0].0, "2026-10-20");
        assert_eq!(writes[0].2, 40.0);
        assert_eq!(nb.len(), 2, "两个会话都要重建基线");
    }

    #[test]
    fn provider_is_encoded_into_ledger_model_key() {
        // P1：计费账户进台账键，同一模型走不同账户不会被合并。
        let session = with_provider(s("a", 1767312000.0, 10.0, 0.0, 1.0, 0.0), "opencode-go", "deepseek-v4.1-flash");
        let (writes, nb) = plan_updates(&[session], &BTreeMap::new(), "2026-09-24", false, false, None);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1, model_key("opencode-go", "deepseek-v4.1-flash"));
        assert!(split_model_key(&writes[0].1) == ("opencode-go", "deepseek-v4.1-flash"));
        assert_eq!(nb[0].0, format!("a{KEY_SEP}opencode-go{KEY_SEP}deepseek-v4.1-flash"));
    }

    #[test]
    fn same_model_two_providers_kept_in_separate_report_rows() {
        let ledger = vec![
            ("2026-09-24".to_string(), model_key("deepseek", "deepseek-v4.1-flash"), 10.0, 0.0, 1.0, 0.0, 0),
            ("2026-09-24".to_string(), model_key("opencode-go", "deepseek-v4.1-flash"), 20.0, 0.0, 2.0, 0.0, 0),
        ];
        let report = build_report(&ledger, "now");
        assert_eq!(report.models.len(), 2, "同模型两个账户必须两行");
        assert_eq!(report.total_tokens, 33.0, "日总量仍是一个会话的合计");
        let opencode = report
            .models
            .iter()
            .find(|m| m.provider == "opencode-go")
            .expect("opencode-go row");
        assert_eq!(opencode.model, "deepseek-v4.1-flash");
        assert_eq!(opencode.total_tokens, 22.0);
        let deepseek = report
            .models
            .iter()
            .find(|m| m.provider == "deepseek")
            .expect("deepseek row");
        assert_eq!(deepseek.total_tokens, 11.0);
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
        assert_eq!(usage.provider, "", "旧台账行没有账户");
        assert_eq!(usage.total_tokens, 178.0); // 100+50+20+5+1+2
        assert_eq!(usage.currency, "USD"); // 币种归一到 USD，不再是 "estimated"/"reported"
        assert!((usage.cost - 0.3).abs() < 1e-9);
        assert!(!usage.cost_estimated);
    }
}
