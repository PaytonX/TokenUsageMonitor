//! Provider abstraction layer.
//!
//! Every AI provider (MiniMax, DeepSeek, Volcano AgentPlan, …) implements
//! the [`Provider`] trait. The frontend never knows which concrete provider
//! it's talking to - it consumes the unified [`UsageSnapshot`] type.
//!
//! Adding a new provider = one new module + one line in the registry.

pub mod codex;
pub mod deepseek;
pub mod kimi;
pub mod minimax;
pub mod minimax_api;
pub mod mock;
pub mod openai;
pub mod opencode;
pub mod volcengine;
pub mod volcengine_api;
pub mod xai;
pub mod xiaomi;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::Storage;

/// How a provider expects its credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Single API key used as `Authorization: Bearer <key>`.
    /// (MiniMax Token Plan, DeepSeek)
    BearerKey,
    /// Access Key + Secret Key, signed with HMAC-SHA256.
    /// (Volcano / Volcengine)
    AccessKeySecret,
    /// Local token-file credential (e.g. ChatGPT Codex `~/.codex/auth.json`).
    LocalToken,
}

/// Credentials container - one variant per [`AuthKind`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Credentials {
    BearerKey { api_key: String },
    AccessKeySecret {
        access_key: String,
        secret_key: String,
    },
    /// Token read from a local file on this machine (no keyring entry needed).
    LocalToken { token: String },
}

/// A user-configurable account bound to one provider kind. Multiple accounts
/// for the same provider kind are allowed (multi-account), each with its own
/// label, accent colour and credentials (keyed by `instance_id` in keyring).
///
/// `instance_id` is the stable key used everywhere the old `provider_id`
/// singleton was used: keyring entry, snapshot `provider_id`, heatmap storage
/// keyspace and the frontend card key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMeta {
    /// Stable, unique id for this account.
    pub instance_id: String,
    /// Which built-in provider this account uses: "deepseek" | "minimax" |
    /// "volcengine".
    pub provider_kind: String,
    /// User-facing display label (defaults to the preset display name).
    pub label: String,
    /// Per-account accent colour, `#RRGGBB`.
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    /// Whether this account participates in polling.
    pub enabled: bool,
    /// Optional free-form note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_accent_color() -> String {
    "#29b6f6".to_string()
}

impl AccountMeta {
    /// Build a fresh account for a given preset kind with sane defaults and a
    /// unique `instance_id`.
    pub fn new(kind: &str, label: String) -> Self {
        let default_accent = accent_for(kind);
        Self {
            instance_id: next_instance_id(kind),
            provider_kind: kind.to_string(),
            label,
            accent_color: default_accent,
            enabled: true,
            note: None,
        }
    }
}

/// Resolve a preset's default accent colour for a given account `kind`. Looks
/// up the top-level preset first, then its `sub_modes` (so e.g. `minimax_api`
/// falls back to MiniMax's accent). Falls back to the neutral default.
pub fn accent_for(kind: &str) -> String {
    for preset in PRESETS {
        if preset.kind == kind {
            return preset.default_accent.to_string();
        }
        if let Some(modes) = preset.sub_modes {
            if modes.iter().any(|m| m.kind == kind) {
                return preset.default_accent.to_string();
            }
        }
    }
    "#29b6f6".to_string()
}

/// Generate a fresh, reasonably-unique instance id for a new account. Local
/// monotonic counter + epoch seconds is enough: ids only need to be unique
/// within one machine's config, not globally.
fn next_instance_id(kind: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{kind}-{ts}-{n}")
}

/// An account-load mode selectable under a multi-mode preset (e.g. a provider
/// with both a fixed subscription tier and pay-as-you-go API). Maps to a single
/// account `kind`.
#[derive(Debug, Clone, Serialize)]
pub struct PresetSubMode {
    /// Account kind produced when this mode is chosen.
    pub kind: &'static str,
    /// Short label shown in the choice menu.
    pub label: &'static str,
    /// Optional explanatory note (shown as a tooltip on the menu item).
    pub note: &'static str,
    /// true = skeleton / not-yet-implemented (the UI greys it out and tags it).
    pub limited: bool,
}

