//! SQLite persistence for daily usage snapshots.
//!
//! Some providers (MiniMax, DeepSeek) only expose the *current* remaining
//! quota / monthly aggregate, not a per-day breakdown. To still render a
//! heatmap for those providers, we capture a daily snapshot each time
//! `fetch_usage` runs and store it here.
//!
//! Volcano's `GetUsageDetails` API returns daily breakdown directly, so
//! Volcano providers return the heatmap inline rather than reading this store.

use crate::providers::{HeatmapCell, UsageUnit};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

pub struct Storage {
    conn: Mutex<Connection>,
}

/// 统一日账本（usage_daily）的一行。`model` 为空串 = 来源总量行；
/// 非空 = 该模型的分项行。
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageDailyRow {
    pub source: String,
    pub kind: String, // "tool" | "provider"
    pub date: String, // YYYY-MM-DD 本地日
    pub model: String,
    pub input: f64,
    pub cache_read: f64,
    pub output: f64,
    pub total: f64,
    pub unit: UsageUnit,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub cost_estimated: bool,
}

impl Storage {
    /// Open (or create) the SQLite database at `path`. Schema is migrated on
    /// first open. Designed for `app_data_dir()/token_usage_monitor.db`.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| "creating app data dir")?;
        }
        let conn = Connection::open(path).with_context(|| "opening sqlite db")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS daily_snapshots (
                provider_id   TEXT    NOT NULL,
                date          TEXT    NOT NULL,             -- YYYY-MM-DD
                value         REAL    NOT NULL,             -- used quota
                unit          TEXT    NOT NULL,             -- tokens|afp|cny|credits|percent
                captured_at   TEXT    NOT NULL,             -- ISO8601 UTC
                PRIMARY KEY (provider_id, date)
            );
            CREATE INDEX IF NOT EXISTS idx_daily_snapshots_provider_date
                ON daily_snapshots(provider_id, date DESC);
            CREATE TABLE IF NOT EXISTS provider_kv (
                provider_id TEXT NOT NULL,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (provider_id, key)
            );
            CREATE TABLE IF NOT EXISTS local_scan_cache (
                key         INTEGER NOT NULL PRIMARY KEY CHECK (key = 1),
                fingerprint TEXT NOT NULL,
                payload     TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hub_device (
                device_id  TEXT NOT NULL PRIMARY KEY,
                payload    TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_file_offset (
                tool   TEXT NOT NULL,
                path   TEXT NOT NULL,
                offset INTEGER NOT NULL,             -- 续读字节指针（停在完整行末尾）
                len    INTEGER NOT NULL,             -- 记录该 offset 时的文件长度
                mtime  INTEGER NOT NULL,             -- 记录时文件的 last_write_time / mtime
                PRIMARY KEY (tool, path)
            );
            CREATE TABLE IF NOT EXISTS hermes_baseline (
                session_id TEXT NOT NULL PRIMARY KEY,
                input      REAL NOT NULL,
                cache_read REAL NOT NULL,
                output     REAL NOT NULL,
                cost_usd   REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hermes_daily (
                date           TEXT NOT NULL,             -- YYYY-MM-DD 本地日
                model          TEXT NOT NULL,
                input          REAL NOT NULL,
                cache_read     REAL NOT NULL,
                output         REAL NOT NULL,
                cost           REAL NOT NULL,
                cost_estimated INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, model)
            );
            CREATE TABLE IF NOT EXISTS exchange_rates (
                code       TEXT NOT NULL PRIMARY KEY,   -- 目标币种代码，如 CNY；USD 恒为 1 不入库
                rate       REAL NOT NULL,               -- 每 1 USD 兑该币种的数值
                updated_at TEXT NOT NULL                -- ISO8601 UTC 抓取时间
            );
            CREATE TABLE IF NOT EXISTS usage_daily (
                source       TEXT NOT NULL,             -- 来源键：工具 id（claude-code/zcode/minimax-code…）或 provider 键（volcengine…）
                kind         TEXT NOT NULL,             -- 'tool' | 'provider'
                date         TEXT NOT NULL,             -- YYYY-MM-DD 本地日
                model        TEXT NOT NULL DEFAULT '',  -- '' = 来源总量行；非空 = 该模型的分项行
                input        REAL NOT NULL DEFAULT 0,
                cache_read   REAL NOT NULL DEFAULT 0,
                output       REAL NOT NULL DEFAULT 0,
                total        REAL NOT NULL DEFAULT 0,
                unit         TEXT NOT NULL DEFAULT 'tokens',
                cost         REAL,                      -- 分摊到该日的成本（cherry 实报 / openai·xai 服务端日账）
                currency     TEXT,
                cost_estimated INTEGER NOT NULL DEFAULT 0,
                captured_at  TEXT NOT NULL,
                PRIMARY KEY (source, date, model)
            );
            CREATE INDEX IF NOT EXISTS idx_usage_daily_lookup
                ON usage_daily(kind, date DESC);
            ",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Upsert today's snapshot for `provider_id`. Called from
    /// `Provider::fetch_usage` implementations that don't return a native
    /// heatmap.
    pub fn record_daily(&self, provider_id: &str, value: f64, unit: UsageUnit) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let today = chrono::Local::now().date_naive();
        let now_iso = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO daily_snapshots (provider_id, date, value, unit, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                provider_id,
                today.to_string(),
                value,
                unit_db_code(unit),
                now_iso,
            ],
        )?;
        Ok(())
    }

    /// Add `delta` onto today's snapshot for `provider_id`, creating the row
    /// if missing. Unlike `record_daily` this ACCUMULATES instead of
    /// replacing, so repeated polls in one day sum up (used by the DeepSeek
    /// balance-delta accounting).
    pub fn accumulate_daily(&self, provider_id: &str, delta: f64, unit: UsageUnit) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let today = chrono::Local::now().date_naive();
        let now_iso = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO daily_snapshots (provider_id, date, value, unit, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider_id, date) DO UPDATE SET
                 value = value + excluded.value,
                 captured_at = excluded.captured_at",
            params![
                provider_id,
                today.to_string(),
                delta,
                unit_db_code(unit),
                now_iso,
            ],
        )?;
        Ok(())
    }

    /// Write a snapshot for a specific `date`, not necessarily today. This lets
    /// providers (MiniMax) attribute a window's usage to the calendar day the
    /// window *ends* on, rather than the day the poll happened. If the day
    /// already holds a row, the larger value wins (peak usage across several
    /// windows that closed the same day), so a day is never under-reported.
    pub fn record_daily_on(
        &self,
        provider_id: &str,
        date: NaiveDate,
        value: f64,
        unit: UsageUnit,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now_iso = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO daily_snapshots (provider_id, date, value, unit, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider_id, date) DO UPDATE SET
                 value = MAX(value, excluded.value),
                 unit = excluded.unit,
                 captured_at = excluded.captured_at",
            params![
                provider_id,
                date.to_string(),
                value,
                unit_db_code(unit),
                now_iso,
            ],
        )?;
        Ok(())
    }

    /// Read a provider-scoped key/value pair (e.g. balance baselines).
    pub fn kv_get(&self, provider_id: &str, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let value = conn
            .query_row(
                "SELECT value FROM provider_kv WHERE provider_id = ?1 AND key = ?2",
                params![provider_id, key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Write a provider-scoped key/value pair.
    pub fn kv_set(&self, provider_id: &str, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_kv (provider_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(provider_id, key) DO UPDATE SET value = excluded.value",
            params![provider_id, key, value],
        )?;
        Ok(())
    }

    /// Returns up to `days` most recent daily cells for `provider_id`,
    /// oldest first. Days without a snapshot are not included - the frontend
    /// fills gaps with grey cells.
    pub fn load_heatmap(&self, provider_id: &str, days: u32) -> Result<Vec<HeatmapCell>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date, value, unit FROM daily_snapshots
             WHERE provider_id = ?1
             ORDER BY date DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![provider_id, days as i64], |row| {
            let date_str: String = row.get(0)?;
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                ))?;
            let value: f64 = row.get(1)?;
            let unit_str: String = row.get(2)?;
            let unit = unit_from_db_str(&unit_str);
            Ok(HeatmapCell { date, value, unit })
        })?;
        let mut cells: Vec<HeatmapCell> = rows.collect::<rusqlite::Result<_>>()?;
        cells.reverse(); // oldest first for left-to-right rendering
        Ok(cells)
    }

    // ---- 统一日账本（usage_daily）-----------------------------------------
    //
    // 三个历史视图（日历/趋势/模型）共用的唯一日用量账本。与 daily_snapshots
    // 的区别：写入方只有**全量回放型**来源（本机工具重扫、volcengine/openai/xai
    // 服务端日账回放），逐行 REPLACE 幂等；差分类 provider（窗口/余额差分）不
    // 写此表，避免 MAX/SUM 语义在同表冲突。

    /// 全量回放一批账本行：先删除批内涉及来源的旧行再插入。幂等，且天然清掉
    /// 滚动窗口之外的过期日（工具扫描只保留 90 天）。
    pub fn replace_usage_daily(&self, rows: &[UsageDailyRow]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let sources: Vec<String> = {
            let mut s: Vec<String> = rows.iter().map(|r| r.source.clone()).collect();
            s.sort();
            s.dedup();
            s
        };
        let tx = conn.transaction()?;
        {
            let mut del = tx.prepare("DELETE FROM usage_daily WHERE source = ?1")?;
            for src in &sources {
                del.execute(params![src])?;
            }
        }
        {
            let now = chrono::Utc::now().to_rfc3339();
            let mut ins = tx.prepare(
                "INSERT OR REPLACE INTO usage_daily
                 (source, kind, date, model, input, cache_read, output, total, unit, cost, currency, cost_estimated, captured_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for r in rows {
                ins.execute(params![
                    r.source,
                    r.kind,
                    r.date,
                    r.model,
                    r.input,
                    r.cache_read,
                    r.output,
                    r.total,
                    unit_db_code(r.unit),
                    r.cost,
                    r.currency,
                    r.cost_estimated as i64,
                    now,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 读取最近 `days` 天的账本行（升序）。`kind` 过滤：`None` = 全部。
    pub fn load_usage_daily(
        &self,
        kind: Option<&str>,
        days: u32,
    ) -> Result<Vec<UsageDailyRow>> {
        let cutoff = (chrono::Local::now().date_naive()
            - chrono::Duration::days(days as i64))
        .format("%Y-%m-%d")
        .to_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT source, kind, date, model, input, cache_read, output, total, unit, cost, currency, cost_estimated
             FROM usage_daily
             WHERE date >= ?1 AND (?2 IS NULL OR kind = ?2)
             ORDER BY date, source, model",
        )?;
        let rows = stmt.query_map(params![cutoff, kind], |r| {
            let unit_str: String = r.get(8)?;
            Ok(UsageDailyRow {
                source: r.get(0)?,
                kind: r.get(1)?,
                date: r.get(2)?,
                model: r.get(3)?,
                input: r.get(4)?,
                cache_read: r.get(5)?,
                output: r.get(6)?,
                total: r.get(7)?,
                unit: unit_from_db_str(&unit_str),
                cost: r.get(9)?,
                currency: r.get(10)?,
                cost_estimated: r.get::<_, i64>(11)? != 0,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    /// 路由记账：把一次经 TokenRouter 转发的用量增量累计进统一账本
    /// （`source = router:<instance_id>`，`kind` 由调用方给，`model` 为**实际
    /// 承载模型**——候选指定了就用它，未指定（透传）就用工具请求里的原始模型名）。
    /// `router:*` 键没有其他写入方，与 `replace_usage_daily` 的全量回放语义
    /// 天然隔离——ON CONFLICT 累加不会和 REPLACE 互相覆盖。
    ///
    /// `kind` 独立传参而非写死 'provider'：路由自记账不是服务端日账，混在
    /// `kind='provider'` 里会让前端的 provider 日账分支把它当官方账本吸收。
    pub fn accumulate_usage_daily(
        &self,
        kind: &str,
        source: &str,
        date: NaiveDate,
        model: &str,
        input: f64,
        cache_read: f64,
        output: f64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO usage_daily
             (source, kind, date, model, input, cache_read, output, total, unit, cost, currency, cost_estimated, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'tokens', NULL, NULL, 0, ?9)
             ON CONFLICT(source, date, model) DO UPDATE SET
                input = input + excluded.input,
                cache_read = cache_read + excluded.cache_read,
                output = output + excluded.output,
                total = total + excluded.total,
                captured_at = excluded.captured_at",
            params![
                source,
                kind,
                date.to_string(),
                model,
                input,
                cache_read,
                output,
                input + cache_read + output,
                now,
            ],
        )?;
        Ok(())
    }

    /// 今日某来源的账本总量。TokenRouter 手填日限的 used 口径：used = 路由器
    /// 自记账的当日经路由消耗。
    ///
    /// **不按 `model` 过滤**：路由自记账按实际承载模型分行（`ON CONFLICT
    /// (source,date,model)`，同日多模型各占一行），这里对当天全部模型行求和。
    /// 早期版本只认 `model=''` 总量行，路由开始记模型后会直接归零——手填日限
    /// 随之永不触发，路由会突破用户日上限。`router:*` 的 source 键只有路由自己
    /// 写，无需再按 kind 收窄。
    pub fn sum_usage_daily_today(&self, source: &str) -> Result<f64> {
        let conn = self.conn.lock().unwrap();
        let today = chrono::Local::now().date_naive().to_string();
        let total = conn.query_row(
            "SELECT COALESCE(SUM(total), 0.0) FROM usage_daily
             WHERE source = ?1 AND date = ?2",
            params![source, today],
            |row| row.get::<_, f64>(0),
        )?;
        Ok(total)
    }

    /// 一次性清理**路由改记模型之前**留下的旧行（`kind='provider'` 且
    /// `source LIKE 'router:%'`）。
    ///
    /// 路由现在写 `kind='router'` + 实际承载模型，旧行只剩「走了路由、但不知道
    /// 用了哪个模型」这一条信息——正是 [`crate::local::ROUTED_MODEL`] 哨兵已经
    /// 表达过的状态。留着它们没有额外价值，却会让账本里同时存在两种 kind 的
    /// 路由行，下游按 kind 统计时口径不一。
    ///
    /// 删除是无损的：这些行从未被任何视图消费（前端一律过滤 `kind='tool'`，
    /// provider 日账分支也按 source 前缀匹配不到 `router:`），工具侧的用量早已
    /// 由各自扫描器独立记账。**只删旧 kind，不碰 `router:*` 的新行。**
    pub fn purge_legacy_router_ledger(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM usage_daily WHERE kind = 'provider' AND source LIKE 'router:%'",
            [],
        )?;
        if n > 0 {
            tracing::info!(target: "tum.storage", "purged {n} legacy router ledger rows (kind='provider')");
        }
        Ok(n)
    }

    /// 旧数据一次性迁移：把 `daily_snapshots` 里的 `minimax-code` 键（tokens）
    /// 与 `hermes_daily` 台账导入统一账本。目标来源已有数据时跳过——迁移只发生
    /// 一次，之后由正常回放路径维护。
    pub fn migrate_legacy_usage_ledger(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let has = |src: &str| -> rusqlite::Result<i64> {
            conn.query_row(
                "SELECT COUNT(*) FROM usage_daily WHERE source = ?1",
                [src],
                |r| r.get::<_, i64>(0),
            )
        };

        // minimax-code：daily_snapshots 里的 tokens 日值 → 账本总量行。
        if has("minimax-code")? == 0 {
            conn.execute(
                "INSERT INTO usage_daily
                 (source, kind, date, model, input, cache_read, output, total, unit, cost, currency, cost_estimated, captured_at)
                 SELECT 'minimax-code', 'tool', date, '', 0, 0, 0, value, unit, NULL, NULL, 0, ?1
                 FROM daily_snapshots WHERE provider_id = 'minimax-code'",
                params![now],
            )?;
        }

        // hermes：逐日分模型台账 → 账本分项行。
        if has("hermes")? == 0 {
            conn.execute(
                "INSERT INTO usage_daily
                 (source, kind, date, model, input, cache_read, output, total, unit, cost, currency, cost_estimated, captured_at)
                 SELECT 'hermes', 'tool', date, model, input, cache_read, output,
                        input + cache_read + output, 'tokens', cost, 'USD', cost_estimated, ?1
                 FROM hermes_daily",
                params![now],
            )?;
        }
        Ok(())
    }

    /// Load the persisted local-scan cache. Returns `(fingerprint, payload)` when
    /// present. `fingerprint` is the metadata digest of all local tool sources at
    /// the time the payload was written; if it still matches a freshly-computed
    /// digest, the payload is replayed without re-parsing log files.
    pub fn load_local_scan_cache(&self) -> Option<(String, String)> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT fingerprint, payload FROM local_scan_cache WHERE key = 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .ok()
    }

    /// Overwrite the single persisted local-scan cache row.
    pub fn save_local_scan_cache(&self, fingerprint: &str, payload: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO local_scan_cache (key, fingerprint, payload, updated_at)
             VALUES (1, ?1, ?2, ?3)",
            params![fingerprint, payload, chrono::Utc::now().to_rfc3339()],
        )
        .with_context(|| "saving local scan cache")?;
        Ok(())
    }

    /// Upsert one device's hub report (keyed by `device_id`).
    pub fn upsert_hub_device(&self, device: &crate::hub::HubDevice) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO hub_device (device_id, payload, updated_at)
             VALUES (?1, ?2, ?3)",
            params![
                device.device_id,
                serde_json::to_string(device).with_context(|| "serializing hub device")?,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .with_context(|| "upserting hub device")?;
        Ok(())
    }

    /// All known devices, newest-updated first. Rows that fail to deserialize
    /// (schema drift) are skipped rather than breaking the whole list.
    pub fn list_hub_devices(&self) -> Result<Vec<crate::hub::HubDevice>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT payload, updated_at FROM hub_device")
            .with_context(|| "preparing hub devices")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .with_context(|| "querying hub devices")?;
        let mut out = Vec::new();
        for row in rows.flatten() {
            if let Ok(device) = serde_json::from_str::<crate::hub::HubDevice>(&row.0) {
                out.push(device);
            }
        }
        out.sort_by(|a, b| b.reported_at.cmp(&a.reported_at));
        Ok(out)
    }

    /// 移除一台设备的上报记录（用户在设备页主动删除）。注意：若该设备仍在
    /// 同步（lan mesh 30s 上报 / agent 周期上报），它会在下一个周期重新出现——
    /// 移除的意义是清掉改名残留、已退役机器等"幽灵设备"。
    pub fn remove_hub_device(&self, device_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM hub_device WHERE device_id = ?1",
            [device_id],
        )
        .with_context(|| "removing hub device")?;
        Ok(())
    }

    /// 记录一个 jsonl 文件的续读指针 `offset`（停在完整行末尾），供字节级增量扫描
    /// 判定"从哪读新增行"以及是否轮转。`len`/`mtime` 为记录时刻的文件元数据。
    pub fn upsert_tool_file_offset(
        &self,
        tool: &str,
        path: &str,
        offset: u64,
        len: u64,
        mtime: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO tool_file_offset (tool, path, offset, len, mtime)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![tool, path, offset as i64, len as i64, mtime],
        )
        .with_context(|| "upserting tool file offset")?;
        Ok(())
    }

    /// 读取某 jsonl 文件的续读指针，返回 `(offset, len, mtime)`；无记录返回 `None`。
    pub fn get_tool_file_offset(
        &self,
        tool: &str,
        path: &str,
    ) -> Result<Option<(u64, u64, i64)>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT offset, len, mtime FROM tool_file_offset WHERE tool = ?1 AND path = ?2",
                params![tool, path],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?;
        Ok(row.map(|(o, l, m)| (o as u64, l as u64, m)))
    }

    /// 删除某工具的全部续读指针（轮转/全量重扫后重建）。
    pub fn remove_tool_file_offsets(&self, tool: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tool_file_offset WHERE tool = ?1",
            params![tool],
        )
        .with_context(|| "removing tool file offsets")?;
        Ok(())
    }

    /// 读取所有 Hermes 会话的累计基线，返回 `(session_id, input, cache_read, output, cost_usd)`。
    pub fn get_hermes_baselines(&self) -> Result<Vec<(String, f64, f64, f64, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT session_id, input, cache_read, output, cost_usd FROM hermes_baseline")
            .with_context(|| "preparing hermes baselines")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, f64>(4)?,
                ))
            })
            .with_context(|| "querying hermes baselines")?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 覆写 Hermes 的累计基线（每个会话一条）。删除旧表后重建，天然清理已消失的会话。
    pub fn save_hermes_baselines(&self, rows: &[(String, f64, f64, f64, f64)]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn
            .unchecked_transaction()
            .with_context(|| "starting hermes baseline tx")?;
        tx.execute("DELETE FROM hermes_baseline", [])
            .with_context(|| "clearing hermes baselines")?;
        for (sid, input, cache_read, output, cost) in rows {
            tx.execute(
                "INSERT INTO hermes_baseline (session_id, input, cache_read, output, cost_usd)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sid, input, cache_read, output, cost],
            )
            .with_context(|| "inserting hermes baseline")?;
        }
        tx.commit().with_context(|| "committing hermes baseline tx")?;
        Ok(())
    }

    /// 读取整个 Hermes 逐日台账，返回 `(date, model, input, cache_read, output, cost, cost_estimated)`。
    pub fn get_hermes_daily(&self) -> Result<Vec<(String, String, f64, f64, f64, f64, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT date, model, input, cache_read, output, cost, cost_estimated \
                 FROM hermes_daily",
            )
            .with_context(|| "preparing hermes daily")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, f64>(4)?,
                    r.get::<_, f64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .with_context(|| "querying hermes daily")?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 把 Hermes 一天的用量累加进台账（不存在则新建）。用于头部归属（种子）与当日增量记账。
    pub fn add_hermes_daily(
        &self,
        date: &str,
        model: &str,
        input: f64,
        cache_read: f64,
        output: f64,
        cost: f64,
        estimated: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO hermes_daily (date, model, input, cache_read, output, cost, cost_estimated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(date, model) DO UPDATE SET
                 input = input + excluded.input,
                 cache_read = cache_read + excluded.cache_read,
                 output = output + excluded.output,
                 cost = cost + excluded.cost,
                 cost_estimated = MAX(cost_estimated, excluded.cost_estimated)",
            params![
                date,
                model,
                input,
                cache_read,
                output,
                cost,
                if estimated { 1 } else { 0 }
            ],
        )
        .with_context(|| "adding hermes daily")?;
        Ok(())
    }

    /// 删除 `date` 早于 `cutoff`（含）的台账行，避免无限增长。
    pub fn prune_hermes_daily(&self, cutoff: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM hermes_daily WHERE date < ?1",
            params![cutoff],
        )
        .with_context(|| "pruning hermes daily")?;
        Ok(())
    }

    /// 覆写一批汇率（整批替换，`codes` 为最新拉取到的币种全集），
    /// 未出现在批次里的旧币种行被删除，避免残留陈旧值。
    pub fn save_exchange_rates(&self, rates: &[(String, f64)]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now_iso = chrono::Utc::now().to_rfc3339();
        let tx = conn
            .unchecked_transaction()
            .with_context(|| "starting exchange rate tx")?;
        tx.execute("DELETE FROM exchange_rates", [])
            .with_context(|| "clearing exchange rates")?;
        for (code, rate) in rates {
            tx.execute(
                "INSERT INTO exchange_rates (code, rate, updated_at) VALUES (?1, ?2, ?3)",
                params![code, rate, now_iso],
            )
            .with_context(|| "inserting exchange rate")?;
        }
        tx.commit().with_context(|| "committing exchange rate tx")?;
        Ok(())
    }

    /// 读取全部已缓存汇率，返回 `(code, rate, updated_at)`，按币种代码排序。
    pub fn list_exchange_rates(&self) -> Result<Vec<(String, f64, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT code, rate, updated_at FROM exchange_rates ORDER BY code")
            .with_context(|| "preparing exchange rates")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .with_context(|| "querying exchange rates")?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 读取单个币种的汇率，返回 `(rate, updated_at)`；无缓存返回 `None`。
    pub fn get_exchange_rate(&self, code: &str) -> Result<Option<(f64, String)>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT rate, updated_at FROM exchange_rates WHERE code = ?1",
                params![code],
                |r| Ok((r.get::<_, f64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(row)
    }
}

fn unit_db_code(unit: UsageUnit) -> &'static str {
    match unit {
        UsageUnit::Tokens => "tokens",
        UsageUnit::Afp => "afp",
        UsageUnit::Cny => "cny",
        UsageUnit::Credits => "credits",
        UsageUnit::Usd => "usd",
        UsageUnit::Percent => "percent",
    }
}

