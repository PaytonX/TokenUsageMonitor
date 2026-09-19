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
