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
use std::collections::{BTreeMap, HashMap};
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
    /// 额外的本机工具数据根目录。工具默认把点目录（`.claude` / `.zcode` /
    /// `.minimax` …）写在 `~` 下；用户常整体搬到别处（如 `D:\Lab\.agentdata`）。
    /// 列一个根目录即可覆盖其下所有点目录，扫描器按目录新鲜度择优。
    /// 空 = 只用默认位置。
    #[serde(default)]
    pub tool_data_roots: Vec<String>,
    /// 每工具精确目录覆盖（key 为工具标识，如 `.minimax` / `cherry-studio`），
    /// 作为额外根目录之外的兜底。
    #[serde(default)]
    pub tool_data_dirs: BTreeMap<String, String>,
    /// Polling interval seconds. 0 = use per-provider default.
    pub poll_interval_seconds: u32,
    /// Persisted dashboard position (last known). Restored on startup.
    pub dashboard_x: Option<i32>,
    pub dashboard_y: Option<i32>,
    /// Compact-mode flag.
    #[serde(default)]
    pub compact_mode: bool,
    /// 胶囊上次贴边的锚点。None = 上次是浮动。
    /// 存下来是为了重启后回到同一条边、同一个位置——否则每次启动都靠系统
    /// 随手放，用户看到的现象就是「窗口老在左上角」。
    #[serde(default)]
    pub dock: Option<crate::dock::DockAnchor>,
    /// Whether to launch automatically at system boot (Windows: HKCU Run key,
    /// managed via tauri-plugin-autostart).
    #[serde(default)]
    pub autostart: bool,
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
    /// Currency code used to render estimated costs (USD/CNY/...). Display-only.
    #[serde(default = "default_display_currency")]
    pub display_currency: String,
    /// UI language: "auto" (follow system), "zh-CN" or "en". Read by the
    /// frontend (via settings-changed) and by Rust for the tray menu, window
    /// titles and OS notifications. Display-only.
    #[serde(default = "default_language")]
    pub language: String,
    /// Multi-device hub role: "off" | "hub" | "agent" | "lan". "hub" listens on
    /// `hub_port` for other instances to report; "agent" reports up to
    /// `hub_base`; "lan" additionally discovers same-subnet peers via mDNS and
    /// reports to each of them (full mesh). Default off.
    #[serde(default = "default_hub_mode")]
    pub hub_mode: String,
    /// Local port the hub listener binds to when `hub_mode == "hub"`.
    #[serde(default = "default_hub_port")]
    pub hub_port: u16,
    /// Base URL of the remote hub to report to when `hub_mode == "agent"`
    /// (e.g. `http://192.168.1.20:43210`).
    #[serde(default)]
    pub hub_base: String,
    /// Whether an agent actually reports its own usage (in addition to being
    /// configured). `false` disables reporting.
    #[serde(default)]
    pub report_on: bool,
    /// Shared secret that hub requires (`Authorization: Bearer <token>`) on
    /// `/ingest` and `/devices`. Empty disables auth (trusted LAN only).
    #[serde(default)]
    pub hub_token: String,
    /// Whether the user has ever explicitly configured `hub_token`.
    ///
    /// Distinguishes "never set" from "deliberately cleared to disable auth":
    /// a missing flag (all pre-existing configs) means "never set", so enabling
    /// hub mints a secret once. Once the user has saved the setting — including
    /// saving it empty — we never overwrite their choice.
    #[serde(default)]
    pub hub_token_configured: bool,
    /// 用户手动覆盖的汇率（币种代码 → 每 1 USD 兑该币种数值）。覆盖值优先于
    /// 网络拉取值；键须为受支持币种（USD/CNY/TWD/HKD/JPY/EUR/GBP），非法值忽略。
    #[serde(default)]
    pub rate_overrides: HashMap<String, f64>,
    /// Optional outbound proxy for all provider/hub HTTP traffic.
    /// Accepts `http://host:port` or `socks5://host:port`; `None` uses the default client.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// 启动时联网检查 GitHub Releases 新版本。默认关闭——本项目以"本机数据
    /// 不外发"为卖点，联网行为必须逐项显式开启，更新检查也不例外。
    #[serde(default)]
    pub check_updates_on_start: bool,
    /// TokenRouter 本地路由代理配置（路由表/端口/阈值）。见 `router/` 模块
    /// 与 docs/superpowers/specs/2026-10-04-token-router-design.md。
    #[serde(default)]
    pub router: crate::router::config::RouterSettings,
}

fn default_hub_port() -> u16 {
    43210
}

fn default_hub_mode() -> String {
    "off".to_string()
}

