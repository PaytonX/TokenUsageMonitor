//! OpenCode CLI 本机用量扫描器。
//!
//! 为什么必须有这个扫描器：OpenCode CLI 把每次调用的用量记在
//! `~/.local/share/opencode/opencode.db`（SQLite，WAL 模式）的 `message` 表里，
//! 一条 assistant 消息的 `data` JSON 自带 `providerID` / `modelID` / `tokens` /
//! `cost`。OpenCode 既没有可轮询的余额接口，也没有别的账本文件——不读这张表，
//! 用户经它跑掉的全部 token 在趋势/模型视图里就是空白。
//!
//! 为什么不用 `session` 表的汇总列：`session` 上的 `model` / `cost` /
//! `tokens_input` 等是该会话的**重复汇总**，与 `message` 行相加会双计。这里只认
//! `message` 表的逐条记录，与其它扫描器"一行一次用量"的口径一致。
//!
//! 为什么桶键是 `(providerID, modelID)` 而不是模型名：模型名回答"这是谁家的
//! 模型"，providerID 回答"账单落在谁家"。`deepseek-v4.1-flash` 经 OpenCode Go
//! （providerID `opencode-go` / `opencode-zen`）调用时账单在 OpenCode，只按模型名
//! 归因会把它算进 DeepSeek——用户实际观测到的"趋势页 Provider 用量都被算进
//! DeepSeek"就是这个口径错误。扫描器只如实记录两个字段，归因顺序（provider 列
//! 优先、模型名兜底）由前端决定，见 `src/lib/model-provider.ts`。
//!
//! 为什么走 [`super::sqlite_ro::open_readonly`]：CLI 常驻时库被锁、数据还在
//! `-wal` 里，普通只读打开可能失败或读到 checkpoint 之前的旧快照，而扫描结果走
//! 全量 REPLACE——陈旧快照会覆盖账本里正确的数字。统一交给 `sqlite_ro`（必要时
//! 复制 db+`-wal`+`-shm` 副本再读），打不开就本轮不出数，比写脏数据安全。

use super::{day_from_epoch_ms, finalize_days, LocalModelUsage, LocalToolReport};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// 工具 id（`usage_daily.source`、watch 增量扫描与前端胶囊共用）。
pub const TOOL_ID: &str = "opencode";
const TOOL_NAME: &str = "OpenCode";
/// 与其它扫描器一致的滚动窗口（天）。
const KEEP_DAYS: usize = 90;
/// 数据目录相对用户 home 的位置：`~/.local/share/opencode`。
const DATA_DIR: &str = ".local/share/opencode";
/// 范围内的库文件。同目录里轮转出去的 `opencode-<时间戳>.db` 是历史快照，
/// 读两份库会把同一批消息重复计数，所以只认当前库。
const DB_FILE: &str = "opencode.db";
/// 只取滚动窗口内的消息：`data` 是 JSON，日期过滤只能在 SQL 侧用
/// `time_created` 做，解析归 Rust。
const QRY: &str = "SELECT id, session_id, time_created, data FROM message WHERE time_created >= ?1";

/// 一条 assistant 消息解析出来的用量。`provider` / `model` 原样保留，不做
/// 前缀裁剪或路由哨兵改写（那是前端归因层的事）。
#[derive(Debug, Clone)]
struct ParsedMessage {
    provider: String,
    model: String,
    /// 本地日 `YYYY-MM-DD`。
    day: String,
    input: f64,
    output: f64,
    cache_read: f64,
    /// 消息自带的成本（USD）。OpenCode 只对部分 provider 记 `cost`，缺省为 `None`。
    cost: Option<f64>,
}

/// 单个 `(provider, model)` 桶的累计。
#[derive(Debug, Default)]
struct AccountAgg {
    days: BTreeMap<String, (f64, f64, f64)>,
    /// 消息自报的成本合计（USD）。
    cost_recorded: f64,
    /// 消息没报成本时按价格表估算出来的合计（USD）。
    cost_estimated: f64,
}