fn unit_from_db_str(s: &str) -> UsageUnit {
    match s {
        "tokens" => UsageUnit::Tokens,
        "afp" | "AFP" => UsageUnit::Afp,
        "cny" | "¥" => UsageUnit::Cny,
        "credits" => UsageUnit::Credits,
        "usd" | "$" => UsageUnit::Usd,
        "percent" | "%" => UsageUnit::Percent,
        _ => UsageUnit::Tokens,
    }
}

#[cfg(test)]
mod storage_unit_tests {
    use super::*;
    use crate::providers::UsageUnit;

    fn temp_storage() -> Storage {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("pulse_heatmap_unit_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Storage::open(&path).unwrap()
    }

    /// 只清旧 kind 的路由行，且**幂等**；同批里的服务端日账与新路由行必须留下。
    #[test]
    fn purge_legacy_router_ledger_only_touches_old_kind() {
        let s = temp_storage();
        let today = chrono::Local::now().date_naive();
        // 旧路由行：kind='provider' + router: 来源 —— 应被清掉
        s.accumulate_usage_daily("provider", "router:acct-1", today, "", 100.0, 0.0, 0.0)
            .unwrap();
        s.accumulate_usage_daily("provider", "router:acct-2", today, "", 200.0, 0.0, 0.0)
            .unwrap();
        // 新路由行：kind='router' —— 必须保留
        s.accumulate_usage_daily(
            crate::router::ROUTER_LEDGER_KIND,
            "router:acct-1",
            today,
            "ark-seed-1.6",
            50.0,
            0.0,
            0.0,
        )
        .unwrap();
        // 真正的服务端日账 —— 必须保留
        s.accumulate_usage_daily("provider", "volcengine-1-0", today, "", 999.0, 0.0, 0.0)
            .unwrap();

        let purged = s.purge_legacy_router_ledger().unwrap();
        assert_eq!(purged, 2, "两条旧路由行应被清除");

        let rows = s.load_usage_daily(None, 7).unwrap();
        assert!(
            !rows.iter().any(|r| r.source == "router:acct-2"),
            "旧路由行应消失"
        );
        assert!(
            rows.iter().any(|r| r.source == "router:acct-1" && r.model == "ark-seed-1.6"),
            "新路由行须保留: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.source == "volcengine-1-0"),
            "服务端日账须保留"
        );

        // 幂等：再跑一次不删任何东西，也不报错
        assert_eq!(s.purge_legacy_router_ledger().unwrap(), 0);
    }

    /// used 口径对当日全部模型行求和——只认 model='' 会让路由日限静默失效。
    #[test]
    fn sum_usage_daily_today_sums_across_models() {
        let s = temp_storage();
        let today = chrono::Local::now().date_naive();
        let src = "router:acct-1";
        s.accumulate_usage_daily(crate::router::ROUTER_LEDGER_KIND, src, today, "model-a", 100.0, 0.0, 0.0)
            .unwrap();
        s.accumulate_usage_daily(crate::router::ROUTER_LEDGER_KIND, src, today, "model-b", 250.0, 0.0, 0.0)
            .unwrap();
        // 同模型再累加（ON CONFLICT 累加语义）
        s.accumulate_usage_daily(crate::router::ROUTER_LEDGER_KIND, src, today, "model-a", 50.0, 0.0, 0.0)
            .unwrap();
        assert!((s.sum_usage_daily_today(src).unwrap() - 400.0).abs() < 1e-9);
    }

    #[test]
    fn tool_file_offset_round_trip_upsert_get_remove() {
        let s = temp_storage();
        assert_eq!(s.get_tool_file_offset("claude-code", "/x/s.jsonl").unwrap(), None);
        s.upsert_tool_file_offset("claude-code", "/x/s.jsonl", 5, 5, 11).unwrap();
        assert_eq!(
            s.get_tool_file_offset("claude-code", "/x/s.jsonl").unwrap(),
            Some((5, 5, 11))
        );
        // 覆盖已存在的行（追加后指针前移）。
        s.upsert_tool_file_offset("claude-code", "/x/s.jsonl", 20, 20, 12).unwrap();
        assert_eq!(
            s.get_tool_file_offset("claude-code", "/x/s.jsonl").unwrap(),
            Some((20, 20, 12))
        );
        // 不同工具互不影响，remove 只删指定工具。
        s.upsert_tool_file_offset("codex", "/y/c.jsonl", 1, 1, 0).unwrap();
        s.remove_tool_file_offsets("claude-code").unwrap();
        assert_eq!(s.get_tool_file_offset("claude-code", "/x/s.jsonl").unwrap(), None);
        assert_eq!(s.get_tool_file_offset("codex", "/y/c.jsonl").unwrap(), Some((1, 1, 0)));
    }

    #[test]
    fn local_scan_cache_round_trip_overwrites() {
        let s = temp_storage();
        assert_eq!(s.load_local_scan_cache(), None);
        s.save_local_scan_cache("fp1", "{\"a\":1}").unwrap();
        assert_eq!(s.load_local_scan_cache(), Some(("fp1".to_string(), "{\"a\":1}".to_string())));
        // Overwrite the single row (key=1).
        s.save_local_scan_cache("fp2", "{\"b\":2}").unwrap();
        assert_eq!(s.load_local_scan_cache(), Some(("fp2".to_string(), "{\"b\":2}".to_string())));
    }

    #[test]
    fn exchange_rates_round_trip_and_batch_replace() {
        let s = temp_storage();
        assert!(s.list_exchange_rates().unwrap().is_empty());
        assert_eq!(s.get_exchange_rate("CNY").unwrap(), None);

        s.save_exchange_rates(&[
            ("CNY".to_string(), 7.13),
            ("HKD".to_string(), 7.79),
        ])
        .unwrap();
        let all = s.list_exchange_rates().unwrap();
        assert_eq!(all.len(), 2);
        // 按 code 排序：CNY 在 HKD 前。
        assert_eq!(all[0].0, "CNY");
        assert!((all[0].1 - 7.13).abs() < 1e-9);
        assert!(!all[0].2.is_empty(), "updated_at 必须写入");
        // 整批替换：新批次没有 HKD，旧行应被清掉。
        s.save_exchange_rates(&[("CNY".to_string(), 7.20)]).unwrap();
        let all = s.list_exchange_rates().unwrap();
        assert_eq!(all.len(), 1);
        assert!((all[0].1 - 7.20).abs() < 1e-9);
        assert_eq!(s.get_exchange_rate("HKD").unwrap(), None);
        let (rate, updated) = s.get_exchange_rate("CNY").unwrap().unwrap();
        assert!((rate - 7.20).abs() < 1e-9);
        assert!(!updated.is_empty());
    }

    #[test]
    fn round_trips_all_canonical_units() {
        let cases = [
            ("p_tokens", UsageUnit::Tokens),
            ("p_afp", UsageUnit::Afp),
            ("p_cny", UsageUnit::Cny),
            ("p_credits", UsageUnit::Credits),
            ("p_upd", UsageUnit::Usd),
            ("p_percent", UsageUnit::Percent),
        ];
        for (provider_id, unit) in cases {
            let storage = temp_storage();
            storage.record_daily(provider_id, 42.0, unit).unwrap();
            let cells = storage.load_heatmap(provider_id, 31).unwrap();
            assert_eq!(cells.len(), 1, "expected exactly one cell for {provider_id}");
            assert_eq!(
                cells[0].unit, unit,
                "unit round-trip mismatch for {provider_id}"
            );
        }
    }

    #[test]
    fn legacy_afp_label_still_parses() {
        assert_eq!(unit_from_db_str("AFP"), UsageUnit::Afp);
    }

    #[test]
    fn legacy_cny_symbol_still_parses() {
        assert_eq!(unit_from_db_str("¥"), UsageUnit::Cny);
    }

    #[test]
    fn legacy_percent_symbol_still_parses() {
        assert_eq!(unit_from_db_str("%"), UsageUnit::Percent);
    }

    #[test]
    fn canonical_codes_parse() {
        assert_eq!(unit_from_db_str("tokens"), UsageUnit::Tokens);
        assert_eq!(unit_from_db_str("afp"), UsageUnit::Afp);
        assert_eq!(unit_from_db_str("cny"), UsageUnit::Cny);
        assert_eq!(unit_from_db_str("credits"), UsageUnit::Credits);
        assert_eq!(unit_from_db_str("percent"), UsageUnit::Percent);
    }

    #[test]
    fn unknown_unit_falls_back_to_tokens() {
        assert_eq!(unit_from_db_str("nonsense"), UsageUnit::Tokens);
    }
}