/// 生成一个新的 hub 共享密钥（32 位十六进制 = 128 bit 随机熵）。
///
/// hub 过去允许 `hub_token` 留空并直接放行所有请求，在局域网里等于没有
/// 鉴权。改为「启用 hub 时若未设置密钥就自动生成并落盘」，让默认路径
/// 始终是安全的：用户若真想关闭鉴权，仍可显式清空该字段。
///
/// 熵源来自 `getrandom`（Windows 下即 BCryptGenRandom），不引入新依赖；
/// 不使用时间播种的伪随机——密钥强度直接决定 hub 的访问控制边界。
pub fn generate_hub_token() -> String {
    let mut buf = [0u8; 16];
    // 取不到系统随机源时退回全零，并由调用方决定是否启用；这里的失败
    // 概率极低（getrandom 在受支持的平台不会失败），不引入 panic。
    if getrandom::fill(&mut buf).is_err() {
        return "0".repeat(32);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn default_display_currency() -> String {
    "auto".to_string()
}

fn default_language() -> String {
    "auto".to_string()
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
            tool_data_roots: Vec::new(),
            tool_data_dirs: BTreeMap::new(),
            poll_interval_seconds: 0,
            dashboard_x: None,
            dashboard_y: None,
            compact_mode: false,
            dock: None,
            autostart: false,
            close_to_tray: default_close_to_tray(),
            notify_enabled: default_notify_enabled(),
            notify_warn_percent: default_notify_warn_percent(),
            notify_crit_percent: default_notify_crit_percent(),
            ring_window: default_ring_window(),
            edge_snap: default_edge_snap(),
            countdown_mode: default_countdown_mode(),
            display_currency: default_display_currency(),
            language: default_language(),
            hub_mode: default_hub_mode(),
            hub_port: default_hub_port(),
            hub_base: String::new(),
            report_on: false,
            hub_token: String::new(),
            hub_token_configured: false,
            rate_overrides: HashMap::new(),
            proxy_url: None,
            check_updates_on_start: false,
            router: crate::router::config::RouterSettings::default(),
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

    /// Read exactly what a poll loop needs - "is this account enabled" and
    /// "what is the global interval" - under a single read lock.
    ///
    /// `get()` clones the whole `Settings`: the accounts vector, the
    /// rate-override map and every `String` field. The scheduler called it on
    /// both of its wake-up paths purely to read two `Copy` values, so every
    /// wake-up allocated a full settings copy for nothing. This keeps the
    /// accounts list as the single source of truth (no mirrored set to keep in
    /// sync at four write sites) while making the per-wake read allocation
    /// free.
    pub async fn poll_view(&self, instance_id: &str) -> (bool, u32) {
        let s = self.cache.read().await;
        let enabled = s
            .accounts
            .iter()
            .any(|a| a.instance_id == instance_id && a.enabled);
        (enabled, s.poll_interval_seconds)
    }

    /// Synchronous read of the cached settings. Safe for one-shot startup use
    /// before the async runtime drives concurrent accesses (a write hold here
    /// would only come from an in-flight `save`, which hasn't happened yet).
    pub fn read_blocking(&self) -> Settings {
        self.cache.try_read().map(|s| s.clone()).unwrap_or_default()
    }

    /// 把本机工具的数据目录配置应用到扫描器的进程级快照。启动与设置保存时
    /// 各调一次即可（见 [`crate::local::roots`]）。空串条目被忽略，允许 UI
    /// 保留一个空的输入框而不影响解析。
    pub fn apply_tool_roots(&self) {
        let s = self.read_blocking();
        let extra = s
            .tool_data_roots
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .map(PathBuf::from)
            .collect();
        let overrides = s
            .tool_data_dirs
            .iter()
            .map(|(k, v)| (k.trim().to_string(), PathBuf::from(v.trim())))
            .filter(|(k, v)| !k.is_empty() && !v.as_os_str().is_empty())
            .collect();
        crate::local::roots::set_config(extra, overrides);
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

    /// Sync, non-async read of `compact_mode`. Same rationale as
    /// `close_to_tray_now`. Used at startup to decide whether to size and
    /// dock the window as a compact pill before it is shown.
    pub fn compact_mode_now(&self) -> bool {
        self.cache.try_read().map(|s| s.compact_mode).unwrap_or(false)
    }

    /// Sync, non-async read of `dock`. Same rationale as `compact_mode_now`:
    /// startup positioning runs synchronously inside `setup`, which can't await.
    pub fn dock_now(&self) -> Option<crate::dock::DockAnchor> {
        self.cache.try_read().ok().and_then(|s| s.dock)
    }

    /// Sync, non-async read of the persisted free-floating position.
    pub fn get_xy_now(&self) -> (Option<i32>, Option<i32>) {
        self.cache
            .try_read()
            .map(|s| (s.dashboard_x, s.dashboard_y))
            .unwrap_or((None, None))
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
    fn generate_hub_token_is_32_hex_and_unique() {
        let a = super::generate_hub_token();
        let b = super::generate_hub_token();
        assert_eq!(a.len(), 32, "token should be 32 hex chars: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "not hex: {a}");
        assert_ne!(a, b, "two tokens must differ (CSPRNG sanity)");
        assert_ne!(a, "0".repeat(32), "must not be the failure fallback");
    }

    #[test]
    fn old_config_without_token_flag_is_treated_as_unset() {
        // A pre-existing config has hub_token = "" and no hub_token_configured
        // key at all. serde(default) must yield false so the hub mints a secret
        // exactly once instead of silently running unauthenticated.
        let raw = "\
enabled_providers = []
poll_interval_seconds = 60
hub_mode = \"hub\"
hub_port = 43210
";
        let parsed: Settings = toml::from_str(raw).expect("parse legacy config");
        assert!(parsed.hub_token.is_empty());
        assert!(
            !parsed.hub_token_configured,
            "missing flag must default to false"
        );
    }

    #[test]
    fn explicitly_cleared_token_is_preserved_on_round_trip() {
        // User deliberately clears the secret to disable auth: the flag must
        // survive a save/load cycle, otherwise the next hub start would refill
        // it and silently re-enable authentication they turned off.
        let mut s = Settings::default();
        s.hub_mode = "hub".into();
        s.hub_token = String::new();
        s.hub_token_configured = true;
        let dumped = toml::to_string(&s).expect("serialize");
        let back: Settings = toml::from_str(&dumped).expect("deserialize");
        assert!(back.hub_token_configured, "flag lost on round trip");
        assert!(back.hub_token.is_empty());
    }

    #[test]
    fn autostart_field_parses_and_round_trips() {
        let raw = "\
enabled_providers = []
poll_interval_seconds = 60
autostart = true
";
        let parsed: Settings = toml::from_str(raw).expect("parse config with autostart");
        assert!(parsed.autostart);
        let dumped = toml::to_string(&parsed).expect("serialize settings");
        let reparsed: Settings = toml::from_str(&dumped).expect("reparse dumped settings");
        assert!(reparsed.autostart);
    }

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
        assert!(parsed.rate_overrides.is_empty());
        assert!(parsed.proxy_url.is_none());
        // Legacy configs carry autostart_hint_shown; serde ignores the
        // unknown key and the new field defaults to false.
        assert!(!parsed.autostart);
    }

    #[test]
    fn proxy_url_defaults_to_none_and_round_trips() {
        let raw = "\
enabled_providers = []
poll_interval_seconds = 60
";
        let parsed: Settings = toml::from_str(raw).expect("parse config without proxy");
        assert!(parsed.proxy_url.is_none());

        let with_proxy = "\
enabled_providers = []
poll_interval_seconds = 60
proxy_url = \"socks5://127.0.0.1:1080\"
";
        let parsed: Settings = toml::from_str(with_proxy).expect("parse config with proxy");
        assert_eq!(parsed.proxy_url.as_deref(), Some("socks5://127.0.0.1:1080"));

        let dumped = toml::to_string(&parsed).expect("serialize settings");
        let reparsed: Settings = toml::from_str(&dumped).expect("reparse dumped settings");
        assert_eq!(reparsed.proxy_url.as_deref(), Some("socks5://127.0.0.1:1080"));
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

    #[test]
    fn language_field_defaults_to_auto_and_round_trips() {
        // 旧配置没有 language 键：serde(default) 给 "auto"；显式值往返不丢。
        let raw = "\
enabled_providers = []
poll_interval_seconds = 30
";
        let parsed: Settings = toml::from_str(raw).expect("legacy config must parse");
        assert_eq!(parsed.language, "auto");

        let with_lang = "\
enabled_providers = []
poll_interval_seconds = 30
language = \"en\"
";
        let parsed: Settings = toml::from_str(with_lang).expect("config with language");
        assert_eq!(parsed.language, "en");
        let dumped = toml::to_string(&parsed).expect("serialize");
        let back: Settings = toml::from_str(&dumped).expect("reparse");
        assert_eq!(back.language, "en");
    }

    #[test]
    fn a_config_without_the_dock_field_deserialises_to_none() {
        // 老配置里根本没有 dock 这行；serde(default) 必须给出 None 而不是报错，
        // 否则升级即读不出配置，用户会以为设置被清空了。
        // raw 里必须列全所有**没有** #[serde(default)] 的字段，否则解析失败。
        let raw = "\
enabled_providers = []
poll_interval_seconds = 30
dashboard_x = 100
dashboard_y = 200
compact_mode = true
";
        let s: Settings = toml::from_str(raw).expect("old config must still parse");
        assert_eq!(s.dock, None);
    }

    #[test]
    fn a_config_with_a_dock_field_round_trips() {
        use crate::dock::{DockAnchor, DockSide};
        let a = DockAnchor { side: DockSide::Bottom, along: 640 };
        let base = Settings::default();
        let s = Settings { dock: Some(a), ..base };
        let text = toml::to_string(&s).expect("serialize");
        let back: Settings = toml::from_str(&text).expect("deserialize");
        assert_eq!(back.dock, Some(a));
    }
}