/// A built-in provider preset exposed in the Settings "account" pane. Users
/// create accounts from these presets; the app knows nothing more.
///
/// When `sub_modes` is present, the "add account" UI shows a chooser menu
/// instead of adding immediately; each entry maps to a concrete account kind.
/// When `None`, the preset adds directly.
#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub kind: &'static str,
    pub display_name: &'static str,
    pub auth_kind: AuthKind,
    pub default_accent: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_modes: Option<&'static [PresetSubMode]>,
    /// Experimental providers show an「实验」badge in Settings and are opt-in.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub experimental: bool,
}

/// The fixed catalogue of built-in provider kinds. Keep in sync with the
/// `Provider` implementations in this module directory.
pub const PRESETS: &[Preset] = &[
    Preset {
        kind: "minimax",
        display_name: "MiniMax Token Plan",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#667eea",
        sub_modes: Some(&[
            PresetSubMode {
                kind: "minimax",
                label: "Token Plan · 固定订阅",
                note: "订阅档，按套餐固定额度计费",
                limited: false,
            },
            PresetSubMode {
                kind: "minimax_api",
                label: "API 按量",
                note: "官方暂未提供余额/用量查询接口，该模式尚未实装",
                limited: true,
            },
        ]),
        experimental: false,
    },
    Preset {
        kind: "deepseek",
        display_name: "DeepSeek API",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#10b981",
        sub_modes: None,
        experimental: false,
    },
    Preset {
        kind: "volcengine",
        display_name: "Volcano AgentPlan",
        auth_kind: AuthKind::AccessKeySecret,
        default_accent: "#f59e0b",
        sub_modes: Some(&[
            PresetSubMode {
                kind: "volcengine",
                label: "AgentPlan · 固定订阅",
                note: "AgentPlan 订阅档，按 AFP 配额计费",
                limited: false,
            },
            PresetSubMode {
                kind: "volcengine_api",
                label: "API 按量 · GetInferenceUsage",
                note: "按推理 token 后付费，查询真实用量",
                limited: false,
            },
        ]),
        experimental: false,
    },
    Preset {
        kind: "openai",
        display_name: "OpenAI 计费",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#10a37f",
        sub_modes: None,
        experimental: false,
    },
    Preset {
        kind: "xiaomi",
        display_name: "小米 MiMo",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#ff6900",
        sub_modes: Some(&[
            PresetSubMode {
                kind: "xiaomi_plan",
                label: "Token Plan",
                note: "订阅档，按 Credits 配额计费",
                limited: false,
            },
            PresetSubMode {
                kind: "xiaomi_api",
                label: "API 按量",
                note: "官方暂未提供余额/用量查询接口，该模式尚未实装",
                limited: true,
            },
        ]),
        experimental: false,
    },
    Preset {
        kind: "xai",
        display_name: "xAI Grok",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#1d1d1d",
        sub_modes: None,
        experimental: false,
    },
    Preset {
        kind: "kimi",
        display_name: "Kimi",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#16c2a3",
        sub_modes: Some(&[
            PresetSubMode {
                kind: "kimi",
                label: "国内站",
                note: "api.moonshot.cn，CNY 计费",
                limited: false,
            },
            PresetSubMode {
                kind: "kimi_global",
                label: "国际站",
                note: "api.moonshot.ai，USD 计费，Key 与国内站不通用",
                limited: false,
            },
        ]),
        experimental: false,
    },
    Preset {
        kind: "opencode",
        display_name: "OpenCode Go",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#e8eaed",
        sub_modes: None,
        // 端点未公开文档（源自社区逆向），且仅覆盖 Go 订阅，标记为实验。
        experimental: true,
    },
    Preset {
        kind: "codex",
        display_name: "ChatGPT Codex",
        auth_kind: AuthKind::LocalToken,
        default_accent: "#10a37f",
        sub_modes: None,
        experimental: true,
    },
];

/// Resolve a preset's display name by kind, falling back to the kind itself.
pub fn preset_display_name(kind: &str) -> String {
    PRESETS
        .iter()
        .find(|p| p.kind == kind)
        .map(|p| p.display_name)
        .unwrap_or(kind)
        .to_string()
}

