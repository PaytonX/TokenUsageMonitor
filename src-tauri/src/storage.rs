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
                unit          TEXT    NOT NULL,             -- tokens|afp|cny|credits
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
                unit.label(),
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
                unit.label(),
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
            let unit = match unit_str.as_str() {
                "tokens" => UsageUnit::Tokens,
                "afp" => UsageUnit::Afp,
                "cny" => UsageUnit::Cny,
                "credits" => UsageUnit::Credits,
                _ => UsageUnit::Tokens,
            };
            Ok(HeatmapCell { date, value, unit })
        })?;
        let mut cells: Vec<HeatmapCell> = rows.collect::<rusqlite::Result<_>>()?;
        cells.reverse(); // oldest first for left-to-right rendering
        Ok(cells)
    }
}
