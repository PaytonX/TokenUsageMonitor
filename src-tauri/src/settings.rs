//! User settings: provider enable list, polling intervals, dashboard position.
//! Non-sensitive data lives in `config.toml` under the OS app data dir.
//! Sensitive credentials (API keys, secret keys) are stored in the OS
//! credential manager via the `keyring` crate (Windows DPAPI).
//!
//! Until step 7 is exercised in the UI, `Settings::load()` returns a default
//! config with all providers disabled, which lets the dashboard start clean.

use crate::providers::{preset_display_name, AccountMeta, Credentials};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Legacy field (pre multi-account). Kept for backward compatibility: on
    /// startup, if `accounts` is empty this list seeds the initial accounts.
    #[serde(default)]
    pub enabled_providers: Vec<String>,
    /// User-configured accounts (the source of truth since multi-account).
    #[serde(default)]
    pub accounts: Vec<AccountMeta>,
    /// Polling interval seconds. 0 = use per-provider default.
    pub poll_interval_seconds: u32,
    /// Persisted dashboard position (last known). Restored on startup.
    pub dashboard_x: Option<i32>,
    pub dashboard_y: Option<i32>,
    /// Compact-mode flag.
    #[serde(default)]
    pub compact_mode: bool,
    /// Whether to autostart on system boot (informational; the user must add
    /// a shortcut to shell:startup themselves for now).
    #[serde(default)]
    pub autostart_hint_shown: bool,
    /// Close button hides to tray instead of quitting. Default true.
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// Whether to fire OS notifications when usage crosses a threshold.
    /// Default true.
    #[serde(default = "default_notify_enabled")]
    pub notify_enabled: bool,
    /// Notification threshold (percent used) for the warning level.
    #[serde(default = "default_notify_warn_percent")]
    pub notify_warn_percent: u8,
    /// Notification threshold (percent used) for the critical level.
    #[serde(default = "default_notify_crit_percent")]
    pub notify_crit_percent: u8,
    /// Which usage window the ring gauges show: "auto" (per provider's most
    /// critical window), or an explicit "five_hour" | "daily" | "weekly" |
    /// "monthly".
    #[serde(default = "default_ring_window")]
    pub ring_window: String,
    /// Snap the dashboard to screen edges when it is dragged near one. Default
    /// on.
    #[serde(default = "default_edge_snap")]
    pub edge_snap: bool,
    /// Ring gauge display mode. `false` = show usage *used*; `true` = show the
    /// *remaining* position (countdown). Display-only, read by the frontend.
    #[serde(default = "default_countdown_mode")]
    pub countdown_mode: bool,
}

fn default_countdown_mode() -> bool {
    false
}

fn default_close_to_tray() -> bool {
    true
}

fn default_notify_enabled() -> bool {
    true
}

fn default_notify_warn_percent() -> u8 {
    80
}

fn default_notify_crit_percent() -> u8 {
    95
}

fn default_ring_window() -> String {
    "auto".to_string()
}

fn default_edge_snap() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled_providers: Vec::new(),
            accounts: Vec::new(),
            poll_interval_seconds: 0,
            dashboard_x: None,
            dashboard_y: None,
            compact_mode: false,
            autostart_hint_shown: false,
            close_to_tray: default_close_to_tray(),
            notify_enabled: default_notify_enabled(),
            notify_warn_percent: default_notify_warn_percent(),
            notify_crit_percent: default_notify_crit_percent(),
            ring_window: default_ring_window(),
            edge_snap: default_edge_snap(),
            countdown_mode: default_countdown_mode(),
        }
    }
}

/// Backward compatibility: turn the pre-multi-account `enabled_providers` list
/// into initial `AccountMeta` entries so existing users' configs still load.
/// No-op when the config already has accounts.
///
/// Legacy credentials lived under the provider kind (e.g. "deepseek"); the
/// migrated `instance_id` is the same value, so the existing keyring entries
/// are picked up as-is - no credential migration needed.
fn migrate_legacy(mut settings: Settings, config_path: &PathBuf) -> Result<Settings> {
    if !settings.accounts.is_empty() || settings.enabled_providers.is_empty() {
        return Ok(settings);
    }
    for kind in std::mem::take(&mut settings.enabled_providers) {
        settings.accounts.push(AccountMeta {
            instance_id: kind.clone(),
            provider_kind: kind.clone(),
            label: preset_display_name(&kind),
            accent_color: crate::providers::PRESETS
                .iter()
                .find(|p| p.kind == kind)
                .map(|p| p.default_accent)
                .unwrap_or("#29b6f6")
                .to_string(),
            enabled: true,
            note: None,
        });
    }
    let new_settings = settings.clone();
    let tmp = config_path.with_extension("toml.tmp");
    let body = toml::to_string_pretty(&new_settings).with_context(|| "serializing migrated config")?;
    std::fs::write(&tmp, body).with_context(|| "writing migrated tmp config")?;
    std::fs::rename(&tmp, config_path).with_context(|| "renaming migrated config")?;
    Ok(new_settings)
}

