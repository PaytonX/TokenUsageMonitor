//! ChatGPT Codex (subscription) usage provider — EXPERIMENTAL.
//!
//! Credentials come from the local Codex CLI OAuth token file
//! `~/.codex/auth.json` (`tokens.access_token` + `tokens.account_id`).
//! The constructor never touches the network or the keyring; `fetch_usage`
//! re-reads the file so a fresh `codex login` is picked up on the next poll.
//!
//! Endpoint (undocumented, may change without notice):
//!   - `GET https://chatgpt.com/backend-api/wham/usage`
//!     Returns `rate_limit.primary_window` (5h) and `secondary_window`
//!     (weekly), each with `used_percent` and `reset_at` (unix seconds).
//!
//! NOTE: the approved spec sketched `.../codex/usage`; live probing showed
//! the real path is `.../wham/usage` — we implement the live value and keep
//! the provider marked experimental in the UI.
//!
//! Auth: `Authorization: Bearer <access_token>` (+ `ChatGPT-Account-ID`
//! when the auth file carries an account id).

use crate::providers::{
    AuthKind, CostSource, Credentials, Provider, ProviderError, UsageSnapshot, UsageUnit,
    UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// Parsed contents of `~/.codex/auth.json`.
#[derive(Debug, Clone)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: String,
    pub last_refresh: String,
}

#[derive(Debug, Deserialize, Default)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: CodexTokens,
    #[serde(default)]
    last_refresh: String,
}

#[derive(Debug, Deserialize, Default)]
struct CodexTokens {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    account_id: String,
}

/// Parse auth.json contents; `None` when unparsable or the token is blank.
fn parse_auth_json(raw: &str) -> Option<CodexAuth> {
    let file: CodexAuthFile = serde_json::from_str(raw).ok()?;
    if file.tokens.access_token.trim().is_empty() {
        return None;
    }
    Some(CodexAuth {
        access_token: file.tokens.access_token,
        account_id: file.tokens.account_id,
        last_refresh: file.last_refresh,
    })
}

/// Auto-detect the local Codex CLI credentials. Reads the file only — no
/// network, so it is safe to call from tests, constructors and IPC.
pub fn detect_auth() -> Option<CodexAuth> {
    let path = dirs::home_dir()?.join(".codex").join("auth.json");
    let raw = std::fs::read_to_string(path).ok()?;
    parse_auth_json(&raw)
}

#[derive(Debug, Deserialize, Default)]
struct RateWindow {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct RateLimit {
    #[serde(default)]
    primary_window: Option<RateWindow>,
    #[serde(default)]
    secondary_window: Option<RateWindow>,
}

#[derive(Debug, Deserialize, Default)]
struct UsageResponse {
    #[serde(default)]
    rate_limit: RateLimit,
}

/// Map one rate-limit window to the unified `WindowUsage` (percent scale).
fn percent_window(window: Option<RateWindow>) -> Option<WindowUsage> {
    window.map(|w| WindowUsage {
        used: w.used_percent.max(0.0),
        quota: 100.0,
        unit: UsageUnit::Percent,
        reset_at: w.reset_at.and_then(|t| Utc.timestamp_opt(t, 0).single()),
        over_quota: w.used_percent >= 100.0,
        cost_source: CostSource::ProviderReported,
        tokens: None,
    })
}

pub struct CodexProvider {
    http: Client,
    instance_id: String,
    label: String,
}

impl CodexProvider {
    /// `storage` is accepted for registry-signature uniformity; the Codex
    /// endpoint exposes no daily-spend data, so nothing is persisted.
    pub fn new(http: Client, _storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            instance_id,
            label,
        }
    }

    fn self_id(&self) -> &str {
        &self.instance_id
    }
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::LocalToken
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        // Resolve the token: manual entry wins; an empty entry falls back to
        // the local auth.json (auto-detected on every poll).
        let (token, account_id) = match creds {
            Credentials::LocalToken { token } if !token.trim().is_empty() => {
                let account_id = detect_auth().map(|a| a.account_id).unwrap_or_default();
                (token.clone(), account_id)
            }
            Credentials::LocalToken { .. } => detect_auth()
                .map(|a| (a.access_token, a.account_id))
                .ok_or(ProviderError::NotConfigured)?,
            _ => {
                return Err(ProviderError::Auth {
                    message: "Codex requires LocalToken credentials".into(),
                })
            }
        };

        let mut req = self.http.get(USAGE_URL).bearer_auth(&token);
        if !account_id.trim().is_empty() {
            req = req.header("ChatGPT-Account-ID", account_id.trim());
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "Codex 凭证失效，请在本机运行 `codex login` 重新授权".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("Codex usage HTTP {}", resp.status()),
            });
        }
        let usage: UsageResponse = resp.json().await?;

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                five_hour: percent_window(usage.rate_limit.primary_window),
                weekly: percent_window(usage.rate_limit.secondary_window),
                ..Default::default()
            },
            heatmap: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rate_limit_windows() {
        let raw = r#"{"rate_limit":{"primary_window":{"used_percent":42.5,"reset_at":1790000000},"secondary_window":{"used_percent":100.0}}}"#;
        let parsed: UsageResponse = serde_json::from_str(raw).expect("parse usage fixture");
        let five_hour = percent_window(parsed.rate_limit.primary_window).expect("primary");
        assert!((five_hour.used - 42.5).abs() < 1e-9);
        assert_eq!(five_hour.quota, 100.0);
        assert_eq!(five_hour.unit, UsageUnit::Percent);
        assert!(five_hour.reset_at.is_some());
        assert!(!five_hour.over_quota);
        let weekly = percent_window(parsed.rate_limit.secondary_window).expect("secondary");
        assert!((weekly.used - 100.0).abs() < 1e-9);
        assert!(weekly.over_quota);
        assert!(weekly.reset_at.is_none());
    }

    #[test]
    fn missing_fields_default_to_none() {
        let parsed: UsageResponse = serde_json::from_str("{}").expect("parse empty fixture");
        assert!(parsed.rate_limit.primary_window.is_none());
        assert!(percent_window(parsed.rate_limit.primary_window).is_none());
    }

    #[test]
    fn parse_auth_json_requires_access_token() {
        let raw = r#"{"tokens":{"access_token":"tok","account_id":"acc-1"},"last_refresh":"2026-09-01"}"#;
        let auth = parse_auth_json(raw).expect("auth parsed");
        assert_eq!(auth.access_token, "tok");
        assert_eq!(auth.account_id, "acc-1");
        assert_eq!(auth.last_refresh, "2026-09-01");
        assert!(parse_auth_json(r#"{"tokens":{}}"#).is_none());
        assert!(parse_auth_json("not json").is_none());
    }
}