impl AccountAgg {
    fn add(&mut self, m: &ParsedMessage) {
        let e = self.days.entry(m.day.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += m.input;
        e.1 += m.cache_read;
        e.2 += m.output;
        match m.cost {
            Some(c) => self.cost_recorded += c,
            // 与 dsh.rs 同口径：消息没给成本才估算，估算不出来就保持 0（不编数字）。
            None => {
                if let Some(c) = estimate_cost(m) {
                    self.cost_estimated += c;
                }
            }
        }
    }
}

/// 扫描过程的全部累计。拆成独立结构是为了让"解析 → 归桶 → 出报告"三段都能
/// 在测试里直接驱动：纯 JSON 用例不需要任何数据库。
#[derive(Debug, Default)]
struct Agg {
    by_day: BTreeMap<String, (f64, f64, f64)>,
    accounts: BTreeMap<(String, String), AccountAgg>,
    /// 去重后的会话数（`message.session_id`）；同会话多轮只算一次。
    sessions: HashSet<String>,
    /// 计入用量的 assistant 行数。
    rows: u64,
    /// `data` 不是合法 JSON 的行数（只用于 debug 留痕，不影响结果）。
    bad_json: u64,
    /// assistant 行但缺 `tokens` 的行数（中断/未完成的轮次）。
    missing_tokens: u64,
}

impl Agg {
    fn add(&mut self, m: ParsedMessage, session_id: &str) {
        let e = self.by_day.entry(m.day.clone()).or_insert((0.0, 0.0, 0.0));
        e.0 += m.input;
        e.1 += m.cache_read;
        e.2 += m.output;

        self.accounts
            .entry((m.provider.clone(), m.model.clone()))
            .or_default()
            .add(&m);

        self.sessions.insert(session_id.to_string());
        self.rows += 1;
    }
}

/// 非负有限数值；缺失/负数/非数字一律按 0 处理（JSON 里这些字段偶尔是 null）。
fn num(v: Option<&serde_json::Value>) -> f64 {
    v.and_then(|x| x.as_f64())
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(0.0)
}

/// 消息没报成本时的兜底估算。价格表未知模型返回 `None`，调用方据此保持 0。
fn estimate_cost(m: &ParsedMessage) -> Option<f64> {
    crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
        input: m.input,
        cache_read: m.cache_read,
        output: m.output,
        model_id: Some(m.model.clone()),
    })
    .filter(|c| c.is_finite() && *c > 0.0)
}

/// 从一条已解析的 `data` JSON 提取用量。返回 `None` 表示"这行不该计入"
/// （用户轮次、无 `tokens` 的中断轮次、时间戳越界）。
///
/// 日期取 `time.completed`（本轮真正产生的时刻）优先，缺失才退回
/// `time_created`——实测有 3 行没有 `completed`、2 行 `completed` 与创建不同日，
/// 按完成时刻归日才与账单当天对齐。
fn parse_value(v: &serde_json::Value, time_created: i64) -> Option<ParsedMessage> {
    if v.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    let tokens = v.get("tokens")?;
    let completed = v
        .get("time")
        .and_then(|t| t.get("completed"))
        .and_then(|c| c.as_i64())
        .filter(|ms| *ms > 0)
        .unwrap_or(time_created);
    let day = day_from_epoch_ms(completed);
    if day.is_empty() {
        return None;
    }
    let provider = v
        .get("providerID")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let model = v
        .get("modelID")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    // 与其它扫描器同口径：没有模型名的真实用量归入"未标记模型"，不伪造模型名，
    // 也不丢弃（否则模型页合计与趋势页对不上）。
    let model = if model.is_empty() {
        super::UNCLASSIFIED_MODEL.to_string()
    } else {
        model.to_string()
    };
    let cost = v
        .get("cost")
        .and_then(|x| x.as_f64())
        .filter(|c| c.is_finite() && *c > 0.0);
    Some(ParsedMessage {
        provider,
        model,
        day,
        input: num(tokens.get("input")),
        output: num(tokens.get("output")),
        // 缓存读单独一桶，绝不能并进 input：价格表里两者单价不同，
        // 并进 input 会把缓存折扣丢掉（dsh.rs 的注释同样强调这一点）。
        cache_read: num(tokens.get("cache").and_then(|c| c.get("read"))),
        cost,
    })
}