/// Unit a usage value is measured in. Frontend uses this for label rendering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UsageUnit {
    Tokens,
    /// Volcano AgentPlan "Agent Fuel Point"
    Afp,
    Cny,
    Credits,
    /// OpenAI billing usage, measured in US Dollars.
    Usd,
    /// MiniMax Token Plan reports percentages, not absolute token counts.
    Percent,
}

impl UsageUnit {
    pub fn label(self) -> &'static str {
        match self {
            UsageUnit::Tokens => "tokens",
            UsageUnit::Afp => "AFP",
            UsageUnit::Cny => "¥",
            UsageUnit::Credits => "credits",
            UsageUnit::Usd => "$",
            UsageUnit::Percent => "%",
        }
    }
}

/// A single rolling-window usage bar (e.g. "5 hours", "weekly", "monthly").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowUsage {
    pub used: f64,
    pub quota: f64,
    pub unit: UsageUnit,
    /// When this window resets, in UTC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<DateTime<Utc>>,
    /// True if used > quota (overage detected).
    pub over_quota: bool,
    /// Whether the value this window reports is authoritative (provider-reported),
    /// estimated, or of unknown provenance. Lets the UI label estimated figures
    /// instead of presenting them as exact.
    #[serde(default)]
    pub cost_source: CostSource,
    /// Detailed token split (input/cache-reused/output) when the provider
    /// reports it. `None` for providers that only give an aggregate figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenBreakdown>,
}

/// Per-window token usage split, mirroring the fields tokscale exposes. Enables
/// cache-hit statistics and input/output-differentiated cost estimates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenBreakdown {
    /// Input tokens consumed (includes cache-read reuse counted at input rate).
    pub input: f64,
    /// Input tokens served from the provider's prompt cache ("cache hit").
    pub cache_read: f64,
    /// Output (completion) tokens generated.
    pub output: f64,
    /// Which model this breakdown pertains to, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// Where a `WindowUsage` value's cost/usage provenance comes from. Mirrors
/// tokscale's `CostSource`: providers that report exact figures annotate
/// `ProviderReported`; anything derived/estimated is `Estimated`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    #[default]
    Unknown,
    ProviderReported,
    Estimated,
}

impl WindowUsage {
    /// Percentage in [0, 1]; capped at 1.0 for progress display.
    pub fn percent(&self) -> f64 {
        if self.quota <= 0.0 {
            0.0
        } else {
            (self.used / self.quota).clamp(0.0, 1.0)
        }
    }

    /// Remaining absolute value.
    pub fn remaining(&self) -> f64 {
        (self.quota - self.used).max(0.0)
    }
}

/// Balance info for prepaid / metered providers (DeepSeek CNY balance).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceInfo {
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
    pub currency: String,
    pub is_available: bool,
}

/// Four standard rolling windows + optional balance. Missing windows are `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageWindows {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<WindowUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily: Option<WindowUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly: Option<WindowUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly: Option<WindowUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<BalanceInfo>,
}

/// A single day's usage value for heatmap rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapCell {
    pub date: NaiveDate,
    /// Raw usage value (used quota).
    pub value: f64,
    pub unit: UsageUnit,
}

/// The unified snapshot every `Provider::fetch_usage` returns.
/// Frontend renders this verbatim - no per-provider branching in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider_id: String,
    pub provider_display_name: String,
    /// Plan tier badge label, e.g. "Max", "Ultra", "Pro 1M".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_tier: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub windows: UsageWindows,
    /// Pre-built heatmap if the API returned daily breakdown; None otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<Vec<HeatmapCell>>,
}

/// Burn rate derived from two consecutive snapshots of the same window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnInfo {
    /// Usage units consumed per minute.
    pub rate_per_min: f64,
    /// Unit of `rate_per_min` (matches the window being diffed).
    pub unit: UsageUnit,
    /// Estimated seconds until the window quota is exhausted at this rate.
    /// None when the quota is unknown or the rate is non-positive.
    pub eta_seconds: Option<u64>,
}

