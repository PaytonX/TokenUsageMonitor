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