/// `data` 字符串版的解析入口，供测试直接喂 JSON。
fn parse_message(data: &str, time_created: i64) -> Option<ParsedMessage> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    parse_value(&v, time_created)
}

/// 从一个已完成的读取连接里查询并归桶，供 `scan_db` 与测试共用。
fn scan_conn(conn: &Connection, agg: &mut Agg, cutoff_ms: i64) {
    let mut stmt = match conn.prepare(QRY) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                target: "tum.local.opencode",
                error = %e,
                "message 表不可读，本轮跳过 OpenCode"
            );
            return;
        }
    };
    let mut rows = match stmt.query([cutoff_ms]) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(
                target: "tum.local.opencode",
                error = %e,
                "message 查询失败，本轮跳过 OpenCode"
            );
            return;
        }
    };
    while let Ok(Some(row)) = rows.next() {
        let id: String = row.get(0).unwrap_or_default();
        let session_id: String = row.get(1).unwrap_or_default();
        let time_created: i64 = row.get(2).unwrap_or_default();
        let data: String = row.get(3).unwrap_or_default();
        match parse_message(&data, time_created) {
            Some(msg) => agg.add(msg, &session_id),
            None => {
                // 用户轮次本来就没有 tokens（正常跳过）；只有 assistant 行缺
                // tokens 或 JSON 损坏才是数据异常，计数并留一条 debug 痕迹，
                // 不打断整轮扫描。
                match serde_json::from_str::<serde_json::Value>(&data) {
                    Err(_) => {
                        agg.bad_json += 1;
                        tracing::debug!(
                            target: "tum.local.opencode",
                            message_id = %id,
                            "data 不是合法 JSON，跳过该行"
                        );
                    }
                    Ok(v) => {
                        if v.get("role").and_then(|r| r.as_str()) == Some("assistant")
                            && v.get("tokens").is_none()
                        {
                            agg.missing_tokens += 1;
                            tracing::debug!(
                                target: "tum.local.opencode",
                                message_id = %id,
                                "assistant 消息没有 tokens（中断轮次），不计用量"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// 滚动窗口的 epoch-ms 下界。
fn cutoff_ms() -> i64 {
    chrono::Utc::now().timestamp_millis() - (KEEP_DAYS as i64) * 86_400_000
}

/// 定位库文件。
///
/// 默认 `%USERPROFILE%\.local\share\opencode\opencode.db`；用户声明的额外数据根
/// 目录按同样的相对路径命中，精确覆盖则可以是目录（就地找 `opencode.db`）或
/// 直接指向库文件。多个候选目录并存时按新鲜度取最近活跃的那个——搬迁后旧位置
/// 常留一份停更副本，读它会表现为"近几天用量为 0"（见 `roots` 模块注释）。
pub(super) fn db_path() -> Option<PathBuf> {
    if let Some(p) = super::roots::override_for(TOOL_ID) {
        if p.is_file() {
            return Some(p);
        }
    }
    // 先按"目录里真的有 opencode.db"过滤，再按新鲜度裁决：只挑目录会把一个
    // 恰好最近被动过、但没库文件的目录排到前面，结果本轮读到 0 行。
    let candidates: Vec<PathBuf> = super::roots::windows_candidates(DATA_DIR, TOOL_ID)
        .into_iter()
        .filter(|dir| dir.join(DB_FILE).is_file())
        .collect();
    let dir = super::roots::pick_freshest(&candidates)?;
    Some(dir.join(DB_FILE))
}

/// 把累计结果收成一份报告。
///
/// 不复用 [`super::finalize_models`]：它按模型名建键并且把 `provider` 写死为空串，
/// 正好丢掉本扫描器唯一在意的那一维（同一模型经不同账户调用必须分行）。
fn finalize(agg: Agg) -> LocalToolReport {
    let daily = finalize_days(agg.by_day, KEEP_DAYS);
    let total_tokens: f64 = daily.iter().map(|d| d.total).sum();

    let mut models: Vec<LocalModelUsage> = Vec::with_capacity(agg.accounts.len());
    for ((provider, model), acc) in agg.accounts {
        let series = finalize_days(acc.days, KEEP_DAYS);
        let model_total: f64 = series.iter().map(|d| d.total).sum();
        if model_total <= 0.0 {
            continue;
        }
        let cost = acc.cost_recorded + acc.cost_estimated;
        models.push(LocalModelUsage {
            model,
            provider,
            total_tokens: model_total,
            cost,
            currency: if cost > 0.0 {
                "USD".to_string()
            } else {
                String::new()
            },
            cost_estimated: acc.cost_estimated > 0.0,
            daily: series,
        });
    }
    models.sort_by(|a, b| {
        b.total_tokens
            .partial_cmp(&a.total_tokens)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    LocalToolReport {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        daily,
        total_tokens,
        session_count: agg.sessions.len() as u64,
        project_count: 0,
        scanned_at: chrono::Utc::now().to_rfc3339(),
        models,
    }
}

/// 扫描指定库文件（`scan` 的实体，测试也直接用它）。打不开就是空报告，
/// 绝不 panic、也不写任何东西。
fn scan_db(path: &Path) -> LocalToolReport {
    let mut agg = Agg::default();
    if let Some(db) = super::sqlite_ro::open_readonly(path) {
        scan_conn(&db, &mut agg, cutoff_ms());
    }
    tracing::debug!(
        target: "tum.local.opencode",
        path = %path.display(),
        rows = agg.rows,
        sessions = agg.sessions.len(),
        days = agg.by_day.len(),
        models = agg.accounts.len(),
        bad_json = agg.bad_json,
        missing_tokens = agg.missing_tokens,
        "OpenCode 用量扫描完成"
    );
    finalize(agg)
}

/// 扫描 OpenCode CLI 的本地用量库，按本地日 / (provider, model) 聚合。
pub fn scan() -> LocalToolReport {
    match db_path() {
        Some(path) => scan_db(&path),
        None => finalize(Agg::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定时间点（2026 年中），只用于纯 JSON 用例；与滚动窗口无关。
    const T0: i64 = 1_780_000_000_000;

    /// 造一条 assistant 消息的 `data` JSON。
    fn assistant_json(
        provider: &str,
        model: &str,
        completed: i64,
        input: f64,
        output: f64,
        cache_read: f64,
        cost: Option<f64>,
    ) -> String {
        let mut v = serde_json::json!({
            "role": "assistant",
            "providerID": provider,
            "modelID": model,
            "time": { "created": completed, "completed": completed },
            "tokens": {
                // total 是 OpenCode 自己写的汇总（含 cache.write），这里故意写错
                // 来证明扫描器按明细重算、不读这个字段。
                "total": 999_999_999,
                "input": input,
                "output": output,
                "reasoning": 0,
                "cache": { "write": 0, "read": cache_read }
            }
        });
        if let Some(c) = cost {
            v["cost"] = serde_json::json!(c);
        }
        v.to_string()
    }

    fn bucket(rows: &[(&str, i64, &str)]) -> Agg {
        let mut agg = Agg::default();
        for (data, created, session) in rows {
            let msg = parse_message(data, *created).expect("row should parse");
            agg.add(msg, session);
        }
        agg
    }

    /// 用户轮次即使自带 tokens 也绝不能计入：它没有账单，计入就是虚增。
    #[test]
    fn user_rows_with_tokens_do_not_count() {
        let user = serde_json::json!({
            "role": "user",
            "modelID": "MiniMax-M2.7",
            "providerID": "minimax",
            "tokens": { "input": 5000, "output": 5000, "cache": { "read": 5000 } }
        })
        .to_string();
        assert!(parse_message(&user, T0).is_none());

        let good = assistant_json("minimax", "MiniMax-M2.7", T0, 100.0, 10.0, 0.0, None);
        let mut agg = Agg::default();
        agg.add(
            parse_message(&good, T0).expect("assistant row parses"),
            "s1",
        );
        assert_eq!(agg.rows, 1);
        assert_eq!(agg.sessions.len(), 1);
        let day = agg.by_day.values().next().unwrap();
        assert_eq!(*day, (100.0, 0.0, 10.0));
    }

    /// 核心回归：同一模型名在两个 provider 下必须分成两行，且逐日总量不重复。
    /// 前端按模型名归档因时，这正是把账算进别家的口径错误。
    #[test]
    fn same_model_under_two_providers_splits_rows() {
        let a = assistant_json(
            "opencode-go",
            "deepseek-v4.1-flash",
            T0,
            1000.0,
            100.0,
            2000.0,
            Some(0.5),
        );
        let b = assistant_json("deepseek", "deepseek-v4.1-flash", T0, 10.0, 1.0, 0.0, None);
        let report = finalize(bucket(&[(&a, T0, "s1"), (&b, T0, "s2")]));

        assert_eq!(report.models.len(), 2, "同模型不同 provider 必须分行");
        let go = report
            .models
            .iter()
            .find(|m| m.provider == "opencode-go")
            .expect("opencode-go bucket");
        assert_eq!(go.model, "deepseek-v4.1-flash");
        assert_eq!(go.total_tokens, 3100.0);
        assert_eq!(go.cost, 0.5);
        assert_eq!(go.currency, "USD");
        assert!(!go.cost_estimated);

        let direct = report
            .models
            .iter()
            .find(|m| m.provider == "deepseek")
            .expect("deepseek bucket");
        assert_eq!(direct.total_tokens, 11.0);

        // 逐日总量 = 两条消息之和，不是同一模型的两倍。
        assert_eq!(report.total_tokens, 3111.0);
        assert_eq!(report.daily.len(), 1);
        assert_eq!(report.daily[0].total, 3111.0);
        assert_eq!(report.session_count, 2);
    }

    /// 按 `time.completed` 的本地日归桶：相差 48 小时的两条消息落在两天。
    #[test]
    fn messages_on_different_days_make_two_day_entries() {
        let day1 = T0;
        let day2 = T0 + 48 * 3_600_000;
        let a = assistant_json("minimax", "MiniMax-M3", day1, 100.0, 10.0, 0.0, None);
        let b = assistant_json("minimax", "MiniMax-M3", day2, 200.0, 20.0, 0.0, None);
        let report = finalize(bucket(&[(&a, day1, "s1"), (&b, day1, "s1")]));

        assert_eq!(report.daily.len(), 2);
        assert_eq!(report.daily[0].date, day_from_epoch_ms(day1));
        assert_eq!(report.daily[1].date, day_from_epoch_ms(day2));
        assert_ne!(report.daily[0].date, report.daily[1].date);
        assert_eq!(report.daily[0].total, 110.0);
        assert_eq!(report.daily[1].total, 220.0);
        assert_eq!(report.total_tokens, 330.0);
    }

    /// `cache_read` 只来自 `tokens.cache.read`；`reasoning` 不得混进 output
    /// （它已含在 output 里，另计就是双计）。
    #[test]
    fn cache_read_comes_from_nested_cache_and_reasoning_is_not_added() {
        let data = serde_json::json!({
            "role": "assistant",
            "providerID": "minimax-cn",
            "modelID": "MiniMax-M2.7",
            "time": { "completed": T0 },
            "tokens": {
                "input": 300,
                "output": 40,
                "reasoning": 7000,
                "cache": { "write": 111, "read": 500 }
            }
        })
        .to_string();
        let report = finalize(bucket(&[(&data, T0, "s1")]));

        assert_eq!(report.daily.len(), 1);
        let d = &report.daily[0];
        assert_eq!(d.input, 300.0);
        assert_eq!(d.output, 40.0);
        assert_eq!(d.cache_read, 500.0);
        // 500 缓存读 + 300 输入 + 40 输出；reasoning/write 都不进 total。
        assert_eq!(d.total, 840.0);
        assert_eq!(report.models[0].total_tokens, 840.0);
    }

    /// 坏 JSON 行被跳过且不 panic：扫描继续，好行照常计入。
    #[test]
    fn malformed_json_rows_are_skipped() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        let good = assistant_json("minimax", "MiniMax-M3", T0, 100.0, 10.0, 0.0, None);
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data)
             VALUES ('bad', 's1', ?1, ?1, '{not json'),
                    ('good', 's1', ?1, ?1, ?2),
                    ('user', 's1', ?1, ?1, '{\"role\":\"user\"}')",
            rusqlite::params![T0, good],
        )
        .unwrap();

        let mut agg = Agg::default();
        scan_conn(&conn, &mut agg, 0);
        assert_eq!(agg.bad_json, 1);
        assert_eq!(agg.rows, 1);
        let report = finalize(agg);
        assert_eq!(report.total_tokens, 110.0);
    }

    /// 消息自报 `cost` 优先，且不标成估算；没报成本且价格表查不到时保持 0，
    /// 绝不编造金额（`COST_ESTIMATION_ENABLED=false` 时估算恒为 0）。
    #[test]
    fn recorded_cost_is_used_and_not_flagged_as_estimated() {
        let recorded = assistant_json(
            "opencode-zen",
            "totally-unpriced-model",
            T0,
            100.0,
            10.0,
            0.0,
            Some(0.125),
        );
        let unpriced = assistant_json(
            "minimax",
            "totally-unpriced-model",
            T0,
            10.0,
            1.0,
            0.0,
            None,
        );
        let report = finalize(bucket(&[(&recorded, T0, "s1"), (&unpriced, T0, "s2")]));

        let zen = report
            .models
            .iter()
            .find(|m| m.provider == "opencode-zen")
            .expect("opencode-zen bucket");
        assert_eq!(zen.cost, 0.125);
        assert_eq!(zen.currency, "USD");
        assert!(!zen.cost_estimated, "自报成本不得标成估算");

        let mini = report
            .models
            .iter()
            .find(|m| m.provider == "minimax")
            .expect("minimax bucket");
        assert_eq!(mini.cost, 0.0);
        assert_eq!(mini.currency, "");
        assert!(!mini.cost_estimated);
    }

    /// 端到端：真实临时库文件 → `sqlite_ro` 只读打开 → SQL 窗口过滤 → 报告。
    /// 同时验证 `time_created` 早于窗口的行被 SQL 挡掉，而不是靠 Rust 侧丢。
    #[test]
    fn reads_window_from_a_real_sqlite_file() {
        let path = std::env::temp_dir().join(format!(
            "tum-opencode-test-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);

        let now = chrono::Utc::now().timestamp_millis();
        let older = now - 3_600_000;
        let writer = Connection::open(&path).expect("create temp db");
        writer
            .execute_batch(
                "CREATE TABLE message (
                    id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
        let rows = [
            (
                "m1",
                "s1",
                older,
                assistant_json(
                    "opencode-go",
                    "deepseek-v4.1-flash",
                    older,
                    1000.0,
                    100.0,
                    2000.0,
                    Some(0.25),
                ),
            ),
            (
                "m2",
                "s2",
                now,
                assistant_json("minimax-cn", "MiniMax-M2.7", now, 300.0, 30.0, 5.0, None),
            ),
            (
                "m3",
                "s2",
                1_000,
                assistant_json("minimax-cn", "MiniMax-M2.7", 1_000, 777.0, 77.0, 0.0, None),
            ),
            ("u1", "s2", now, "{\"role\":\"user\"}".to_string()),
        ];
        for (id, session, created, data) in rows {
            writer
                .execute(
                    "INSERT INTO message (id, session_id, time_created, time_updated, data)
                     VALUES (?1, ?2, ?3, ?3, ?4)",
                    rusqlite::params![id, session, created, data],
                )
                .unwrap();
        }
        drop(writer);

        let report = scan_db(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(report.id, TOOL_ID);
        assert_eq!(report.models.len(), 2, "窗口外的 m3 不得出现");
        assert_eq!(report.session_count, 2);
        // m1: 1000+2000+100 = 3100；m2: 300+5+30 = 335（m3 的 777/77 被 SQL 挡掉）。
        assert_eq!(report.total_tokens, 3435.0);
        let go = report
            .models
            .iter()
            .find(|m| m.provider == "opencode-go")
            .expect("opencode-go bucket");
        assert_eq!(go.cost, 0.25);
        assert!(!go.cost_estimated);
        assert!(report.models.iter().any(|m| m.provider == "minimax-cn"));
    }
}