/// Payload of the `usage-updated` event: the snapshot plus pulse metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageUpdate {
    pub snapshot: UsageSnapshot,
    pub burn: Option<BurnInfo>,
    /// True when usage grew since the previous snapshot (active generation).
    pub active: bool,
}

/// Provider error surfaced to the frontend.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum ProviderError {
    #[error("network error: {message}")]
    Network { message: String },
    #[error("authentication failed: {message}")]
    Auth { message: String },
    #[error("provider returned malformed response: {message}")]
    Parse { message: String },
    #[error("provider rate-limited the request")]
    RateLimited,
    #[error("credentials not configured")]
    NotConfigured,
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl From<reqwest::Error> for ProviderError {
    fn from(err: reqwest::Error) -> Self {
        ProviderError::Network {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for ProviderError {
    fn from(err: serde_json::Error) -> Self {
        ProviderError::Parse {
            message: err.to_string(),
        }
    }
}

/// The trait every provider implements.
///
/// A provider instance is bound to one user account: its `id()` is the
/// account's `instance_id` and `display_name()` is the account label, so two
/// accounts on the same kind (e.g. two DeepSeek keys) are fully isolated in
/// storage and in the frontend.
///
/// Implementations must be `Send + Sync` so they can live in a `tokio::task`
/// and be polled concurrently with other providers.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable account identifier - matches the keyring storage key.
    fn id(&self) -> String;
    /// Provider kind ("deepseek" | "minimax" | "volcengine"), for interval
    /// defaults.
    fn kind(&self) -> &'static str;
    /// User-facing account label.
    fn display_name(&self) -> String;
    fn auth_kind(&self) -> AuthKind;

    /// Fetch the latest usage. Implementations should:
    /// 1. Use the provided credentials (DO NOT cache them internally).
    /// 2. Return a complete `UsageSnapshot` with whatever windows the API exposes.
    /// 3. Surface failures as `ProviderError` - the scheduler handles them.
    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError>;
}

/// In-memory registry of live provider instances, keyed by `instance_id`.
/// The scheduler iterates these; IPC queries them. Mutated at runtime when
/// accounts are added/removed in Settings.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, provider: Arc<dyn Provider>) {
        let id = provider.id();
        self.providers.insert(id, provider);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }

    pub fn remove(&mut self, id: &str) {
        self.providers.remove(id);
    }

    pub fn list(&self) -> Vec<Arc<dyn Provider>> {
        let mut list: Vec<_> = self.providers.values().cloned().collect();
        list.sort_by_key(|p| p.id());
        list
    }
}

/// Snapshot of the latest `UsageSnapshot` per provider, with the last error
/// if any. Wrapped in `RwLock` so the scheduler can update while IPC reads.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct ProviderState {
    pub snapshot: Option<UsageSnapshot>,
    pub last_error: Option<ProviderError>,
    pub last_updated_at: Option<DateTime<Utc>>,
}

pub type SharedProviderState = Arc<RwLock<HashMap<String, ProviderState>>>;

/// Helper to compute percent across a snapshot for the compact ball color.
impl UsageSnapshot {
    /// Lowest remaining percentage across all available windows.
    /// Used for compact-ball progress ring color (red when any window is exhausted).
    pub fn min_remaining_percent(&self) -> f64 {
        let windows = [
            self.windows.five_hour.as_ref(),
            self.windows.daily.as_ref(),
            self.windows.weekly.as_ref(),
            self.windows.monthly.as_ref(),
        ];
        windows
            .iter()
            .flatten()
            .map(|w| 1.0 - w.percent())
            .fold(1.0_f64, f64::min)
    }
}

/// Signature shared by every concrete provider constructor in the registry.
/// All providers take the same (http client, storage, instance id, label) and
/// return a boxed `Provider`. Keeping this uniform lets the registry stay a
/// small table instead of a hand-written `match` that grows with each provider.
pub type ProviderBuilder =
    fn(Client, Arc<Storage>, String, String) -> Arc<dyn Provider>;

