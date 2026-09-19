//! Provider abstraction layer.
//!
//! Every AI provider (MiniMax, DeepSeek, Volcano AgentPlan, …) implements
//! the [`Provider`] trait. The frontend never knows which concrete provider
//! it's talking to - it consumes the unified [`UsageSnapshot`] type.
//!
//! Adding a new provider = one new module + one line in the registry.

pub mod deepseek;
pub mod minimax;
pub mod mock;
pub mod openai;
pub mod volcengine;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
        let default_accent = PRESETS
            .iter()
            .find(|p| p.kind == kind)
            .map(|p| p.default_accent)
            .unwrap_or("#29b6f6")
            .to_string();
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

/// A built-in provider preset exposed in the Settings "account" pane. Users
/// create accounts from these presets; the app knows nothing more.
#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub kind: &'static str,
    pub display_name: &'static str,
    pub auth_kind: AuthKind,
    pub default_accent: &'static str,
}

/// The fixed catalogue of built-in provider kinds. Keep in sync with the
/// `Provider` implementations in this module directory.
pub const PRESETS: &[Preset] = &[
    Preset {
        kind: "minimax",
        display_name: "MiniMax Token Plan",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#667eea",
    },
    Preset {
        kind: "deepseek",
        display_name: "DeepSeek API",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#10b981",
    },
    Preset {
        kind: "volcengine",
        display_name: "Volcano AgentPlan",
        auth_kind: AuthKind::AccessKeySecret,
        default_accent: "#f59e0b",
    },
    Preset {
        kind: "openai",
        display_name: "OpenAI 计费",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#10a37f",
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
