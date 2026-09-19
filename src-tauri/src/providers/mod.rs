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

/// Unit a usage value is measured in. Frontend uses this for label rendering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UsageUnit {
    Tokens,
    /// Volcano AgentPlan "Agent Fuel Point"
    Afp,
    Cny,
    Credits,
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
/// Implementations must be `Send + Sync` so they can live in a `tokio::task`
/// and be polled concurrently with other providers.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier - matches `Credentials` storage key.
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn auth_kind(&self) -> AuthKind;

    /// Fetch the latest usage. Implementations should:
    /// 1. Use the provided credentials (DO NOT cache them internally).
    /// 2. Return a complete `UsageSnapshot` with whatever windows the API exposes.
    /// 3. Surface failures as `ProviderError` - the scheduler handles them.
    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError>;
}

/// In-memory registry of all known providers, keyed by `id()`.
/// Frontend reads the list to know which providers exist; scheduler iterates
/// enabled ones to poll.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P: Provider + 'static>(&mut self, provider: P) {
        let id = provider.id();
        self.providers.insert(id, Arc::new(provider));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<dyn Provider>> {
        let mut list: Vec<_> = self.providers.values().cloned().collect();
        list.sort_by_key(|p| p.id().to_string());
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