/// Static registry: every account kind → its constructor. This is the single
/// source of truth for kind dispatch — adding a provider is one entry here
/// plus one `PRESETS` entry (top-level kind or a `sub_modes` item); the
/// frontend only adds the kind's short-name mapping. No match arms to update.
pub static PROVIDER_REGISTRY: &[(&'static str, ProviderBuilder)] = &[
    ("minimax", |h, s, i, l| Arc::new(minimax::MiniMaxProvider::new(h, s, i, l))),
    ("minimax_api", |h, s, i, l| Arc::new(minimax_api::MiniMaxApiProvider::new(h, s, i, l))),
    ("deepseek", |h, s, i, l| Arc::new(deepseek::DeepSeekProvider::new(h, s, i, l))),
    ("volcengine", |h, s, i, l| Arc::new(volcengine::VolcengineProvider::new(h, s, i, l))),
    ("volcengine_api", |h, s, i, l| Arc::new(volcengine_api::VolcengineApiProvider::new(h, s, i, l))),
    ("openai", |h, s, i, l| Arc::new(openai::OpenAIProvider::new(h, s, i, l))),
    (
        "opencode",
        |h, s, i, l| Arc::new(opencode::OpenCodeProvider::new(h, s, i, l)),
    ),
    ("xiaomi_plan", |h, s, i, l| Arc::new(xiaomi::XiaoMiPlanProvider::new(h, s, i, l))),
    ("xiaomi_api", |h, s, i, l| Arc::new(xiaomi::XiaoMiApiProvider::new(h, s, i, l))),
    ("xai", |h, s, i, l| Arc::new(xai::XaiProvider::new(h, s, i, l))),
    ("kimi", |h, s, i, l| Arc::new(kimi::KimiProvider::new(h, s, i, l))),
    (
        "kimi_global",
        |h, s, i, l| {
            Arc::new(kimi::KimiProvider::new_with_base(h, s, i, l, kimi::GLOBAL_BASE))
        },
    ),
    ("codex", |h, s, i, l| Arc::new(codex::CodexProvider::new(h, s, i, l))),
];

/// Build a live provider for an account `kind`, or `None` for an unknown kind.
///
/// This is the single dispatch point between an account kind and its
/// constructor, keeping provider instantiation out of `lib.rs`. The kind list
/// lives in [`PROVIDER_REGISTRY`]; [`provider_kinds`] derives from it too.
pub fn build_provider(
    kind: &str,
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
) -> Option<Arc<dyn Provider>> {
    let build = PROVIDER_REGISTRY
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, build)| build)?;
    Some(build(http, storage, instance_id, label))
}

/// Every registered account kind, derived from [`PROVIDER_REGISTRY`] so the
/// list can never drift from the dispatch table.
pub fn provider_kinds() -> Vec<&'static str> {
    PROVIDER_REGISTRY.iter().map(|(k, _)| *k).collect()
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// Registry kinds must be unique — a duplicate would make the lookup
    /// order-dependent and silently shadow one constructor.
    #[test]
    fn registry_kinds_are_unique() {
        let mut kinds = provider_kinds();
        let total = kinds.len();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), total);
    }

    /// Every registry kind must be reachable from `PRESETS` (top-level kind
    /// or a `sub_modes` entry) — otherwise the Settings UI could offer an
    /// account the backend cannot build.
    #[test]
    fn every_registry_kind_is_exposed_by_a_preset() {
        for kind in provider_kinds() {
            let exposed = PRESETS.iter().any(|p| {
                p.kind == kind
                    || p.sub_modes
                        .map_or(false, |modes| modes.iter().any(|m| m.kind == kind))
            });
            assert!(exposed, "kind `{kind}` is registered but not exposed by PRESETS");
        }
    }

    /// Every registered constructor must build without touching the network:
    /// `new` is pure wiring, so a temp SQLite file suffices.
    #[test]
    fn every_kind_builds_a_provider() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("pulse_registry_unit_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let storage = Arc::new(Storage::open(&path).unwrap());
        let http = Client::new();
        for kind in provider_kinds() {
            let p = build_provider(
                kind,
                http.clone(),
                storage.clone(),
                format!("{kind}-instance"),
                format!("{kind}-label"),
            );
            assert!(p.is_some(), "kind `{kind}` failed to build");
        }
        let _ = std::fs::remove_file(&path);
    }
}