#[derive(Clone)]
pub struct SettingsStore {
    /// In-memory cache (RwLock so concurrent IPC reads don't block writes).
    cache: Arc<RwLock<Settings>>,
    /// Where the `config.toml` file lives on disk.
    config_path: PathBuf,
    /// Credential manager namespace prefix. Each provider's credentials are
    /// stored at `keyring::Entry::new("TokenUsageMonitor", provider_id)`.
    service_name: String,
}

impl SettingsStore {
    /// Construct a settings store under the OS app data dir.
    pub fn new(app_data_dir: PathBuf) -> Result<Self> {
        let config_path = app_data_dir.join("config.toml");
        let cache = if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)
                .with_context(|| "reading config.toml")?;
            let parsed: Settings = toml::from_str(&raw)
                .with_context(|| "parsing config.toml")?;
            // Migrate legacy `enabled_providers` configs to the multi-account
            // model, writing the upgraded config back when it changed.
            let migrated = migrate_legacy(parsed, &config_path)?;
            Arc::new(RwLock::new(migrated))
        } else {
            Arc::new(RwLock::new(Settings::default()))
        };
        Ok(Self {
            cache,
            config_path,
            service_name: "TokenUsageMonitor".to_string(),
        })
    }

    pub async fn get(&self) -> Settings {
        self.cache.read().await.clone()
    }

    /// Synchronous read of the cached settings. Safe for one-shot startup use
    /// before the async runtime drives concurrent accesses (a write hold here
    /// would only come from an in-flight `save`, which hasn't happened yet).
    pub fn read_blocking(&self) -> Settings {
        self.cache.try_read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Sync, non-async read of `close_to_tray`. Used by the synchronous
    /// `on_window_event` close interceptor in lib.rs. Tokio RwLock write holds
    /// are brief (settings saves are rare), so `try_read` is reliable here.
    pub fn close_to_tray_now(&self) -> bool {
        self.cache.try_read().map(|s| s.close_to_tray).unwrap_or(true)
    }

    /// Sync, non-async read of `edge_snap`. Same rationale as
    /// `close_to_tray_now`.
    pub fn edge_snap_now(&self) -> bool {
        self.cache.try_read().map(|s| s.edge_snap).unwrap_or(true)
    }

    pub async fn save(&self, new_settings: Settings) -> Result<()> {
        // Write atomically: tmp file + rename.
        let tmp = self.config_path.with_extension("toml.tmp");
        let body = toml::to_string_pretty(&new_settings).with_context(|| "serializing config")?;
        std::fs::write(&tmp, body).with_context(|| "writing tmp config")?;
        std::fs::rename(&tmp, &self.config_path).with_context(|| "renaming tmp config")?;
        *self.cache.write().await = new_settings;
        Ok(())
    }

    /// Load credentials for a provider from the OS credential store.
    /// Returns `None` if not set or if the OS store is unavailable.
    pub fn load_credentials(&self, provider_id: &str) -> Option<Credentials> {
        let entry = keyring::Entry::new(&self.service_name, provider_id).ok()?;
        let raw = entry.get_password().ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn save_credentials(&self, provider_id: &str, creds: &Credentials) -> Result<()> {
        let entry = keyring::Entry::new(&self.service_name, provider_id)
            .context("creating keyring entry")?;
        let raw = serde_json::to_string(creds).context("serializing credentials")?;
        entry.set_password(&raw).context("writing keyring entry")?;
        Ok(())
    }

    pub fn delete_credentials(&self, provider_id: &str) -> Result<()> {
        if let Ok(entry) = keyring::Entry::new(&self.service_name, provider_id) {
            let _ = entry.delete_credential();
        }
        Ok(())
    }
}

#[cfg(test)]
mod settings_defaults_tests {
    use super::Settings;

    #[test]
    fn old_toml_without_new_fields_gets_defaults() {
        let raw = "\
enabled_providers = [\"minimax\"]
poll_interval_seconds = 30
dashboard_x = 100
dashboard_y = 200
compact_mode = true
autostart_hint_shown = false
";
        let parsed: Settings = toml::from_str(raw).expect("parse legacy config");
        assert!(parsed.close_to_tray);
        assert_eq!(parsed.notify_warn_percent, 80);
        assert_eq!(parsed.notify_crit_percent, 95);
    }

    #[test]
    fn new_fields_round_trip() {
        let raw = "\
enabled_providers = []
poll_interval_seconds = 60
close_to_tray = false
notify_warn_percent = 70
notify_crit_percent = 90
";
        let parsed: Settings = toml::from_str(raw).expect("parse new config");
        assert!(!parsed.close_to_tray);
        assert_eq!(parsed.notify_warn_percent, 70);
        assert_eq!(parsed.notify_crit_percent, 90);

        let dumped = toml::to_string(&parsed).expect("serialize parsed");
        let reparsed: Settings = toml::from_str(&dumped).expect("reparse dumped config");
        assert!(!reparsed.close_to_tray);
        assert_eq!(reparsed.notify_warn_percent, 70);
        assert_eq!(reparsed.notify_crit_percent, 90);

        let default_dumped = toml::to_string(&Settings::default()).expect("serialize defaults");
        assert!(default_dumped.contains("close_to_tray = true"));
        assert!(default_dumped.contains("notify_warn_percent = 80"));
        assert!(default_dumped.contains("notify_crit_percent = 95"));
    }
}
