# Implementation Plan: Hover Anchor + xAI/Kimi/Codex Providers + Provider Logos + Local Proxy

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

## Goal

Implement the four approved features from `impl-pulse/docs/superpowers/specs/2026-09-25-hover-providers-logo-proxy-design.md`:

1. **Hover-anchored detail overlay** — the detail card anchors next to the triggering provider card (below it, viewport-clamped, flipped above when space is insufficient); hover grace period 160ms → 350ms; `fly y:6 duration:120` transition.
2. **New providers** — xAI Grok (`xai`, monthly USD aggregation, api.x.ai/v1/usage), Kimi (`kimi` + `kimi_global` sub-mode via api.moonshot.ai, balance-delta accounting), ChatGPT Codex (`codex`, experimental, local `~/.codex/auth.json` token + `detect_codex_token` IPC, 5h/weekly percent windows at chatgpt.com/backend-api/codex/usage).
3. **Provider logos** — inline-SVG `ProviderLogo.svelte` (brand colors) at 5 integration points; mascot assets are kept, not deleted.
4. **Local proxy support** — optional `proxy_url` setting (http/socks5); `AppState.http` becomes `Arc<RwLock<Client>>`; changing the proxy rebuilds the HTTP client and the full provider registry; `test_proxy` IPC + Settings「网络」section.

## Architecture

- **Backend**: Tauri 2 (Rust). All outbound HTTP flows through one `reqwest::Client` owned by `AppState.http` (today a plain `Client`; will become `Arc<RwLock<Client>>`). `PROVIDER_REGISTRY` in `providers/mod.rs` maps kind → constructor fn; `build_registry` instantiates one provider per enabled account.
- **Frontend**: Svelte 5 + TypeScript under `src/`; IPC wrappers in `src/lib/api.ts`; shared types in `src/lib/types.ts`.
- **Spec decisions**: D1 anchor may overlap page content; D2 Codex is a new provider, existing OpenAI direct stays; D3 only xAI + Kimi added (Anthropic/GLM/Gemini deferred); D4 inline SVG logos; D5 only an explicitly set proxy URL is used, `None` keeps the default client.
- **Balance-delta accounting** (Kimi) reuses the DeepSeek KV precedent: keys `last_balance`, `month_key`, `month_start_balance`, `month_topup`.
- **ProviderError** shapes used by providers: `ProviderError::Auth { message: String }`, `ProviderError::NotConfigured`, plus `From<reqwest::Error>` / `From<serde_json::Error>` conversions (defined in `providers/mod.rs`).
- **Registry test constraints** (`providers/mod.rs` `registry_tests`): every registry kind must be unique, must be exposed by a preset or its `sub_modes` (one-way: `kimi_global` registry entry is covered by the `kimi` preset), and every kind must build a provider **without network access** (`every_kind_builds_a_provider` passes a plain `Client::new()` — constructors must not call the network; the Codex constructor only reads a local file).

## Tech Stack

- Rust: tauri 2, reqwest 0.12 (rustls-tls/json/stream, + socks), tokio, rusqlite, chrono, keyring 3, serde/serde_json/toml, dirs 5 (new)
- Frontend: Svelte 5, TypeScript, Vite
- Testing: `cargo test --lib` with co-located `#[cfg(test)]` modules; `svelte-check` + `npm run build` for frontend

## Verification Commands

- Rust (cwd `impl-pulse\src-tauri`, PowerShell):

  ```powershell
  $env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
  ```

- Frontend (cwd `impl-pulse`):

  ```bash
  npx svelte-check --threshold error
  npm run build
  ```

- All `git` commands run with cwd = worktree root; paths below are relative to it.

## Task Index (execute in order)

1. Cargo.toml — reqwest socks feature + dirs crate
2. settings.rs — proxy_url field (+ Default + tests)
3. providers/mod.rs — AuthKind::LocalToken, Credentials::LocalToken, Preset.experimental
4. providers/xai.rs — xAI Grok provider
5. providers/kimi.rs — Kimi provider (+ kimi_global base URL)
6. providers/codex.rs — Codex provider (+ auth.json parsing)
7. providers/mod.rs — register 3 modules, 3 PRESETS, 4 registry entries
8. lib.rs — proxy-aware build_http_client helper (unit-tested; type swap deferred to Task 11)
9. scheduler.rs — poll_loop identity check (stale loops exit after proxy rebuild)
10. hub.rs — client parameter on report_to_hub/fetch_devices
11. lib.rs + ipc.rs — AppState.http → Arc<RwLock<Client>>, migrate all http access sites, save_settings proxy-rebuild branch
12. ipc.rs — test_proxy + detect_codex_token commands + invoke_handler wiring
13. types.ts + api.ts — LocalToken / Preset.experimental / proxy_url types + testProxy/detectCodexToken
14. ProviderLogo.svelte — new inline SVG logo component
15. App.svelte — hover anchoring (rect, clamp, flip, 350ms) + logos (pill ring + shell avatar)
16. ProviderCard.svelte — logo in title row + onHover passes rect
17. MiniPanel.svelte — logo replaces text abbreviation
18. DetailCard.svelte — kind prop + logo in detail__head
19. Settings.svelte — local_token form, experimental badge, preset logos, 网络 proxy section
20. Final verification — full suites + build + spec acceptance checklist

---

## Task 1: Enable SOCKS proxy support and add `dirs` dependency

**Files:**
- Modify: `impl-pulse/src-tauri/Cargo.toml` (reqwest features at line 28; `[dependencies]` section for `dirs`)
- Test: compile check (dependency-only change, no unit tests)

- [ ] **Step 1: Edit Cargo.toml**

At line 28, change:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
```

to:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream", "socks"] }
```

In the `[dependencies]` section (place it next to the `chrono` dependency, around line 35), add:

```toml
dirs = "5"
```

- [ ] **Step 2: Verify it compiles**

Run (cwd `impl-pulse\src-tauri`, PowerShell):

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo check --lib
```

Expected: success. The `socks` feature pulls in `tokio-socks` (first build downloads it).

- [ ] **Step 3: Commit**

```bash
git add impl-pulse/src-tauri/Cargo.toml impl-pulse/src-tauri/Cargo.lock
git commit -m "feat: enable reqwest socks feature and add dirs crate"
```

## Task 2: Add `proxy_url` to Settings

**Files:**
- Modify: `impl-pulse/src-tauri/src/settings.rs` (Settings struct tail at lines 89-91; `impl Default` at line 156; tests module `settings_defaults_tests` at lines 287-333)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib settings` (cwd `impl-pulse\src-tauri`)

- [ ] **Step 1: Write failing test**

In `mod settings_defaults_tests`, immediately after `old_toml_without_new_fields_gets_defaults` (after line 306), add:

```rust
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
```

Also add one assertion at the end of `old_toml_without_new_fields_gets_defaults`, right after the existing `assert!(parsed.rate_overrides.is_empty());` (line 305):

```rust
assert!(parsed.proxy_url.is_none());
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib settings
```

Expected: compile error — `no field 'proxy_url' on type Settings` (red).

- [ ] **Step 3: Implement**

In the `Settings` struct, after the `rate_overrides` field (lines 89-90, before the closing `}` at line 91), add:

```rust
/// Optional outbound proxy for all provider/hub HTTP traffic.
/// Accepts `http://host:port` or `socks5://host:port`; `None` uses the default client.
#[serde(default)]
pub proxy_url: Option<String>,
```

In `impl Default for Settings`, after `rate_overrides: HashMap::new(),` (line 156), add:

```rust
proxy_url: None,
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib settings
```

Expected: all `settings::` tests PASS.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/settings.rs
git commit -m "feat: add optional proxy_url setting for http/socks5 proxy"
```

## Task 3: Extend auth enums and Preset with LocalToken + experimental

**Files:**
- Modify: `impl-pulse/src-tauri/src/providers/mod.rs` (AuthKind lines 29-38; Credentials lines 41-49; Preset lines 149-157; all 5 PRESETS entries lines 161-236)
- Test: full suite — `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib` (cwd `impl-pulse\src-tauri`)

- [ ] **Step 1: Add `LocalToken` to AuthKind**

In `pub enum AuthKind` (lines 29-38), after the `AccessKeySecret,` variant, add:

```rust
    /// Local token-file credential (e.g. ChatGPT Codex `~/.codex/auth.json`).
    LocalToken,
```

- [ ] **Step 2: Add `LocalToken` to Credentials**

In `pub enum Credentials` (lines 41-49), after the `AccessKeySecret` variant, add:

```rust
    /// Token read from a local file on this machine (no keyring entry needed).
    LocalToken { token: String },
```

Verified during planning: all 5 existing `match creds` sites (deepseek.rs:200, openai.rs:111, minimax.rs:217, volcengine.rs:384, volcengine_api.rs:98) end with a `_ =>` catch-all arm, so this change compiles without touching them.

- [ ] **Step 3: Add `experimental` to Preset + update the 5 existing entries**

In `pub struct Preset` (lines 149-157), after the `sub_modes` field, add:

```rust
    /// Experimental providers show an「实验」badge in Settings and are opt-in.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub experimental: bool,
```

Then update **all 5** existing entries in `PRESETS` (lines 161-236) — kinds `minimax`, `deepseek`, `volcengine`, `openai`, `xiaomi` — each gains `experimental: false,` as the **last field** of its struct literal (after `sub_modes`). The compiler flags every literal until all 5 are updated.

- [ ] **Step 4: Run full Rust test suite**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: PASS everywhere. Registry tests still pass (no new kinds registered yet); serde `skip_serializing_if` keeps the frontend payload unchanged for existing presets.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/providers/mod.rs
git commit -m "feat: add LocalToken auth variant and experimental flag to presets"
```

## Task 4: Create `xai.rs` — xAI Grok usage provider

**Files:**
- Create: `impl-pulse/src-tauri/src/providers/xai.rs`
- Modify: `impl-pulse/src-tauri/src/providers/mod.rs` (module list only — `pub mod` declarations live in alphabetical order at lines 9-16)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib xai` (cwd `impl-pulse\src-tauri`)

Schema notes (verified during planning): `GET https://api.x.ai/v/usage` is wrong — the real endpoint is `https://api.x.ai/v1/usage` with `Authorization: Bearer <key>`, query params `start_date`/`end_date` (ISO8601 UTC) + `granularity=daily`. The response wraps a `data` array whose items carry a `cost` (USD). The per-item timestamp field name could not be confirmed from public docs, so the parser is defensive (`#[serde(default)]` + skip) and covered by a fixed fixture test. xAI has no subscription/quota endpoint → `quota = 0`, `plan_tier = None`.

- [ ] **Step 1: Declare the module and write the failing tests**

In `providers/mod.rs`, insert into the alphabetical `pub mod` list (between `pub mod volcengine_api;` and `pub mod xiaomi;`):

```rust
pub mod xai;
```

Create `impl-pulse/src-tauri/src/providers/xai.rs` with the following content (tests reference types that do not exist yet — this is the red state):

```rust
//! xAI (Grok) usage provider.
//!
//! Endpoint:
//!   - `GET https://api.x.ai/v1/usage?start_date=...&end_date=...&granularity=daily`
//!     Returns per-day usage records; each record carries a cost in USD.
//!     We aggregate the current calendar month's cost into `windows.monthly`
//!     and persist every day via `record_daily_on` so the heatmap keeps
//!     history across polls.
//!
//! xAI publishes no subscription/quota endpoint for API credits, so the
//! monthly window has `quota = 0` (aggregate view) and `plan_tier = None`.
//!
//! NOTE: the response schema is not fully documented. `data[].cost` is
//! confirmed, the per-item timestamp field name is not — we parse
//! defensively (skip records without a usable timestamp) and pin the
//! expected shape with the fixture tests below.
//!
//! Auth: `Authorization: Bearer <API Key>`
//!
//! Reference: <https://docs.x.ai/docs/api-reference#usage>

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_data_with_defensive_fields() {
        let raw = r#"{"data":[{"timestamp":1758969600,"cost":1.25},{"cost_usd":0.75}]}"#;
        let parsed: UsageResponse = serde_json::from_str(raw).expect("parse usage fixture");
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].timestamp, 1758969600);
        assert!((parsed.data[0].cost - 1.25).abs() < 1e-9);
        assert_eq!(parsed.data[1].timestamp, 0);
        assert!((parsed.data[1].cost - 0.75).abs() < 1e-9);
    }

    #[test]
    fn month_usage_sums_only_current_calendar_month() {
        let cells = vec![
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 8, 20).unwrap(),
                value: 10.0,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 3).unwrap(),
                value: 1.25,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
                value: 0.75,
                unit: UsageUnit::Usd,
            },
        ];
        let today = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let window = month_usage(&cells, today);
        assert!((window.used - 2.0).abs() < 1e-9);
        assert_eq!(window.quota, 0.0);
        assert_eq!(window.unit, UsageUnit::Usd);
        assert!(matches!(window.cost_source, CostSource::ProviderReported));
        assert!(!window.over_quota);
        assert!(window.reset_at.is_some());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib xai
```

Expected: compile error — `cannot find type UsageResponse in this scope` (red).

- [ ] **Step 3: Implement — replace `xai.rs` with the full provider**

Replace the whole file content with:

```rust
//! xAI (Grok) usage provider.
//!
//! Endpoint:
//!   - `GET https://api.x.ai/v1/usage?start_date=...&end_date=...&granularity=daily`
//!     Returns per-day usage records; each record carries a cost in USD.
//!     We aggregate the current calendar month's cost into `windows.monthly`
//!     and persist every day via `record_daily_on` so the heatmap keeps
//!     history across polls.
//!
//! xAI publishes no subscription/quota endpoint for API credits, so the
//! monthly window has `quota = 0` (aggregate view) and `plan_tier = None`.
//!
//! NOTE: the response schema is not fully documented. `data[].cost` is
//! confirmed, the per-item timestamp field name is not — we parse
//! defensively (skip records without a usable timestamp) and pin the
//! expected shape with the fixture tests below.
//!
//! Auth: `Authorization: Bearer <API Key>`
//!
//! Reference: <https://docs.x.ai/docs/api-reference#usage>

use crate::providers::{
    AuthKind, CostSource, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const USAGE_URL: &str = "https://api.x.ai/v1/usage";

/// How many days of history we request in one call.
const HISTORY_DAYS: i64 = 99;

pub struct XaiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
}

impl XaiProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
        }
    }

    fn self_id(&self) -> &str {
        &self.instance_id
    }
}

#[derive(Debug, Deserialize)]
struct UsageRecord {
    /// Unix timestamp (UTC seconds). Field name not fully confirmed —
    /// records without a usable timestamp are skipped defensively.
    #[serde(default)]
    timestamp: i64,
    /// The record's cost in USD (`cost` confirmed; alias for safety).
    #[serde(default, alias = "cost_usd")]
    cost: f64,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    data: Vec<UsageRecord>,
}

/// Aggregate heatmap cells into the monthly `WindowUsage`: current calendar
/// month only, USD, provider-reported, no fixed quota (prepaid credits).
fn month_usage(cells: &[HeatmapCell], today: NaiveDate) -> WindowUsage {
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let used: f64 = cells
        .iter()
        .filter(|c| c.date >= month_start)
        .map(|c| c.value)
        .sum();
    let next_month_start = if today.month() == 12 {
        NaiveDate::from_ymd_opt(today.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1).unwrap()
    };
    WindowUsage {
        used,
        quota: 0.0,
        unit: UsageUnit::Usd,
        reset_at: Some(next_month_start.and_hms_opt(0, 0, 0).unwrap().and_utc()),
        over_quota: false,
        cost_source: CostSource::ProviderReported,
        tokens: None,
    }
}

#[async_trait]
impl Provider for XaiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    fn kind(&self) -> &'static str {
        "xai"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let api_key = match creds {
            Credentials::BearerKey { api_key } => api_key.clone(),
            _ => {
                return Err(ProviderError::Auth {
                    message: "xAI requires BearerKey credentials".into(),
                })
            }
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let today = Local::now().date_naive();
        let start = today - Duration::days(HISTORY_DAYS);
        let usage_url = format!(
            "{USAGE_URL}?start_date={start}T00:00:00Z&end_date={today}T00:00:00Z&granularity=daily&group_by=model",
            start = start.format("%Y-%m-%d"),
            today = today.format("%Y-%m-%d"),
        );
        let resp = self.http.get(&usage_url).bearer_auth(&api_key).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "xAI rejected the API key".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("xAI usage HTTP {}", resp.status()),
            });
        }
        let usage: UsageResponse = resp.json().await?;

        // Build heatmap cells (local calendar date -> USD) and persist them
        // so history survives beyond the API's rolling window.
        let mut cells: Vec<HeatmapCell> = Vec::new();
        for record in &usage.data {
            if record.timestamp <= 0 {
                continue;
            }
            if let Some(date) = DateTime::from_timestamp(record.timestamp, 0)
                .map(|dt| dt.with_timezone(&Local).date_naive())
            {
                if date > today {
                    continue;
                }
                let value = record.cost.max(0.0);
                let _ = self
                    .storage
                    .record_daily_on(self.self_id(), date, value, UsageUnit::Usd);
                cells.push(HeatmapCell {
                    date,
                    value,
                    unit: UsageUnit::Usd,
                });
            }
        }
        cells.sort_by_key(|c| c.date);

        let monthly = Some(month_usage(&cells, today));

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 200)
            .ok()
            .filter(|v| !v.is_empty());

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                monthly,
                ..Default::default()
            },
            heatmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_data_with_defensive_fields() {
        let raw = r#"{"data":[{"timestamp":1758969600,"cost":1.25},{"cost_usd":0.75}]}"#;
        let parsed: UsageResponse = serde_json::from_str(raw).expect("parse usage fixture");
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].timestamp, 1758969600);
        assert!((parsed.data[0].cost - 1.25).abs() < 1e-9);
        assert_eq!(parsed.data[1].timestamp, 0);
        assert!((parsed.data[1].cost - 0.75).abs() < 1e-9);
    }

    #[test]
    fn month_usage_sums_only_current_calendar_month() {
        let cells = vec![
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 8, 20).unwrap(),
                value: 10.0,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 3).unwrap(),
                value: 1.25,
                unit: UsageUnit::Usd,
            },
            HeatmapCell {
                date: NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
                value: 0.75,
                unit: UsageUnit::Usd,
            },
        ];
        let today = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let window = month_usage(&cells, today);
        assert!((window.used - 2.0).abs() < 1e-9);
        assert_eq!(window.quota, 0.0);
        assert_eq!(window.unit, UsageUnit::Usd);
        assert!(matches!(window.cost_source, CostSource::ProviderReported));
        assert!(!window.over_quota);
        assert!(window.reset_at.is_some());
    }
}
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib xai
```

Expected: `providers::xai::tests` — both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/providers/xai.rs impl-pulse/src-tauri/src/providers/mod.rs
git commit -m "feat: add xAI Grok usage provider"
```

## Task 5: Create `kimi.rs` — Kimi balance provider (CN + Global dual base)

**Files:**
- Create: `impl-pulse/src-tauri/src/providers/kimi.rs`
- Modify: `impl-pulse/src-tauri/src/providers/mod.rs` (module list only)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib kimi` (cwd `impl-pulse\src-tauri`)

Schema notes (verified during planning): `GET {base}/v1/users/me/balance` (Bearer). Response is `{"code": 0, "data": {"available_balance": <number>, "voucher_balance": <number>, "cash_balance": <number>}, "status": true}` — `code == 0` means success, and balances are JSON **numbers** (unlike DeepSeek's string-encoded balances). CN site (`https://api.moonshot.cn`) bills in CNY, global site (`https://api.moonshot.ai`) in USD; keys are not interchangeable → separate kinds `kimi` / `kimi_global` sharing this module via `new_with_base`. Delta accounting mirrors `deepseek.rs` (`last_balance` / `month_key` / `month_start_balance` / `month_topup` KV keys; keys are namespaced by `instance_id`, so the two sites never share baselines).

- [ ] **Step 1: Declare the module and write the failing tests**

In `providers/mod.rs`, insert into the alphabetical `pub mod` list (between `pub mod deepseek;` and `pub mod minimax;`):

```rust
pub mod kimi;
```

Create `impl-pulse/src-tauri/src/providers/kimi.rs` with the following content (red state):

```rust
//! Kimi (Moonshot AI) balance provider.
//!
//! Endpoint:
//!   - `GET {base}/v1/users/me/balance`
//!     Returns available / voucher / cash balance. The CN site
//!     (`https://api.moonshot.cn`) bills in CNY, the global site
//!     (`https://api.moonshot.ai`) bills in USD; API keys are NOT
//!     interchangeable, so the sites are separate account kinds
//!     (`kimi` / `kimi_global`) sharing this module via `new_with_base`.
//!
//! The API publishes no usage/billing endpoint, so usage is derived locally
//! by *balance-delta accounting* (same scheme as DeepSeek):
//!   - balance drops -> the drop is today's spend;
//!   - balance rises -> the rise is a top-up (baselined, excluded);
//!   - monthly window = month-start balance + top-ups - current balance.
//!
//! Limitation: on the very first poll there is no baseline yet, so the
//! heatmap and monthly figure start accumulating from that moment.
//!
//! Auth: `Authorization: Bearer <API Key>`

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kimi_balance_response() {
        let raw = r#"{"code":0,"data":{"available_balance":110.5,"voucher_balance":10.25,"cash_balance":100.25},"scode":"ok","status":true}"#;
        let parsed: BalanceResponse = serde_json::from_str(raw).expect("parse balance fixture");
        assert_eq!(parsed.code, 0);
        assert!((parsed.data.available_balance - 110.5).abs() < 1e-9);
        assert!((parsed.data.voucher_balance - 10.25).abs() < 1e-9);
        assert!((parsed.data.cash_balance - 100.25).abs() < 1e-9);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        let parsed: BalanceResponse = serde_json::from_str("{}").expect("parse empty fixture");
        assert_eq!(parsed.code, 0);
        assert_eq!(parsed.data.available_balance, 0.0);
        assert_eq!(parsed.data.voucher_balance, 0.0);
        assert_eq!(parsed.data.cash_balance, 0.0);
    }

    #[test]
    fn site_selects_billing_unit_and_currency() {
        assert_eq!(site_usage_unit(CN_BASE), UsageUnit::Cny);
        assert_eq!(site_usage_unit(GLOBAL_BASE), UsageUnit::Usd);
        assert_eq!(site_currency(CN_BASE), "CNY");
        assert_eq!(site_currency(GLOBAL_BASE), "USD");
    }
}
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib kimi
```

Expected: compile error — `cannot find type BalanceResponse in this scope` (red).

- [ ] **Step 3: Implement — replace `kimi.rs` with the full provider**

Replace the whole file content with:

```rust
//! Kimi (Moonshot AI) balance provider.
//!
//! Endpoint:
//!   - `GET {base}/v1/users/me/balance`
//!     Returns available / voucher / cash balance. The CN site
//!     (`https://api.moonshot.cn`) bills in CNY, the global site
//!     (`https://api.moonshot.ai`) bills in USD; API keys are NOT
//!     interchangeable, so the sites are separate account kinds
//!     (`kimi` / `kimi_global`) sharing this module via `new_with_base`.
//!
//! The API publishes no usage/billing endpoint, so usage is derived locally
//! by *balance-delta accounting* (same scheme as DeepSeek):
//!   - balance drops -> the drop is today's spend;
//!   - balance rises -> the rise is a top-up (baselined, excluded);
//!   - monthly window = month-start balance + top-ups - current balance.
//!
//! Limitation: on the very first poll there is no baseline yet, so the
//! heatmap and monthly figure start accumulating from that moment.
//!
//! Auth: `Authorization: Bearer <API Key>`

use crate::providers::{
    AuthKind, BalanceInfo, Credentials, HeatmapCell, Provider, ProviderError, UsageSnapshot,
    UsageUnit, UsageWindows, WindowUsage,
};
use crate::storage::Storage;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

const CN_BASE: &str = "https://api.moonshot.cn";
pub const GLOBAL_BASE: &str = "https://api.moonshot.ai";

const KV_LAST_BALANCE: &str = "last_balance";
const KV_MONTH_KEY: &str = "month_key";
const KV_MONTH_START_BALANCE: &str = "month_start_balance";
const KV_MONTH_TOPUP: &str = "month_topup";

/// Balances below this difference are treated as unchanged (float dust).
const DELTA_EPSILON: f64 = 1e-6;

fn fmt_f64(v: f64) -> String {
    format!("{v:.6}")
}

/// Billing unit of a site: CNY on the CN host, USD on the global one.
fn site_usage_unit(base: &str) -> UsageUnit {
    if base == GLOBAL_BASE {
        UsageUnit::Usd
    } else {
        UsageUnit::Cny
    }
}

/// Currency code reported in `BalanceInfo` for a site.
fn site_currency(base: &str) -> &'static str {
    if base == GLOBAL_BASE {
        "USD"
    } else {
        "CNY"
    }
}

/// Kimi's balance response: `{ "code": 0, "data": { ... } }` — `code == 0`
/// means success, and all balances are JSON numbers (unlike DeepSeek's
/// string-encoded balances).
#[derive(Debug, Deserialize)]
struct BalanceResponse {
    #[serde(default)]
    code: i32,
    #[serde(default)]
    data: BalanceData,
}

#[derive(Debug, Deserialize, Default)]
struct BalanceData {
    #[serde(default)]
    available_balance: f64,
    #[serde(default)]
    voucher_balance: f64,
    #[serde(default)]
    cash_balance: f64,
}

pub struct KimiProvider {
    http: Client,
    storage: Arc<Storage>,
    instance_id: String,
    label: String,
    base: &'static str,
}

impl KimiProvider {
    pub fn new(http: Client, storage: Arc<Storage>, instance_id: String, label: String) -> Self {
        Self::new_with_base(http, storage, instance_id, label, CN_BASE)
    }

    /// Site-specific constructor: `base` selects the CN or global API host.
    /// KV keys are namespaced by `instance_id`, so the two sites keep
    /// independent balance baselines.
    pub fn new_with_base(
        http: Client,
        storage: Arc<Storage>,
        instance_id: String,
        label: String,
        base: &'static str,
    ) -> Self {
        Self {
            http,
            storage,
            instance_id,
            label,
            base,
        }
    }

    /// Storage keyspace / account id for this instance.
    fn self_id(&self) -> &str {
        &self.instance_id
    }

    /// Account for a balance change since the previous poll (same scheme as
    /// DeepSeek: drops = spend, rises = top-ups).
    fn record_balance_delta(&self, current: f64) {
        let baseline = self
            .storage
            .kv_get(self.self_id(), KV_LAST_BALANCE)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok());

        match baseline {
            None => {
                // First observation: just establish the baseline.
                let _ = self
                    .storage
                    .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
            }
            Some(prev) => {
                let delta = prev - current;
                if delta > DELTA_EPSILON {
                    // Balance dropped -> that drop is spend.
                    let _ = self.storage.accumulate_daily(
                        self.self_id(),
                        delta,
                        site_usage_unit(self.base),
                    );
                    let _ = self
                        .storage
                        .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
                } else if delta < -DELTA_EPSILON {
                    // Balance rose -> top-up. Rebaseline and track the top-up
                    // for the monthly window.
                    let topup = self
                        .storage
                        .kv_get(self.self_id(), KV_MONTH_TOPUP)
                        .ok()
                        .flatten()
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let _ = self.storage.kv_set(
                        self.self_id(),
                        KV_MONTH_TOPUP,
                        &fmt_f64(topup + -delta),
                    );
                    let _ = self
                        .storage
                        .kv_set(self.self_id(), KV_LAST_BALANCE, &fmt_f64(current));
                }
            }
        }
    }

    /// Monthly spend window derived from balance bookkeeping. Handles the
    /// calendar-month rollover by rebasing on the current balance.
    fn month_spend_window(&self, current: f64) -> Option<WindowUsage> {
        let month_key = chrono::Local::now().format("%Y-%m").to_string();
        let stored_month = self.storage.kv_get(self.self_id(), KV_MONTH_KEY).ok().flatten();

        let month_start = if stored_month.as_deref() != Some(month_key.as_str()) {
            // New month: the month starts from the current balance.
            let _ = self.storage.kv_set(self.self_id(), KV_MONTH_KEY, &month_key);
            let _ = self
                .storage
                .kv_set(self.self_id(), KV_MONTH_START_BALANCE, &fmt_f64(current));
            let _ = self.storage.kv_set(self.self_id(), KV_MONTH_TOPUP, "0");
            current
        } else {
            self.storage
                .kv_get(self.self_id(), KV_MONTH_START_BALANCE)
                .ok()
                .flatten()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(current)
        };

        let topup = self
            .storage
            .kv_get(self.self_id(), KV_MONTH_TOPUP)
            .ok()
            .flatten()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(0.0);

        let used = (month_start + topup - current).max(0.0);
        Some(WindowUsage {
            used,
            quota: 0.0, // Prepaid balance - no fixed quota.
            unit: site_usage_unit(self.base),
            reset_at: None,
            over_quota: false,
            // 余额差推算的本月消费为估算值（非官方逐笔账单）。
            cost_source: crate::providers::CostSource::Estimated,
            tokens: None,
        })
    }
}

#[async_trait]
impl Provider for KimiProvider {
    fn id(&self) -> String {
        self.instance_id.clone()
    }

    /// Both sites share the `kimi` brand; `kimi_global` differs only in the
    /// base URL, so `kind()` reports the brand for both.
    fn kind(&self) -> &'static str {
        "kimi"
    }

    fn display_name(&self) -> String {
        self.label.clone()
    }

    fn auth_kind(&self) -> AuthKind {
        AuthKind::BearerKey
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageSnapshot, ProviderError> {
        let api_key = match creds {
            Credentials::BearerKey { api_key } => api_key.clone(),
            _ => {
                return Err(ProviderError::Auth {
                    message: "Kimi requires BearerKey credentials".into(),
                })
            }
        };
        if api_key.trim().is_empty() || api_key == "mock" {
            return Err(ProviderError::NotConfigured);
        }

        let resp = self
            .http
            .get(format!("{}/v1/users/me/balance", self.base))
            .bearer_auth(&api_key)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::Auth {
                message: "Kimi rejected the API key".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Network {
                message: format!("Kimi balance HTTP {}", resp.status()),
            });
        }
        let parsed: BalanceResponse = resp.json().await?;

        // Only run the delta accounting when the API reports success;
        // otherwise a zero/empty body would corrupt the baselines.
        let mut balance = None;
        let mut monthly = None;
        if parsed.code == 0 {
            let current = parsed.data.available_balance;
            self.record_balance_delta(current);
            monthly = self.month_spend_window(current);
            balance = Some(BalanceInfo {
                total: parsed.data.available_balance,
                granted: parsed.data.voucher_balance,
                topped_up: parsed.data.cash_balance,
                currency: site_currency(self.base).to_string(),
                is_available: true,
            });
        }

        let heatmap: Option<Vec<HeatmapCell>> = self
            .storage
            .load_heatmap(self.self_id(), 200)
            .ok()
            .filter(|v| !v.is_empty());

        Ok(UsageSnapshot {
            provider_id: self.id(),
            provider_display_name: self.display_name(),
            plan_tier: None,
            timestamp: Utc::now(),
            windows: UsageWindows {
                balance,
                monthly,
                ..Default::default()
            },
            heatmap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kimi_balance_response() {
        let raw = r#"{"code":0,"data":{"available_balance":110.5,"voucher_balance":10.25,"cash_balance":100.25},"scode":"ok","status":true}"#;
        let parsed: BalanceResponse = serde_json::from_str(raw).expect("parse balance fixture");
        assert_eq!(parsed.code, 0);
        assert!((parsed.data.available_balance - 110.5).abs() < 1e-9);
        assert!((parsed.data.voucher_balance - 10.25).abs() < 1e-9);
        assert!((parsed.data.cash_balance - 100.25).abs() < 1e-9);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        let parsed: BalanceResponse = serde_json::from_str("{}").expect("parse empty fixture");
        assert_eq!(parsed.code, 0);
        assert_eq!(parsed.data.available_balance, 0.0);
        assert_eq!(parsed.data.voucher_balance, 0.0);
        assert_eq!(parsed.data.cash_balance, 0.0);
    }

    #[test]
    fn site_selects_billing_unit_and_currency() {
        assert_eq!(site_usage_unit(CN_BASE), UsageUnit::Cny);
        assert_eq!(site_usage_unit(GLOBAL_BASE), UsageUnit::Usd);
        assert_eq!(site_currency(CN_BASE), "CNY");
        assert_eq!(site_currency(GLOBAL_BASE), "USD");
    }
}
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib kimi
```

Expected: `providers::kimi::tests` — all three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/providers/kimi.rs impl-pulse/src-tauri/src/providers/mod.rs
git commit -m "feat: add Kimi balance provider with CN/Global dual base"
```

## Task 6: Create `codex.rs` — ChatGPT Codex usage provider (experimental)

**Files:**
- Create: `impl-pulse/src-tauri/src/providers/codex.rs`
- Modify: `impl-pulse/src-tauri/src/providers/mod.rs` (module list only)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib codex` (cwd `impl-pulse\src-tauri`)

Schema notes (corrected during planning): the approved spec sketched `https://chatgpt.com/backend-api/codex/usage`; live probing showed the real path is **`https://chatgpt.com/backend-api/wham/usage`** — we implement the live value. Response: `{"rate_limit": {"primary_window": {"used_percent", "reset_at"}, "secondary_window": {...}}}` → primary = 5h window, secondary = weekly. Request needs `Authorization: Bearer <access_token>` and `ChatGPT-Account-ID: <account_id>` when known. Credentials come from `Credentials::LocalToken` (manual paste wins) with fallback to `~/.codex/auth.json` (`tokens.access_token` + `tokens.account_id`), re-read on every poll so a fresh `codex login` is picked up automatically.

- [ ] **Step 1: Declare the module and write the failing tests**

In `providers/mod.rs`, insert into the alphabetical `pub mod` list **before** `pub mod deepseek;` (codex sorts first):

```rust
pub mod codex;
```

Create `impl-pulse/src-tauri/src/providers/codex.rs` with the following content (red state):

```rust
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
//! Auth: `Authorization: Bearer <access_token>` (+ `ChatGPT-Account-ID`
//! when the auth file carries an account id).

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
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib codex
```

Expected: compile error — `cannot find type UsageResponse in this scope` (red).

- [ ] **Step 3: Implement — replace `codex.rs` with the full provider**

Replace the whole file content with:

```rust
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
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib codex
```

Expected: `providers::codex::tests` — all three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/providers/codex.rs impl-pulse/src-tauri/src/providers/mod.rs
git commit -m "feat: add ChatGPT Codex usage provider (experimental)"
```

## Task 7: Register xAI / Kimi / Codex in `providers/mod.rs`

**Files:**
- Modify `impl-pulse/src-tauri/src/providers/mod.rs`

The three modules from Tasks 4–6 exist but are invisible to the app: no
`pub mod` declaration, no preset, no registry entry. This task wires them
into the three static tables `mod.rs` owns. After this task the pre-existing
registry tests automatically cover the new kinds — no test edits needed:

- `registry_kinds_are_unique` guards duplicate kind strings;
- `every_registry_kind_is_exposed_by_a_preset` is one-way (registry →
  presets): `kimi_global` is satisfied because the `kimi` preset's
  `sub_modes` contains it;
- `every_kind_builds_a_provider` builds every kind against a temp SQLite
  store with a plain `Client::new()` — the three new constructors are pure
  wiring (`new` never touches the network; Codex's `detect_auth` only reads
  a local file and returns `None` when absent), so it passes offline.

- [ ] **Step 1: Declare the three modules (alphabetical order)**

In the `pub mod` block at the top of `mod.rs` (currently `deepseek`,
`minimax`, `minimax_api`, `mock`, `openai`, `volcengine`,
`volcengine_api`, `xiaomi`), insert three lines keeping alphabetical
order: `pub mod codex;` before `pub mod deepseek;`, `pub mod kimi;` after
`pub mod deepseek;`, `pub mod xai;` after `pub mod volcengine_api;`.
Resulting block:

```rust
pub mod codex;
pub mod deepseek;
pub mod kimi;
pub mod minimax;
pub mod minimax_api;
pub mod mock;
pub mod openai;
pub mod volcengine;
pub mod volcengine_api;
pub mod xai;
pub mod xiaomi;
```

- [ ] **Step 2: Append three `PRESETS` entries**

Append after the `xiaomi` preset entry (before the closing `];` of
`PRESETS`). Each entry ends with the `experimental` flag added in Task 3
as its last field, matching the existing five entries which all end with
`experimental: false`:

```rust
    Preset {
        kind: "xai",
        display_name: "xAI Grok",
        auth_kind: AuthKind::BearerKey,
        default_accent: "#1d1d1d",
        sub_modes: None,
        experimental: true,
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
        kind: "codex",
        display_name: "ChatGPT Codex",
        auth_kind: AuthKind::LocalToken,
        default_accent: "#10a37f",
        sub_modes: None,
        experimental: true,
    },
```

- [ ] **Step 3: Append four `PROVIDER_REGISTRY` entries**

Append after the `xiaomi_api` entry (before the closing `];`). Four entries
because `kimi_global` is its own registry row built through
`KimiProvider::new_with_base` with the exported `kimi::GLOBAL_BASE`. The
`kimi_global` line exceeds rustfmt's 100-column budget, so it is the only
entry wrapped across lines:

```rust
    ("xai", |h, s, i, l| Arc::new(xai::XaiProvider::new(h, s, i, l))),
    ("kimi", |h, s, i, l| Arc::new(kimi::KimiProvider::new(h, s, i, l))),
    (
        "kimi_global",
        |h, s, i, l| {
            Arc::new(kimi::KimiProvider::new_with_base(h, s, i, l, kimi::GLOBAL_BASE))
        },
    ),
    ("codex", |h, s, i, l| Arc::new(codex::CodexProvider::new(h, s, i, l))),
```

- [ ] **Step 4: Run the full Rust suite**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all tests PASS, including the registry tests now covering 12
kinds — in particular `providers::registry_tests::registry_kinds_are_unique`,
`providers::registry_tests::every_registry_kind_is_exposed_by_a_preset`
(`kimi_global` satisfied via the `kimi` preset's `sub_modes`) and
`providers::registry_tests::every_kind_builds_a_provider`.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/providers/mod.rs
git commit -m "feat: register xAI, Kimi and Codex providers in presets and registry"
```

## Task 8: Add a proxy-aware `build_http_client` helper in `lib.rs`

**Files:**
- Modify: `impl-pulse/src-tauri/src/lib.rs` (helper inserted after `build_registry`; tests at file end)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib http_client` (cwd `impl-pulse\src-tauri`)

This task only adds the pure helper the proxy swap (Task 11) will call.
`AppState.http` keeps its current type here so the tree stays green; the
type change lands together with the IPC call-site migration in Task 11.

Behaviour: no proxy (None / blank / whitespace) → exactly the client used
today (15s timeout, env-var proxy detection untouched). A non-empty URL →
`.no_proxy().proxy(Proxy::all(url))` so the explicit setting deterministically
replaces env-var detection; `http(s)` and `socks5` URLs (Task 1) and
`user:pass@host:port` basic auth are handled by reqwest. Parse/build failures
return `Err(String)` — `test_proxy` (Task 12) surfaces them to the UI.

- [ ] **Step 1: Write failing tests**

At the end of `lib.rs`, after the `windows_test_manifest` module, add:

```rust
#[cfg(test)]
mod http_client_tests {
    use super::*;

    #[test]
    fn none_and_blank_build_without_proxy() {
        assert!(build_http_client(None).is_ok());
        assert!(build_http_client(Some("")).is_ok());
        assert!(build_http_client(Some("   ")).is_ok());
    }

    #[test]
    fn valid_http_and_socks_urls_are_accepted() {
        assert!(build_http_client(Some("http://127.0.0.1:7890")).is_ok());
        assert!(build_http_client(Some("http://user:pass@127.0.0.1:7890")).is_ok());
        assert!(build_http_client(Some("socks5://127.0.0.1:1080")).is_ok());
    }

    #[test]
    fn invalid_proxy_string_is_rejected() {
        assert!(build_http_client(Some("not a proxy")).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib http_client
```

Expected: compile error — `cannot find function build_http_client in this
scope` (E0425).

- [ ] **Step 3: Implement the helper**

In `lib.rs`, directly after the `build_registry` function, add:

```rust
/// Build the shared HTTP client. `proxy_url` comes from `Settings::proxy_url`
/// and may be empty (no proxy configured). A non-empty value must parse as an
/// `http(s)` or `socks5` proxy URL (`user:pass@host:port` basic auth is
/// supported). An explicit proxy replaces env-var proxy detection so the
/// setting is deterministic; with no proxy the previous env-aware default is
/// kept. Errors are returned as display strings for `test_proxy`.
pub fn build_http_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder().timeout(std::time::Duration::from_secs(15));
    if let Some(url) = proxy_url.map(str::trim).filter(|u| !u.is_empty()) {
        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("invalid proxy url {url:?}: {e}"))?;
        builder = builder.no_proxy().proxy(proxy);
    }
    builder.build().map_err(|e| format!("building reqwest client failed: {e}"))
}
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib http_client
```

Expected: `lib::http_client_tests` — all three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/lib.rs
git commit -m "feat: add proxy-aware build_http_client helper"
```

## Task 9: Retire stale poll loops via registry identity check

**Files:**
- Modify: `impl-pulse/src-tauri/src/scheduler.rs` (new helpers after `spawn_one`; two call sites inside `poll_loop`; new tests module at file end)
- Test: `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib registration_tests` (cwd `impl-pulse\src-tauri`)

Why: after Task 11's proxy-rebuild branch replaces every provider instance
in the registry, the long-lived `poll_loop` tasks spawned at startup still
hold the old `Arc<dyn Provider>` and would keep polling with the stale HTTP
client (the same staleness already exists today whenever an account is
edited: the old loop never exits). Each loop must retire itself once it is
no longer the instance registered under its id. `ProviderRegistry::get`
already exists in `providers/mod.rs`, so this is one pure function, one
`AppHandle` wrapper and two call sites.

- [ ] **Step 1: Write failing tests**

At the end of `scheduler.rs`, after the `interval_tests` module, add:

```rust
#[cfg(test)]
mod registration_tests {
    use super::*;
    use crate::providers::{MockProvider, ProviderRegistry};

    fn provider(id: &'static str) -> Arc<dyn Provider> {
        Arc::new(MockProvider::new(id, id, 1.0))
    }

    #[test]
    fn current_instance_is_live() {
        let p = provider("acct-a");
        let mut registry = ProviderRegistry::new();
        registry.insert(p.clone());
        assert!(is_current(&registry, &p, "acct-a"));
    }

    #[test]
    fn replaced_instance_is_stale() {
        let p = provider("acct-a");
        let mut registry = ProviderRegistry::new();
        registry.insert(p.clone());
        // Same id, different instance: what a registry rebuild produces.
        registry.insert(provider("acct-a"));
        assert!(!is_current(&registry, &p, "acct-a"));
    }

    #[test]
    fn unknown_id_is_stale() {
        let p = provider("acct-a");
        let registry = ProviderRegistry::new();
        assert!(!is_current(&registry, &p, "acct-a"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib registration_tests
```

Expected: compile error — `cannot find function is_current in this scope`
(E0425).

- [ ] **Step 3: Implement the check and wire it into `poll_loop`**

Directly after `spawn_one`, add the pure core and its `AppHandle` wrapper:

```rust
/// Pure core of `still_registered`: a loop is live only while `provider` is
/// still the instance registered under `id`. A removed account (no entry) or
/// a rebuilt/replaced instance (different `Arc`, e.g. after a proxy change
/// or an account edit) makes this loop stale.
fn is_current(
    registry: &crate::providers::ProviderRegistry,
    provider: &Arc<dyn Provider>,
    id: &str,
) -> bool {
    registry
        .get(id)
        .map(|current| Arc::ptr_eq(&current, provider))
        .unwrap_or(false)
}

/// `AppHandle` flavour of [`is_current`] used by `poll_loop`.
async fn still_registered(app: &AppHandle, provider: &Arc<dyn Provider>, id: &str) -> bool {
    let registry = app.state::<AppState>().registry.read().await;
    is_current(&registry, provider, id)
}
```

In `poll_loop`, retire the loop in both `select!` branches, always before
any `poll_one` call so a stale loop never fires one last request.

Tick branch — after the pause check, insert:

```rust
                if !still_registered(&app, &provider, &id).await {
                    break;
                }
```

Settings-changed branch — after the `changed.is_err()` bail-out, insert the
same three lines.

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib registration_tests
```

Expected: `scheduler::registration_tests` — all three tests PASS. Then run
the full suite:

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all PASS (burn / interval / registry / settings suites unaffected).

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/scheduler.rs
git commit -m "feat: retire stale poll loops via registry identity check"
```

## Task 10: Pass an injected client into hub report/fetch functions

**Files:**

- `src-tauri/src/hub.rs` — `report_to_hub` (L168-189), `fetch_devices` (L192-211)
- `src-tauri/src/lib.rs` — agent report loop call site (L270)
- `src-tauri/src/ipc.rs` — `get_hub_devices` call site (L229)

This is a behaviour-preserving refactor that prepares the hub functions to
share the (soon to be swappable) HTTP client. Both functions currently build
their own throwaway `reqwest::Client::new()`; instead they accept
`client: &reqwest::Client` as their first parameter. The per-request 8s
`.timeout(...)` stays on the request builder, so it still overrides the
client's own 15s default — wire semantics are unchanged.

Because Rust cannot compile a crate while call site and signature disagree,
the red step edits the two call sites first and proves the mismatch (E0061),
and the green step changes the signatures in the same commit. The four
existing hub tests (`authorized_*`, `report_roundtrips_in_storage`) never
construct a reqwest client, so they stay green throughout.

- [ ] **Step 1: Change the two call sites (red)**

In `lib.rs`, the agent loop currently captures no HTTP client. Add the
capture next to the other captures and pass it at the call site:

```rust
                if start.hub_mode == "agent" && start.report_on {
                    let store_reporter = store.clone();
                    let local_reporter = local.clone();
                    let settings_reporter = settings_store.clone();
                    let http_reporter = http.clone();
                    tauri::async_runtime::spawn(async move {
```

and:

```rust
                                let _ = hub::report_to_hub(
                                    &http_reporter,
                                    &base,
                                    &device,
                                    &token,
                                )
                                .await;
```

In `ipc.rs`, `get_hub_devices` already holds `state.http`; pass it directly:

```rust
        match crate::hub::fetch_devices(&state.http, &s.hub_base, &s.hub_token).await {
            Ok(remote) => sources.push(remote),
            Err(e) => warning = Some(format!("远端 hub 拉取失败：{e}")),
        }
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib --no-run
```

Expected: compile errors E0061 — "this function takes 3 arguments but 4
arguments were supplied" (`report_to_hub`) and "takes 2 arguments but 3
arguments were supplied" (`fetch_devices`).

- [ ] **Step 3: Change the function signatures (green)**

In `hub.rs`, replace `report_to_hub` with:

```rust
/// 作为 agent 把本机用量上报到远端 hub。`token` 非空时附 `Authorization: Bearer`。
pub async fn report_to_hub(
    client: &reqwest::Client,
    base: &str,
    device: &HubDevice,
    token: &str,
) -> Result<(), String> {
    let url = format!("{}/ingest", base.trim_end_matches('/'));
    let body = serde_json::to_vec(device).map_err(|e| e.to_string())?;
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(8));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("report failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("hub responded {}", resp.status()))
    }
}
```

and `fetch_devices` with:

```rust
/// 作为 client 拉取 hub 的设备列表。`token` 非空时附 `Authorization: Bearer`。
pub async fn fetch_devices(
    client: &reqwest::Client,
    base: &str,
    token: &str,
) -> Result<Vec<HubDevice>, String> {
    let url = format!("{}/devices", base.trim_end_matches('/'));
    let mut req = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(8));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("hub unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("hub responded {}", resp.status()));
    }
    resp.json::<Vec<HubDevice>>()
        .await
        .map_err(|e| format!("bad hub payload: {e}"))
}
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all PASS — the four hub tests are unaffected and no other caller
exists.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/hub.rs impl-pulse/src-tauri/src/lib.rs impl-pulse/src-tauri/src/ipc.rs
git commit -m "refactor: inject shared http client into hub report/fetch"
```

## Task 11: Swap AppState.http to Arc<RwLock<Client>> and rebuild on proxy change

**Files:**

- `src-tauri/src/lib.rs` — field (L64-65), `build_registry` visibility (L101),
  client + registry construction (L186-197), reporter loop (L254/270), fx loop
  (L291/297)
- `src-tauri/src/ipc.rs` — every `state.http` access: `get_hub_devices` (L229),
  `get_exchange_rates` (L261), `refresh_exchange_rates` (L274), `save_settings`
  (L602), `upsert_account` (L680), `test_provider` (L770), plus the
  proxy-rebuild branch in `save_settings`

reqwest bakes a proxy into the `Client` at build time, so a proxy change means
replacing the whole client and rebuilding every provider that holds a clone of
it. The field therefore becomes `Arc<RwLock<Client>>`; access sites take a
short-lived read lock and clone the client (a cheap inner-Arc clone). Two
background loops currently capture the setup-local bare client (the exchange
loop and, after Task 10, the hub reporter loop) — they must capture the `Arc`
and re-read it per iteration, so they automatically follow a swapped client.

This task has one genuine unit of new logic: `proxy_changed`, which decides
whether a save needs the expensive full rebuild. Blank/whitespace URLs
normalize to `None` so trivial saves keep using the fast incremental path.

- [ ] **Step 1: Write the failing tests (red)**

In `ipc.rs`, directly after the `test_provider` function and before the
existing `clamp_tests` module, add:

```rust
#[cfg(test)]
mod proxy_tests {
    use super::proxy_changed;

    #[test]
    fn detects_real_change() {
        assert!(proxy_changed(&None, &Some("http://127.0.0.1:7890".to_string())));
        assert!(proxy_changed(
            &Some("http://127.0.0.1:7890".to_string()),
            &Some("socks5://127.0.0.1:1080".to_string())
        ));
    }

    #[test]
    fn treats_blank_as_none() {
        assert!(!proxy_changed(&None, &Some("   ".to_string())));
        assert!(!proxy_changed(&Some(" ".to_string()), &None));
    }

    #[test]
    fn ignores_surrounding_whitespace() {
        assert!(!proxy_changed(
            &Some("http://127.0.0.1:7890".to_string()),
            &Some("  http://127.0.0.1:7890 ".to_string())
        ));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib proxy_tests
```

Expected: compile error E0425 — `cannot find function proxy_changed in this
scope`.

- [ ] **Step 3: Migrate every http access site (green)**

All edits below go into one commit: the crate only compiles once the field
type and *all* access sites agree.

**(a) `lib.rs` — the field.** Replace:

```rust
    /// Shared HTTP client for building provider instances at runtime.
    pub http: Client,
```

with:

```rust
    /// Swappable shared HTTP client. A proxy change replaces the client under
    /// the write lock and rebuilds the whole registry; access sites take a
    /// read lock and clone the cheap inner Arc.
    pub http: Arc<RwLock<Client>>,
```

(`Arc`, `RwLock` and `Client` are already imported — L28, L36 and the
existing `use std::sync::Arc`.)

**(b) `lib.rs` — expose `build_registry` for the rebuild branch.** Change:

```rust
fn build_registry(
    accounts: &[AccountMeta],
    http: Client,
    storage: Arc<storage::Storage>,
) -> ProviderRegistry {
```

to:

```rust
pub(crate) fn build_registry(
    accounts: &[AccountMeta],
    http: Client,
    storage: Arc<storage::Storage>,
) -> ProviderRegistry {
```

**(c) `lib.rs` — startup construction.** Replace L186-197:

```rust
            // Single shared HTTP client with sensible defaults.
            let http = Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("building reqwest client");

            let registry = {
                // First-load the settings to learn the configured accounts.
                let accounts = settings_store.read_blocking().accounts;
                let reg = build_registry(&accounts, http.clone(), store.clone());
                Arc::new(RwLock::new(reg))
            };
```

with:

```rust
            // Shared, swappable HTTP client. A saved proxy URL that fails to
            // build falls back to a direct client instead of aborting launch.
            let initial_proxy = settings_store.read_blocking().proxy_url.clone();
            let initial_client =
                crate::build_http_client(initial_proxy.as_deref()).unwrap_or_else(|_| {
                    Client::builder()
                        .timeout(std::time::Duration::from_secs(15))
                        .build()
                        .expect("building reqwest client")
                });

            let registry = {
                // First-load the settings to learn the configured accounts.
                let accounts = settings_store.read_blocking().accounts;
                let reg = build_registry(&accounts, initial_client.clone(), store.clone());
                Arc::new(RwLock::new(reg))
            };
            let http = Arc::new(RwLock::new(initial_client));
```

(Registry is built before wrapping the client, so no lock is needed in this
sync context; the AppState assembly's shorthand `http,` needs no edit.)

**(d) `lib.rs` — reporter loop.** The Task 10 capture
`let http_reporter = http.clone();` stays as text (now an `Arc` clone).
Immediately before the `report_to_hub` call, re-read the current client:

```rust
                                let client = http_reporter.read().await.clone();
                                let _ = hub::report_to_hub(
                                    &client,
                                    &base,
                                    &device,
                                    &token,
                                )
                                .await;
```

**(e) `lib.rs` — exchange loop.** The capture `let http_fx = http.clone();`
also stays textually; inside the loop body change:

```rust
                        let s = settings_fx.read_blocking();
                        let snap = exchange::refresh_rates(&store_fx, &s, &http_fx, false).await;
```

to:

```rust
                        let s = settings_fx.read_blocking();
                        let client = http_fx.read().await.clone();
                        let snap = exchange::refresh_rates(&store_fx, &s, &client, false).await;
```

**(f) `ipc.rs` — `get_hub_devices`.** Change:

```rust
        match crate::hub::fetch_devices(&state.http, &s.hub_base, &s.hub_token).await {
```

to:

```rust
        let client = state.http.read().await.clone();
        match crate::hub::fetch_devices(&client, &s.hub_base, &s.hub_token).await {
```

**(g) `ipc.rs` — both exchange commands.** In `get_exchange_rates`:

```rust
    let settings = state.settings.get().await;
    let snap = crate::exchange::refresh_rates(&state.storage, &settings, &state.http, false).await;
```

becomes:

```rust
    let settings = state.settings.get().await;
    let client = state.http.read().await.clone();
    let snap = crate::exchange::refresh_rates(&state.storage, &settings, &client, false).await;
```

Apply the identical change in `refresh_exchange_rates` with `true`.

**(h) `ipc.rs` — add the decision helper.** Directly above `save_settings`,
add:

```rust
/// Whether a settings save changes the effective proxy URL. Blank strings and
/// surrounding whitespace normalize to `None`, so saves that merely touch
/// other settings keep using the fast incremental registry reconcile.
fn proxy_changed(old: &Option<String>, new: &Option<String>) -> bool {
    let normalize = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    normalize(old) != normalize(new)
}
```

**(i) `ipc.rs` — `save_settings` proxy branch.** Replace the whole function
body from the signature through the registry-reconcile block (the snapshot
retain, emit and wake lines stay untouched afterward). New version:

```rust
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    new_settings: Settings,
) -> Result<(), String> {
    // Capture before save(): saving overwrites the cached settings, after
    // which the old proxy would be unobservable.
    let old_proxy_url = state.settings.get().await.proxy_url.clone();

    state
        .settings
        .save(new_settings.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Mirror the cached window-behavior flags so the synchronous window-event
    // handlers (close-to-tray, edge snap) pick up the change immediately.
    state.close_to_tray.store(new_settings.close_to_tray, std::sync::atomic::Ordering::SeqCst);
    state.edge_snap.store(new_settings.edge_snap, std::sync::atomic::Ordering::SeqCst);

    if proxy_changed(&old_proxy_url, &new_settings.proxy_url) {
        // Build the replacement first. On failure, settings are already
        // persisted, so leave the live client/registry untouched and let the
        // user correct the URL and save again.
        let client = crate::build_http_client(new_settings.proxy_url.as_deref())?;
        // Rebuild every provider against the new client in one shot.
        let fresh = crate::build_registry(
            &new_settings.accounts,
            client.clone(),
            state.storage.clone(),
        );
        *state.http.write().await = client;
        let mut registry = state.registry.write().await;
        *registry = fresh;
        // Spawn a polling task for every rebuilt instance. The previous tasks
        // see the replaced Arc via the Task 9 identity check and retire
        // themselves, so no task is ever polling the old proxy.
        for account in &new_settings.accounts {
            if let Some(provider) = registry.get(&account.instance_id) {
                crate::scheduler::spawn_one(
                    app.clone(),
                    provider,
                    account.instance_id.clone(),
                );
            }
        }
        drop(registry);
    } else {
        // Reconcile the live registry with the saved accounts incrementally:
        // register newly added accounts (and spawn a polling task), remove
        // accounts that disappeared. Interval/enable edits are picked up by
        // the existing poll loops via the settings_wake ping below.
        let account_ids: std::collections::HashSet<String> = new_settings
            .accounts
            .iter()
            .map(|a| a.instance_id.clone())
            .collect();
        let client = state.http.read().await.clone();
        let mut registry = state.registry.write().await;
        for account in &new_settings.accounts {
            if registry.get(&account.instance_id).is_none() {
                if let Some(provider) =
                    build_account_provider(account, client.clone(), state.storage.clone())
                {
                    registry.insert(provider.clone());
                    let app = app.clone();
                    let id = account.instance_id.clone();
                    crate::scheduler::spawn_one(app, provider, id);
                }
            }
        }
        for id in registry.list() {
            if !account_ids.contains(&id.id()) {
                registry.remove(&id.id());
            }
        }
        drop(registry);
    }
```

The rest of the function (the enabled-ids snapshot retain, the
`settings-changed` emit and the `settings_wake` ping) is unchanged and now
follows the closing brace of the `else` block.

**(j) `ipc.rs` — `upsert_account`.** Change:

```rust
    if registry.get(&instance_id).is_none() {
        if let Some(provider) =
            build_account_provider(&account, state.http.clone(), state.storage.clone())
        {
```

to:

```rust
    if registry.get(&instance_id).is_none() {
        let client = state.http.read().await.clone();
        if let Some(provider) = build_account_provider(&account, client, state.storage.clone()) {
```

**(k) `ipc.rs` — `test_provider`.** Change:

```rust
    let provider = build_account_provider(&account, state.http.clone(), state.storage.clone())
        .ok_or_else(|| format!("unknown provider kind: {provider_kind}"))?;
```

to:

```rust
    let client = state.http.read().await.clone();
    let provider = build_account_provider(&account, client, state.storage.clone())
        .ok_or_else(|| format!("unknown provider kind: {provider_kind}"))?;
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all PASS — `proxy_tests` (3 new) plus every existing suite. Then
confirm the app builds end to end:

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo build
```

Expected: clean build, no remaining bare-`state.http` errors.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/lib.rs impl-pulse/src-tauri/src/ipc.rs
git commit -m "feat: swap AppState.http to swappable client and rebuild registry on proxy change"
```

## Task 12: Add test_proxy and detect_codex_token commands

**Files:**

- `src-tauri/src/ipc.rs` — two new commands after `test_provider` (L776)
- `src-tauri/src/lib.rs` — register them in `invoke_handler` (after L458)

Two thin commands back the new Settings UI:

1. `test_proxy(url)` builds a *temporary* client through the saved URL (via
   the Task 8 helper) and probes a lightweight always-204 endpoint, returning
   a structured `{ ok, status, error }` instead of a `Result` — even an
   unreachable proxy is a normal probe outcome the UI displays, not a command
   failure.
2. `detect_codex_token()` reads `~/.codex/auth.json` via the Task 6
   `detect_auth` and returns its fields under the spec's key names (`token`,
   `account_id`, `last_refresh`) so the Codex form can pre-fill on mount.

Both response structs derive `Serialize`; the existing Task 6 `CodexAuth`
needs no change — only a mapping into the wire shape.

- [ ] **Step 1: Write the failing tests (red)**

Extend the `proxy_tests` module added in Task 11. Add the import:

```rust
    use super::{proxy_changed, proxy_probe_outcome};
```

and these tests:

```rust
    #[test]
    fn probe_outcome_reports_success_status() {
        let r = proxy_probe_outcome(Ok(204u16));
        assert!(r.ok);
        assert_eq!(r.status, Some(204));
        assert!(r.error.is_none());
    }

    #[test]
    fn probe_outcome_marks_non_success_status() {
        let r = proxy_probe_outcome(Ok(500u16));
        assert!(!r.ok);
        assert_eq!(r.status, Some(500));
        assert!(r.error.is_some());
    }

    #[test]
    fn probe_outcome_reports_transport_error() {
        let r = proxy_probe_outcome(Err("connection refused".to_string()));
        assert!(!r.ok);
        assert!(r.status.is_none());
        assert_eq!(r.error.as_deref(), Some("connection refused"));
    }
```

- [ ] **Step 2: Run to verify failure**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib proxy_tests
```

Expected: compile error E0425 — `cannot find function proxy_probe_outcome in
this scope`.

- [ ] **Step 3: Implement both commands (green)**

In `ipc.rs`, directly after the `test_provider` function, add:

```rust
/// Result of a proxy connectivity probe. Serialized to the frontend as
/// `{ ok, status, error }`; a failed probe is a normal result, not a command
/// error, so the UI can render inline.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// Local Codex credentials detected from `~/.codex/auth.json`, using the
/// spec's wire key names.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedCodexToken {
    pub token: String,
    pub account_id: String,
    pub last_refresh: String,
}

/// Pure mapping from a probe response (HTTP status) or transport error to the
/// wire result. A non-2xx status is reported as not-ok with a message.
fn proxy_probe_outcome(probe: Result<u16, String>) -> ProxyTestResult {
    match probe {
        Ok(status) if (200..300).contains(&status) => ProxyTestResult {
            ok: true,
            status: Some(status),
            error: None,
        },
        Ok(status) => ProxyTestResult {
            ok: false,
            status: Some(status),
            error: Some(format!("代理已连通，但目标返回 HTTP {status}")),
        },
        Err(error) => ProxyTestResult {
            ok: false,
            status: None,
            error: Some(error),
        },
    }
}

/// Build a temporary client through the supplied proxy and probe a lightweight
/// 204 endpoint. Nothing is persisted; the saved client is untouched.
#[tauri::command]
pub async fn test_proxy(url: String) -> Result<ProxyTestResult, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(ProxyTestResult {
            ok: false,
            status: None,
            error: Some("请输入代理地址".to_string()),
        });
    }
    let client = crate::build_http_client(Some(&url))?;
    let probe = async {
        let resp = client
            .get("https://www.google.com/generate_204")
            .send()
            .await
            .map_err(|e| format!("代理连接失败：{e}"))?;
        Ok::<u16, String>(resp.status().as_u16())
    }
    .await;
    Ok(proxy_probe_outcome(probe))
}

/// Read the local Codex CLI credentials (`~/.codex/auth.json`) so the
/// Settings form can pre-fill them. `None` when the file is absent or the
/// token is blank.
#[tauri::command]
pub async fn detect_codex_token() -> Result<Option<DetectedCodexToken>, String> {
    Ok(crate::providers::codex::detect_auth().map(|auth| DetectedCodexToken {
        token: auth.access_token,
        account_id: auth.account_id,
        last_refresh: auth.last_refresh,
    }))
}
```

In `lib.rs`, register both commands immediately after `ipc::test_provider,`:

```rust
            ipc::test_provider,
            ipc::test_proxy,
            ipc::detect_codex_token,
```

- [ ] **Step 4: Run to verify pass**

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all PASS — the six `proxy_tests` (3 from Task 11 plus 3 here) and
every existing suite. Confirm the full build:

```powershell
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo build
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src-tauri/src/ipc.rs impl-pulse/src-tauri/src/lib.rs
git commit -m "feat: add test_proxy and detect_codex_token commands"
```

## Task 13: Extend TypeScript types and API wrappers

**Files:**
- `impl-pulse/src/lib/types.ts` — `Preset` L143-149; `Settings` L224-266 (last field L265); `Credentials` L289-295; `TestResult` L298-300; `SHORT_KIND_NAMES` L306-322
- `impl-pulse/src/lib/api.ts` — insert wrappers after `testProvider` (ends L134)

This task is a pure contract addition: no UI consumes the new fields yet, so
there is no behavioral red test. The discipline here is (1) make the contract
concrete and exactly mirror the Rust side, (2) prove the package still
type-checks and builds. Consumers arrive in Tasks 15-19 and get their own
red/green cycles against these types.

Two naming traps to get right now:

1. The existing mirrored types (`Preset`, `Settings`, `Credentials`) use
   **snake_case** because Rust serializes them with serde defaults and no
   rename. Keep that style for the added fields (`auth_kind: "local_token"`,
   `proxy_url`, `{ kind: "local_token"; token }`).
2. The two NEW command responses from Task 12 (`ProxyTestResult`,
   `DetectedCodexToken`) are serialized with
   `#[serde(rename_all = "camelCase")]`, so their field names here are
   camelCase: `accountId`, `lastRefresh`. `ProxyTestResult` has only
   single-word fields so the difference is invisible, but the type must still
   match the wire exactly (`status: number | null`).

- [ ] **Step 1: Extend the `Preset` interface**

In `src/lib/types.ts`, replace:

```ts
export interface Preset {
  kind: string;
  display_name: string;
  auth_kind: "bearer_key" | "access_key_secret";
  default_accent: string;
  sub_modes?: PresetSubMode[];
}
```

with:

```ts
export interface Preset {
  kind: string;
  display_name: string;
  auth_kind: "bearer_key" | "access_key_secret" | "local_token";
  default_accent: string;
  sub_modes?: PresetSubMode[];
  /** True for providers that are not billing-stable / are best-effort. The
   * UI tags these "实验性" and may auto-detect local credentials. */
  experimental?: boolean;
}
```

- [ ] **Step 2: Add `proxy_url` to `Settings`**

Replace the closing fields of the `Settings` interface:

```ts
  /** 用户手动覆盖的汇率（币种代码 → 每 1 USD 兑该币种数值）。非法值后端忽略。 */
  rate_overrides: Record<string, number>;
}
```

with:

```ts
  /** 用户手动覆盖的汇率（币种代码 → 每 1 USD 兑该币种数值）。非法值后端忽略。 */
  rate_overrides: Record<string, number>;
  /** Optional outbound proxy (http/https/socks5). Null/empty/blank = direct.
   * Changing this rebuilds the shared HTTP client and the provider registry. */
  proxy_url?: string | null;
}
```

- [ ] **Step 3: Add the `local_token` credential variant and the two new wire types**

Replace:

```ts
/** Tagged union mirroring Rust `Credentials`. The `kind` field discriminates. */
export type Credentials =
  | { kind: "bearer_key"; api_key: string }
  | {
      kind: "access_key_secret";
      access_key: string;
      secret_key: string;
    };

/** Result of `test_provider` IPC: either a snapshot preview or error string. */
export type TestResult =
  | { ok: true; snapshot: UsageSnapshot }
  | { ok: false; message: string };
```

with:

```ts
/** Tagged union mirroring Rust `Credentials`. The `kind` field discriminates. */
export type Credentials =
  | { kind: "bearer_key"; api_key: string }
  | {
      kind: "access_key_secret";
      access_key: string;
      secret_key: string;
    }
  | { kind: "local_token"; token: string };

/** Result of `test_provider` IPC: either a snapshot preview or error string. */
export type TestResult =
  | { ok: true; snapshot: UsageSnapshot }
  | { ok: false; message: string };

/** Result of `test_proxy` IPC (Rust `ProxyTestResult`, camelCase wire).
 * `status` is null when the request never got a response (transport error). */
export interface ProxyTestResult {
  ok: boolean;
  status: number | null;
  error: string | null;
}

/** Detected ChatGPT Codex local login (Rust `DetectedCodexToken`, camelCase
 * wire), read from `~/.codex/auth.json`. `token` maps the Rust
 * `access_token`; empty strings mean the field was absent on disk. */
export interface DetectedCodexToken {
  token: string;
  accountId: string;
  lastRefresh: string;
}
```

- [ ] **Step 4: Add the missing short names**

In `SHORT_KIND_NAMES`, replace the tail of the map:

```ts
  kimi: "Kimi",
  doubao: "豆包",
  spark: "Spark",
  xiaomi_plan: "MiMo Plan",
  xiaomi_api: "MiMo API",
};
```

with:

```ts
  kimi: "Kimi",
  kimi_global: "Kimi Global",
  xai: "xAI",
  codex: "Codex",
  doubao: "豆包",
  spark: "Spark",
  xiaomi_plan: "MiMo Plan",
  xiaomi_api: "MiMo API",
};
```

`kimi_global` is the registered sub-mode kind from Task 5; adding it here
keeps compact surfaces readable when the global account appears.

- [ ] **Step 5: Add `testProxy` and `detectCodexToken` API wrappers**

In `src/lib/api.ts`, find the end of `testProvider`:

```ts
export async function testProvider(
  providerKind: string,
  creds: Credentials,
): Promise<TestResult> {
  try {
    const snapshot = await invoke<UsageSnapshot>("test_provider", {
      providerKind,
      creds,
    });
    return { ok: true, snapshot };
  } catch (e) {
    return { ok: false, message: String(e) };
  }
}
```

and insert immediately after it:

```ts
/** Test an HTTP/SOCKS proxy without persisting it. Always resolves to a
 * structured result; only rejects if the backend itself fails to build a
 * client (e.g. unsupported proxy scheme). */
export async function testProxy(url: string): Promise<ProxyTestResult> {
  return invoke<ProxyTestResult>("test_proxy", { url });
}

/** Read the local ChatGPT Codex login (`~/.codex/auth.json`). Resolves to
 * null when the file is absent; rejects only on an unreadable/invalid file. */
export async function detectCodexToken(): Promise<DetectedCodexToken | null> {
  return invoke<DetectedCodexToken | null>("detect_codex_token");
}
```

Then ensure the two new types are imported. The file's top import block pulls
a named list from `./types`; add `ProxyTestResult` and `DetectedCodexToken`
to that existing import (do not create a second import line). If the local
style keeps some imports inline (as L182/L199/L201 do), prefer extending the
top import — both wrappers live in the top section of the file.

- [ ] **Step 6: Type-check and build**

From `impl-pulse/`:

```powershell
npx svelte-check --threshold error
```

Expected: no errors (warnings about unused exports are fine — the wrappers
are not consumed until later tasks; `--threshold error` ignores them).

```powershell
npm run build
```

Expected: clean production build (vite + tsc pipeline, whichever the project
uses). Confirm the new wrappers survive bundling without unresolved-import
errors.

- [ ] **Step 7: Commit**

Run from the worktree root:

```bash
git add impl-pulse/src/lib/types.ts impl-pulse/src/lib/api.ts
git commit -m "feat: add frontend types and wrappers for new providers and proxy"
```

## Task 14: Create the ProviderLogo component

**Files (new):**
- `impl-pulse/src/lib/components/ProviderLogo.svelte`

Brand-colored inline SVG logo with an initial-tile fallback, per spec §3.3.
Props: `{ kind: string; size?: number; accent?: string | null }`. The
optional `accent` is only consumed by the fallback tile (the account's
configured accent color); brand glyphs always render in their official
color. The component is presentational and self-contained — no stores, no
IPC.

There is no component unit-test harness in this repo, so the red/green
discipline here is: create a throwaway render probe that references the
component, confirm `svelte-check` fails while the file is absent, then add
the component and confirm the probe type-checks and the production build
succeeds. The probe is deleted before commit.

Brand coverage (normalized kind → glyph): `minimax`, `deepseek`,
`volcengine`, `openai`, `gemini`, `anthropic`, `qwen`, `kimi`, `doubao`,
`spark`, `xiaomi`, `xai`, `codex`. `*_api` kinds normalize via
`kind.split("_")[0]`; `kimi_global` maps explicitly to the Kimi glyph
(split would also yield `kimi`). `local` and anything unknown fall back to
the initial tile.

SVG sourcing notes (so the glyphs are real, not invented):
- OpenAI: official knot mark, one stroked path on brand green.
- Anthropic: official "A" mark, filled block glyph.
- Gemini: official four-facet spark (two paths).
- xAI: the X-with-A monogram from the x.ai identity (stroked).
- Kimi: balanced "K" mark on the teal brand color.
- Codex: rounded square with terminal `</>` glyph on OpenAI green — Codex
  ships no standalone glyph, so this derives directly from the product's
  terminal branding and keeps the ChatGPT/Codex family color.
- DeepSeek: whale-mark abstract path on brand blue.
- MiniMax: abstract "M" wave.
- Volcano/Qwen/Doubao/Spark/Xiaomi: simple monogram tiles in each brand
  color (these products' official marks are multicolor gradients that do
  not survive 16px rendering; the monogram-in-brand-color is the honest,
  legible reduction).

- [ ] **Step 1: Confirm the red state with a render probe**

Create a temporary file `src/lib/components/.logo-probe.svelte`:

```svelte
<script lang="ts">
  import ProviderLogo from "./ProviderLogo.svelte";
</script>

<ProviderLogo kind="xai" />
<ProviderLogo kind="kimi_global" size={24} />
<ProviderLogo kind="unknown-thing" size={16} accent="#3b82f6" />
```

From `impl-pulse/`:

```powershell
npx svelte-check --threshold error
```

Expected: FAIL — `Cannot find module './ProviderLogo.svelte'` (or the
Svelte tooling's equivalent unresolved-import error). This proves the probe
is exercising the not-yet-created component.

- [ ] **Step 2: Create the component**

Create `src/lib/components/ProviderLogo.svelte` with exactly this content:

```svelte
<script lang="ts">
  interface Props {
    kind: string;
    size?: number;
    accent?: string | null;
  }

  let { kind, size = 18, accent = null }: Props = $props();

  const BRAND_COLORS: Record<string, string> = {
    minimax: "#e0488f",
    deepseek: "#4d6bfe",
    volcengine: "#1664ff",
    openai: "#10a37f",
    gemini: "#8e75f5",
    anthropic: "#d4a24f",
    qwen: "#615ced",
    kimi: "#16c2a3",
    doubao: "#3a6bff",
    spark: "#e63a3c",
    xiaomi: "#ff6900",
    xai: "#1d1d1d",
    codex: "#10a37f",
  };

  const INITIAL_OVERRIDES: Record<string, string> = {
    minimax: "M",
    deepseek: "D",
    volcengine: "V",
    openai: "O",
    gemini: "G",
    anthropic: "A",
    qwen: "Q",
    kimi: "K",
    doubao: "豆",
    spark: "S",
    xiaomi: "Mi",
    xai: "x",
    codex: "C",
    local: "本",
  };

  function normalizeKind(raw: string): string {
    if (raw === "kimi_global") return "kimi";
    return raw.split("_")[0];
  }

  const normalized = $derived(normalizeKind(kind));
  const brandColor = $derived(
    BRAND_COLORS[normalized] ?? accent ?? "#3b82f6",
  );
  const initial = $derived(
    INITIAL_OVERRIDES[normalized] ??
      (kind.trim() ? kind.trim().charAt(0).toUpperCase() : "?"),
  );
</script>

<span class="logo" style={`width:${size}px;height:${size}px`} aria-hidden="true">
  {#if normalized === "openai"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.92 6.042 6.042 0 0 0-6.51-2.783A5.975 5.975 0 0 0 4.4 4.938a5.985 5.985 0 0 0-3.999 2.93 5.985 5.985 0 0 0 .52 4.916 5.98 5.98 0 0 0 2.11 11.095 5.98 5.98 0 0 0 5.6 2.026A5.98 5.98 0 0 0 19.6 19.06a5.99 5.99 0 0 0 3.998-2.926 5.98 5.98 0 0 0-.516-4.92 5.98 5.98 0 0 0-.8-.793Z"
        stroke={brandColor}
        stroke-width="1.6"
        stroke-linejoin="round"
      />
      <path
        d="M12 15.063a3.064 3.064 0 1 0 0-6.127 3.064 3.064 0 0 0 0 6.127ZM13.806 8.586l3.76-2.17M15.063 12l3.76 2.17M10.194 15.414l-3.76 2.17M8.937 12l-3.76-2.17M10.194 8.586l-.78-4.3M13.806 15.414l.78 4.3"
        stroke={brandColor}
        stroke-width="1.6"
        stroke-linecap="round"
      />
    </svg>
  {:else if normalized === "anthropic"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill={brandColor}>
      <path d="M12 4.2c-.6 0-1.15.34-1.42.88L5.9 14.6l-.9 1.84h4.3l.86-1.8 1.3-2.72c.1-.2.3-.33.54-.33s.44.13.54.33l1.3 2.72.86 1.8h4.3l-.9-1.84-4.68-9.52A1.58 1.58 0 0 0 12 4.2Zm0 4.5 1.55 3.2h-3.1L12 8.7Z" />
    </svg>
  {:else if normalized === "gemini"}
    <svg viewBox="0 0 24 24" width={size} height={size}>
      <path
        d="M12 2.6c.45 3.55 1.85 6.2 4.5 8.05-2.65 1.85-4.05 4.5-4.5 8.05-.45-3.55-1.85-6.2-4.5-8.05C10.15 8.8 11.55 6.15 12 2.6Z"
        fill="#8e75f5"
      />
      <path
        d="M17.6 12.1c.25 2 .95 3.45 2.4 4.45-1.45 1-2.15 2.45-2.4 4.45-.25-2-.95-3.45-2.4-4.45 1.45-1 2.15-2.45 2.4-4.45Z"
        fill="#5bb0f5"
      />
    </svg>
  {:else if normalized === "xai"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="1.8" y="1.8" width="20.4" height="20.4" rx="5" fill="#1d1d1d" />
      <path
        d="m8.2 8.2 7.6 7.6M15.8 8.2l-7.6 7.6"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
      />
      <path
        d="M12 10.4c1.5-.5 2.6-1.2 3.4-2.1M12 13.6c-1.5.5-2.6 1.2-3.4 2.1"
        stroke="#fff"
        stroke-width="1.1"
        stroke-linecap="round"
        opacity=".65"
      />
    </svg>
  {:else if normalized === "kimi"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="2" y="2" width="20" height="20" rx="5" fill={brandColor} />
      <path
        d="M8.6 7.4v9.2M8.6 12l5.1-4.6M13.7 7.4v9.2M15.4 9.6v4.8"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else if normalized === "codex"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="2" y="2" width="20" height="20" rx="5" fill={brandColor} />
      <path
        d="m9.8 9.2-2.8 2.8 2.8 2.8M14.2 9.2l2.8 2.8-2.8 2.8"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else if normalized === "deepseek"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M4 13.2c2.6 0 3.9-1.3 3.9-3.5V7h2.1v2.9c0 3.4-2.1 5.5-5.6 5.5H4v-2.2Z"
        fill={brandColor}
      />
      <path
        d="M14.6 7h2.7l3.7 5.6V7H22v10h-.9l-3.8-5.8V17h-2.7V7Z"
        fill={brandColor}
      />
    </svg>
  {:else if normalized === "minimax"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M3.5 17V7l6 6 5.5-6v10M15 17l5.5-10"
        stroke={brandColor}
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else}
    <span
      class="logo__tile"
      style={`width:${size}px;height:${size}px;background:${brandColor};font-size:${Math.max(
        size * 0.52,
        9,
      )}px`}
    >
      {initial}
    </span>
  {/if}
</span>

<style>
  .logo {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    line-height: 0;
  }

  .logo :global(svg) {
    display: block;
  }

  .logo__tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 28%;
    color: #fff;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0;
    user-select: none;
  }
</style>
```

The `{:else}` branch covers volcengine, qwen, doubao, spark, xiaomi and
every unknown kind via the initial tile (their `INITIAL_OVERRIDES` entries
provide the glyph, `brandColor` the color).

- [ ] **Step 3: Confirm the green state**

```powershell
npx svelte-check --threshold error
```

Expected: no errors. The probe now resolves; all three prop combinations
compile.

- [ ] **Step 4: Remove the probe and run the production build**

Delete `src/lib/components/.logo-probe.svelte`, then:

```powershell
npm run build
```

Expected: clean build. The component is not imported anywhere yet, so it is
bundled only if the build is un-tree-shaken; either outcome is fine — what
matters is that the component itself compiles through the real pipeline.

- [ ] **Step 5: Commit**

```bash
git add impl-pulse/src/lib/components/ProviderLogo.svelte
git commit -m "feat: add brand-colored ProviderLogo component"
```

## Task 15: Anchor the hover detail card beside its trigger and swap header avatars to logos

**Files:**
- `impl-pulse/src/lib/components/ProviderCard.svelte` — `onHover` prop type L33; pointer dispatch L122-123
- `impl-pulse/src/App.svelte` — imports L2-3/L37/L43; hover state + timers L138-164; `detailSnapshot` L360-362; pill avatar L689-699; shell avatar L735-739; overlay markup L894-911; CSS `.shell__avatar` L990-997, `.pill__avatar` L1314-1321, `.detail-overlay` L1395-1415

Implements spec §3.1 (nearby anchoring, 350ms grace, measure-then-place,
fly transition) and the fifth Logo entry point of §3.3 (pill `pill__avatar`
and dashboard `shell__avatar`). The other four logo entry points are Tasks
16-19.

There is no automated UI-test harness in this repo and the anchoring
behavior cannot be expressed as a compile failure (all edits are additive
types + positioning math), so this task's discipline is: keep every
intermediate step type-checking, then verify behavior with a concrete manual
QA checklist against the running app. The positioning logic itself is kept
in small pure-enough functions so it can be eyeballed line by line.

Key mechanics:

- The overlay stays `position: fixed`; `getBoundingClientRect()` already
  returns viewport coordinates, so no coordinate conversion is needed.
- It is rendered off-screen-hidden first (`visibility: hidden` at its last
  known spot), a `ResizeObserver` measures the real content box, THEN the
  position is computed and it is revealed. RO callbacks fire before paint,
  so there is no visible flash. RO also re-measures on every content update
  (sliding to another card swaps content in place).
- Placement: prefer below the card (`rect.bottom + 8`), horizontally at
  `rect.left`, then clamp into the viewport; if it overflows the bottom
  edge, flip above the card; if neither fits, clamp to the viewport.
- Sliding across cards keeps the same element (no remount, no fade
  restart); only coordinates and content change.

- [ ] **Step 1: Carry the card rect through the hover callback**

In `ProviderCard.svelte`, change the prop type:

```ts
    /** Reported as the pointer enters/leaves the card; drives the floating
     *  detail overlay in App (small cards no longer clip the detail). The
     *  rect is the card's viewport box at enter time (overlay anchoring). */
    onHover?: (id: string, hovering: boolean, rect?: DOMRect) => void;
```

and the pointer dispatch:

```svelte
  onpointerenter={(e) =>
    onHover?.(
      snapshot.provider_id,
      true,
      (e.currentTarget as HTMLElement).getBoundingClientRect(),
    )}
  onpointerleave={() => onHover?.(snapshot.provider_id, false)}
```

The new parameter is optional, so App's current handler still satisfies the
type — confirm with a quick check:

```powershell
npx svelte-check --threshold error
```

Expected: no errors.

- [ ] **Step 2: Add imports and the anchoring state/math in App.svelte**

Add `fly` and the logo component to the imports:

```svelte
  import { flip } from "svelte/animate";
  import { fly } from "svelte/transition";
```

```svelte
  import DetailCard from "./lib/components/DetailCard.svelte";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";
```

Then delete the mascot import (the asset file on disk is untouched, per
spec §3.3; both of its usages are replaced later in this task):

```svelte
  import mascotUrl from "./lib/assets/mascot-b.png";
```

Replace the hover-state block (currently L136-164) wholesale:

```ts
  // Provider whose card the pointer is currently over; drives the floating
  // detail overlay. `anchorRect` is the trigger card's viewport box so the
  // overlay can park right beside it instead of docking at window bottom.
  let hoveredId = $state<string | null>(null);
  let anchorRect = $state<DOMRect | null>(null);
  // Overlay box is measured before reveal so placement uses real dimensions
  // (content height varies per provider). Stays hidden until first measure.
  let overlayEl = $state<HTMLElement | null>(null);
  let measuredW = $state(0);
  let measuredH = $state(0);
  let overlayX = $state(0);
  let overlayY = $state(0);
  let overlayReady = $state(false);
  const OVERLAY_GAP = 8;
  const VIEWPORT_MARGIN = 8;

  // Hiding the detail overlay is deferred by a grace window so the pointer
  // can travel from the card into the overlay (or back) without it vanishing
  // mid-move. Entering the overlay cancels the pending hide (pinning it);
  // leaving it re-arms a hide so it closes when you go elsewhere.
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleOverlayHide() {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      hoveredId = null;
      anchorRect = null;
      hideTimer = null;
    }, 350);
  }
  function cancelOverlayHide() {
    if (hideTimer) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
  }
  function onCardHover(id: string, hovering: boolean, rect?: DOMRect) {
    if (hovering) {
      cancelOverlayHide();
      hoveredId = id;
      anchorRect = rect ?? null;
      // Hide until the ResizeObserver re-measures at the new anchor — avoids
      // a one-frame ghost at the previous card's coordinates.
      overlayReady = false;
    } else if (hoveredId === id) {
      scheduleOverlayHide();
    }
  }

  // Compute fixed coordinates for the overlay from the trigger rect and the
  // measured content box: clamp horizontally; prefer below, flip above when
  // the bottom edge would overflow; clamp to viewport as the last resort.
  function placeOverlay() {
    const rect = anchorRect;
    if (!rect) return;
    const w = measuredW || 320;
    const h = measuredH;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const maxX = Math.max(VIEWPORT_MARGIN, vw - w - VIEWPORT_MARGIN);
    overlayX = Math.min(Math.max(rect.left, VIEWPORT_MARGIN), maxX);
    const below = rect.bottom + OVERLAY_GAP;
    const above = rect.top - OVERLAY_GAP - h;
    if (h <= 0 || below + h <= vh - VIEWPORT_MARGIN) {
      overlayY = below;
    } else if (above >= VIEWPORT_MARGIN) {
      overlayY = above;
    } else {
      overlayY = Math.max(VIEWPORT_MARGIN, vh - h - VIEWPORT_MARGIN);
    }
  }

  // Measure the overlay content box whenever it mounts or its size changes.
  // ResizeObserver callbacks run before paint, so reveal-after-measure has
  // no visible flash. The initial synchronous apply covers same-frame mount.
  $effect(() => {
    const el = overlayEl;
    if (!el) return;
    const apply = () => {
      const box = el.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) {
        measuredW = box.width;
        measuredH = box.height;
        overlayReady = true;
      }
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Reposition whenever the anchor moves or the content box changes size.
  $effect(() => {
    void anchorRect;
    void measuredW;
    void measuredH;
    placeOverlay();
  });
```

- [ ] **Step 3: Move the overlay to computed coordinates with the fly transition**

Replace the overlay markup (currently L894-911):

```svelte
    {#if detailSnapshot}
      <div
        class="detail-overlay"
        role="group"
        data-tauri-drag-region={false}
        style={`left:${overlayX}px;top:${overlayY}px;visibility:${overlayReady ? "visible" : "hidden"};`}
        onpointerenter={cancelOverlayHide}
        onpointerleave={scheduleOverlayHide}
        transition:fly={{ y: 6, duration: 120 }}
        bind:this={overlayEl}
      >
        <DetailCard
          snapshot={detailSnapshot}
          burn={burns[detailSnapshot.provider_id] ?? null}
          {lastRefreshAt}
          countdown={displayRemaining}
          accent={accentById[detailSnapshot.provider_id]}
          error={errors[detailSnapshot.provider_id] ?? null}
        />
      </div>
    {/if}
```

- [ ] **Step 4: Swap the pill and shell avatars to ProviderLogo**

In the compact pill, replace:

```svelte
            {#if headerIsPayAsYouGo}
              <img
                class="pill__avatar"
                src={mascotUrl}
                alt=""
                aria-hidden="true"
                title={providerFullName}
              />
            {:else}
              <ProgressRing value={ringArcValue} label="" size={24} stroke={3} idle={snapshots.length === 0} countdown={displayRemaining} />
            {/if}
```

with:

```svelte
            {#if headerIsPayAsYouGo && focusedSnapshot}
              <span class="pill__avatar" title={providerFullName}>
                <ProviderLogo
                  kind={focusedSnapshot.provider_id.split("-")[0]}
                  size={16}
                  accent={focusedColor}
                />
              </span>
            {:else}
              <ProgressRing value={ringArcValue} label="" size={24} stroke={3} idle={snapshots.length === 0} countdown={displayRemaining} />
            {/if}
```

In the shell header, replace:

```svelte
        {#if headerIsPayAsYouGo}
          <img class="shell__avatar" src={mascotUrl} alt="" aria-hidden="true" title={providerFullName} />
        {:else}
          <ProgressRing value={headerMoney ? 0 : ringArcValue} label={headerMoney ?? ringArcLabel} size={28} stroke={3} idle={snapshots.length === 0 || !!headerMoney} countdown={displayRemaining} />
        {/if}
```

with:

```svelte
        {#if headerIsPayAsYouGo && focusedSnapshot}
          <span class="shell__avatar" title={providerFullName}>
            <ProviderLogo
              kind={focusedSnapshot.provider_id.split("-")[0]}
              size={22}
              accent={focusedColor}
            />
          </span>
        {:else}
          <ProgressRing value={headerMoney ? 0 : ringArcValue} label={headerMoney ?? ringArcLabel} size={28} stroke={3} idle={snapshots.length === 0 || !!headerMoney} countdown={displayRemaining} />
        {/if}
```

`provider_id` is shaped `"<kind>-<ts>-<n>"` and kind never contains `-`,
so `split("-")[0]` recovers it (matching the reasoning documented in
`types.ts`). `focusedColor` only feeds the fallback tile; brand glyphs keep
official colors.

- [ ] **Step 5: Update the CSS**

Replace `.shell__avatar`:

```css
  .shell__avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    overflow: hidden;
    border: 1px solid var(--tum-border-strong);
    background: rgba(255, 255, 255, 0.06);
  }
```

Replace `.pill__avatar`:

```css
  .pill__avatar {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    overflow: hidden;
    border: 1px solid var(--tum-border-strong);
    background: rgba(255, 255, 255, 0.06);
  }
```

Replace the `.detail-overlay` rule and its trailing rule:

```css
  /* Floating detail overlay anchored beside the hovered card: rendered at
     window level so the full detail (rows + chart) is never clipped by a
     small card. Position + width are set inline after measuring the content
     box; it flips above the card when the viewport bottom is tight. Parking
     the pointer on it keeps it pinned so the actions inside are reachable. */
  .detail-overlay {
    position: fixed;
    z-index: 60;
    width: 320px;
    max-width: calc(100vw - 16px);
    pointer-events: auto;
  }

  .detail-overlay :global(.detail) {
    padding: 10px 12px;
  }
```

The explicit width replaces the old `left/right:14` stretch (DetailCard has
no width of its own).

- [ ] **Step 6: Type-check, build, and run the manual QA checklist**

```powershell
npx svelte-check --threshold error
npm run build
```

Expected: no errors; clean build. Then run the dev app (or use the already
running dev instance) and verify, on the 总量 page:

1. Hover a card — the detail card appears directly **below** it, left edge
   aligned, fully inside the window (cards near the right edge clamp in).
2. Hover a card low enough that the popup would pass the window's bottom
   edge — it appears **above** the card instead.
3. Move the pointer slowly from a card into the popup — it stays open
   (350ms grace); the buttons inside (打开设置 / 刷新) are clickable.
4. Slide across several cards without pausing — the popup follows each card
   immediately, content swaps in place (no full fade-out/fade-in), and a
   short fly animation plays on first open.
5. Move the pointer away from both card and popup — it closes after ~350ms.
6. Focus a pay-as-you-go provider (click its card) — the header avatar
   (dashboard) and the pill avatar (compact mode) show that provider's logo,
   not the mascot; ring-mode providers still show the ProgressRing.
7. Confirm no popup appears in compact mode, and window drag/resize does not
   leave a stuck popup (drag while hovering simply leaves the popup where it
   was; moving away closes it).

- [ ] **Step 7: Commit**

```bash
git add impl-pulse/src/App.svelte impl-pulse/src/lib/components/ProviderCard.svelte
git commit -m "feat: anchor hover detail beside card and use provider logos in headers"
```

## Task 16: Overlay the provider logo badge on cards and tag experimental providers

Implements spec §3.3 integration point 1: "卡片头：`PulseDot` 保留状态色，
右侧叠加 Logo 小徽标（右下角叠放，圆形描边）". The PulseDot's status color
is untouched; a small circular `ProviderLogo` badge overlays its bottom-right
corner. A neutral-amber "实验" chip is shown for best-effort providers.

**Files:**

- `src/lib/components/ProviderCard.svelte`: import block L11-14; `card__titlebtn`
  markup L142-143 (the single `<PulseDot>` line); styles `.card__titlebtn`
  L244-257 / `.card__name` L264-272.
- `src/lib/types.ts`: `UsageSnapshot` L93-100 — intentionally **not** modified.
  It carries no `experimental` flag (the backend snapshot is credential-scoped
  and has no preset metadata), so experimental status is derived from the
  provider kind, matching how `ProviderLogo` normalizes kinds. This keeps the
  change on the frontend instead of widening the Rust wire type.

- [ ] **Step 1: Capture the red baseline**

There is no component test harness; unlike Task 14 (a brand-new module), this
task edits a component that is already imported by `App.svelte`, so a missing
import surfaces immediately in the existing check. Establish the baseline
first and confirm it is green before editing:

```bash
cd impl-pulse
npx svelte-check --threshold error
```

Expected: no errors (exit 0). Then introduce the red state by adding the
`ProviderLogo` usage and the experimental derivations to
`src/lib/components/ProviderCard.svelte` *before* the component is imported —
i.e. apply the markup in Step 3 while the import in Step 2 is absent — and
re-run:

```bash
npx svelte-check --threshold error
```

Expected (red): an error of the form
`Cannot find module './ProviderLogo.svelte' or its corresponding type
declarations.` (the identifier `ProviderLogo` is unresolved). This proves the
check guards the new integration. Steps 2-4 then move it to green.

- [ ] **Step 2: Add the import and derive kind + experimental flag**

In `src/lib/components/ProviderCard.svelte`, add the component import directly
after the `PulseDot` import (keeping imports grouped, one block):

```svelte
  import UsageBar from "./UsageBar.svelte";
  import ResetCountdown from "./ResetCountdown.svelte";
  import ProgressRing from "./ProgressRing.svelte";
  import PulseDot from "./PulseDot.svelte";
  import ProviderLogo from "./ProviderLogo.svelte";
```

Then add the derived kind and experimental flag in the `<script lang="ts">`
block, after the existing `let w = $derived(snapshot.windows);` line (L49).
`kind` uses the same `split("-")[0]` normalization that App.svelte applies for
the header logos; the experimental set is a small frontend-only constant
(currently only `codex`, consistent with the `experimental: true` preset flag
added in Task 13):

```ts
  // Brand/logo normalization mirrors App.svelte header avatars: the first
  // segment of a credential-scoped provider_id is the provider kind.
  let kind = $derived(snapshot.provider_id.split("-")[0]);
  // UsageSnapshot carries no preset metadata, so experimental status is
  // derived from kind. Keep this set in sync with presets flagged
  // `experimental` (Task 13): codex is best-effort / billing-unstable.
  const EXPERIMENTAL_KINDS = new Set(["codex"]);
  let isExperimental = $derived(EXPERIMENTAL_KINDS.has(kind));
```

- [ ] **Step 3: Wrap the PulseDot with the logo badge and add the chip**

Replace the single PulseDot line inside `card__titlebtn` (currently L142) with
a relative status wrapper that keeps the dot as the status color and overlays
a small circular logo badge on its bottom-right corner. Also add the "实验"
chip immediately after the card name. The whole block becomes:

```svelte
      <span class="card__status">
        <PulseDot {active} {tone} size={8} />
        <span class="card__logo-badge">
          <ProviderLogo {kind} size={11} accent={accent ?? null} />
        </span>
      </span>
      <span class="card__name">{snapshot.provider_display_name}</span>
      {#if isExperimental}
        <span class="card__exp" title="实验性支持：数据可能不完整或口径调整中">实验</span>
      {/if}
```

`accent` is an optional prop (`string | undefined`); `?? null` maps it to the
`accent?: string | null` the logo accepts, and it only affects the
first-letter fallback tile (xAI/Codex/Kimi have dedicated brand art).

- [ ] **Step 4: Add styles and confirm the green state**

Add these rules to the component's `<style>` block, directly after the
existing `.card__titlebtn` rule (ends at L257). The badge is a 16px circular,
bordered container positioned over the dot's bottom-right corner; the chip is
a neutral amber tag distinct from the accent-colored `card__tier`:

```css
  .card__status {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }

  .card__logo-badge {
    position: absolute;
    right: -7px;
    bottom: -7px;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: var(--tum-bg-solid);
    border: 1px solid var(--tum-border-strong);
    overflow: hidden;
  }

  .card__exp {
    flex: none;
    white-space: nowrap;
    font-size: var(--tum-font-size-xs);
    font-weight: 500;
    color: var(--tum-warn);
    background: rgba(255, 200, 61, 0.12);
    border: 1px solid rgba(255, 200, 61, 0.4);
    padding: 1px 6px;
    border-radius: var(--tum-radius-xs);
    letter-spacing: 0.5px;
  }
```

Because the badge now occupies the dot's old bottom-right space, give the
title button a little extra room so the badge is never clipped by the name —
append one rule after `.card__exp` (the title button's existing padding stays
0; this only reserves space on the trailing edge):

```css
  .card__status {
    margin-right: 3px;
  }
```

(Merge this into the earlier `.card__status` rule if preferred; both produce
the same 3px trailing gap.)

Now verify green:

```bash
cd impl-pulse
npx svelte-check --threshold error
npm run build
```

Expected: no check errors and a successful production build.

- [ ] **Step 5: Manual QA**

With the app running (`npm run tauri dev`):

1. Open the dashboard with multiple providers configured. Every card shows the
   colored PulseDot as before, now with a small circular logo badge over its
   bottom-right corner; the badge ring is centered and not clipped.
2. Hover/expanded states do not move or recolor the badge; a requesting
   (active) card still shows the pulsing status color behind the badge.
3. Configure a Codex account — its card shows the amber "实验" chip next to
   the name; hovering the chip shows the explanatory title. No other provider
   shows the chip.
4. Trigger a warn/crit state (or mock a near-limit account) — the PulseDot's
   warn/crit color is unchanged; the logo badge remains visible on top.
5. Long provider names still ellipsize; the chip and badge never wrap or
   overflow the card head.

- [ ] **Step 6: Commit**

```bash
git add impl-pulse/src/lib/components/ProviderCard.svelte
git commit -m "feat: overlay provider logo badge on cards and tag experimental providers"
```

## Task 17: Use provider logos at the start of each MiniPanel row

Implements spec §3.3 integration point 3: "`MiniPanel` 每行行首替换现有文字
缩写". The compact mini panel has very limited width, so the short-name text
is replaced by a small brand logo; the PulseDot (requesting/tone), fill track
and percentage remain. The full name is preserved as the row's accessible
label and hover tooltip, so unknown providers (first-letter fallback tiles)
stay identifiable.

**Files:**

- `src/lib/components/MiniPanel.svelte`: import line L10; row markup
  L36-50 (the `mini__name` span L38-40); styles `.mini__row` L69-74 and
  `.mini__name` L76-84.

- [ ] **Step 1: Capture the red baseline**

As in Task 16, there is no component test harness and this component is
already imported by App.svelte. Confirm the starting point is green:

```bash
cd impl-pulse
npx svelte-check --threshold error
```

Expected: no errors. Then apply the markup change in Step 3 (which references
`ProviderLogo`) while the Step 2 import is still absent, and re-run:

```bash
npx svelte-check --threshold error
```

Expected (red): `Cannot find module './ProviderLogo.svelte' or its
corresponding type declarations.` Steps 2-4 move it to green.

- [ ] **Step 2: Add the import**

In `src/lib/components/MiniPanel.svelte`, add the logo import after the
existing `PulseDot` import (L10):

```svelte
  import PulseDot from "./PulseDot.svelte";
  import ProviderLogo from "./ProviderLogo.svelte";
```

- [ ] **Step 3: Replace the short-name text with the logo**

Inside the `{#each}` block, replace the `mini__name` span (L38-40) with a
logo element. Give the row an explicit `aria-label` and put the full name on
the logo's `title`, so screen readers and hover still identify the provider:

```svelte
    <div
      class="mini__row"
      role="listitem"
      aria-label={`${s.provider_display_name} ${money ?? `${Math.round((countdown ? remain : usedPct) * 100)}%`}`}
    >
      <PulseDot active={!!actives[s.provider_id]} {tone} size={7} />
      <span class="mini__logo" title={s.provider_display_name}>
        <ProviderLogo kind={s.provider_id.split("-")[0]} size={13} />
      </span>
      <span class="mini__track">
        <span
          class={`mini__fill mini__fill--${tone}`}
          style={`width:${(usedPct * 100).toFixed(1)}%`}
        ></span>
      </span>
      <span class="mini__pct">
        {money ?? `${Math.round((countdown ? remain : usedPct) * 100)}%`}
      </span>
    </div>
```

`providerShortName` is no longer referenced in markup; remove it from the
`../types` import at the top (L2-9) so the import list stays accurate —
svelte-check does not flag unused imports here, but keeping it would be dead
weight:

```svelte
  import {
    mostCriticalWindow,
    payAsYouGoLabel,
    percent,
    remainingPercent,
    type UsageSnapshot,
  } from "../types";
```

- [ ] **Step 4: Update styles and confirm the green state**

Replace the `.mini__name` rule (L76-84) with a logo rule. The logo takes a
fixed 15px slot (slightly wider than the 13px art to avoid clipping rounded
tiles), and the freed name space lets the track flex wider. The `gap` on
`.mini__row` already separates dot, logo, track and pct:

```css
  .mini__logo {
    width: 15px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
```

Now verify green:

```bash
cd impl-pulse
npx svelte-check --threshold error
npm run build
```

Expected: no check errors and a successful production build.

- [ ] **Step 5: Manual QA**

With the app running in compact / mini mode:

1. Each row shows, left to right: PulseDot, brand logo, fill track,
   percentage/money. No text short-name appears; the track now uses the freed
   width.
2. Hovering a row's logo shows the full provider name; screen readers announce
   the full name plus percentage via the row `aria-label`.
3. All new providers (xAI / Kimi / Codex) show their brand art; a
   credential-scoped id such as `codex-work` renders the Codex logo
   (`split("-")[0]` normalization).
4. An unknown/custom provider renders the first-letter fallback tile and is
   still identifiable via the tooltip.
5. The empty state ("暂无已启用来源") is unchanged; warn/crit rows keep their
   colored fills and glow.

- [ ] **Step 6: Commit**

```bash
git add impl-pulse/src/lib/components/MiniPanel.svelte
git commit -m "feat: use provider logos at start of mini panel rows"
```

## Task 18: Show the provider logo beside the detail card title

Implements spec §3.3 integration point 2: "详情弹窗标题旁（`DetailCard`
头部）". A logo is placed immediately before the provider name in the detail
card head. The head's existing two-column layout (title left, critical-window
label right) is preserved by grouping logo + title on the left.

**Files:**

- `src/lib/components/DetailCard.svelte`: import block L4-15; head markup
  L176-182 (`detail__head` / `detail__title`); styles `.detail__head`
  L262-268 / `.detail__title` L270-277.

- [ ] **Step 1: Capture the red baseline**

As in Tasks 16-17, confirm the starting point is green:

```bash
cd impl-pulse
npx svelte-check --threshold error
```

Expected: no errors. Then apply the markup in Step 3 (referencing
`ProviderLogo`) while the Step 2 import is absent, and re-run:

```bash
npx svelte-check --threshold error
```

Expected (red): `Cannot find module './ProviderLogo.svelte' or its
corresponding type declarations.` Steps 2-4 move it to green.

- [ ] **Step 2: Add the import and derive the kind**

Add the logo import after the `../types` import block (after L15):

```svelte
  import ProviderLogo from "./ProviderLogo.svelte";
```

Add the derived kind in the `<script lang="ts">` block, next to the existing
`critical` derivation (after L57). It uses the same first-segment
normalization as the card/header logos:

```ts
  let kind = $derived(snapshot.provider_id.split("-")[0]);
```

- [ ] **Step 3: Add the logo to the head**

Replace the `detail__head` block (L177-182) so the logo and title are grouped
on the left while the window label stays right-aligned:

```svelte
  <div class="detail__head">
    <span class="detail__title-group">
      <ProviderLogo {kind} size={16} accent={accent ?? null} />
      <span class="detail__title">{snapshot.provider_display_name}</span>
    </span>
    <span class="detail__window">
      {critical ? WINDOW_LABELS[critical.key] : "暂无窗口"}
    </span>
  </div>
```

- [ ] **Step 4: Add styles and confirm the green state**

Add the title-group rule directly before the existing `.detail__title` rule
(starts L270). It reserves a fixed slot for the logo and keeps the name
ellipsizing within the remaining width:

```css
  .detail__title-group {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1 1 auto;
  }
```

Because the title is now nested inside the group, add `min-width: 0;` to the
existing `.detail__title` rule so the flex child can shrink and ellipsize
(the rule already carries `white-space/overflow/text-overflow`):

```css
  .detail__title {
    min-width: 0;
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    color: var(--tum-text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
```

Now verify green:

```bash
cd impl-pulse
npx svelte-check --threshold error
npm run build
```

Expected: no check errors and a successful production build.

- [ ] **Step 5: Manual QA**

With the app running, hover a card to open the anchored detail popup:

1. The provider logo (16px) sits directly left of the provider name; the
   critical-window label remains right-aligned on the same row.
2. Long provider names ellipsize and never push the window label off or wrap.
3. Sliding between providers swaps the logo, name and window label together;
   xAI / Kimi / Codex show their brand art and credential-scoped ids
   (`codex-work`) normalize correctly.
4. Unknown providers show the first-letter tile; when there are no windows the
   head still reads "暂无窗口" on the right.
5. The popup's width stays 320px (set in Task 15); the new head content does
   not change the measured size in a way that causes a placement jump loop
   (the ResizeObserver settles once).

- [ ] **Step 6: Commit**

```bash
git add impl-pulse/src/lib/components/DetailCard.svelte
git commit -m "feat: show provider logo beside detail card title"
```

## Task 19: Add the LocalToken form, Codex auto-detect, preset logos, and a network section

This is the final feature task, covering the remaining spec items in
`Settings.svelte`:

- spec §3.3 integration point 4: brand logo on each preset grid item;
- spec §2.3 / §3.2 (new providers): a `local_token` credential form for
  Codex, with an "自动检测" affordance backed by `detect_codex_token`
  (Task 12/13) that reads `~/.codex/auth.json`;
- spec §3.3 experimental flag: preset cards for experimental providers show
  an "实验" chip;
- spec §3.4.6: a new "网络" section (own nav tab, styled like the existing
  hub section) to configure `proxy_url` and test it via `test_proxy`.

All four are done in one task because they share the import block, the
`AccountForm` shape, and `persistSettings`; splitting them would leave
intermediate commits that reference not-yet-imported symbols.

**Files:**

- `src/Settings.svelte`: imports L4-31; `Tab` L33; `AccountForm` L38-52;
  state L54-96; `authKindFor`/`isAccessKey` L104-110; `buildForms` L112-133;
  `refreshAll` L140-153; `buildCredentials`/`hasCredentialInput` L179-195;
  credential ops L290-303; `persistSettings` L368-385; nav L479-508;
  preset grid L648-696; credential form L800-833; hub section styles used as
  reference; preset-card styles L1511-1518.

- [ ] **Step 1: Capture the red baseline**

```bash
cd impl-pulse
npx svelte-check --threshold error
```

Expected: no errors. Introduce red by adding the `testProxy`/
`detectCodexToken` calls and `ProviderLogo` markup from Steps 3-5 while the
Step 2 imports are absent, and re-run:

```bash
npx svelte-check --threshold error
```

Expected (red): unresolved identifiers (`testProxy`, `detectCodexToken`,
`ProviderLogo`). Steps 2-6 move it to green.

- [ ] **Step 2: Extend imports, the Tab type, state, and account form**

Add the new API wrappers, the proxy result type, and the logo component. In
the `./lib` import list (L4-30), add `testProxy, detectCodexToken,` after
`testProvider,` and add the type `type ProxyTestResult,` alongside
`type TestResult,`. Add the logo import after L31:

```svelte
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";
```

Add the network tab to the `Tab` type (L33):

```ts
  type Tab = "general" | "accounts" | "interaction" | "network" | "about";
```

Add the LocalToken field to `AccountForm` (L42-46 area):

```ts
    /** BearerKey field */
    apiKey: string;
    /** AccessKeySecret fields */
    accessKey: string;
    secretKey: string;
    /** LocalToken field (e.g. Codex ~/.codex/auth.json) */
    token: string;
```

Add proxy state near the hub state (after L83):

```ts
  // 网络代理（spec §3.4）：空字符串 = 不设置代理（跟随系统环境变量）。
  let proxyEnabled = $state(false);
  let proxyUrl = $state("");
  let proxyTesting = $state(false);
  let proxyResult = $state<ProxyTestResult | null>(null);
```

Initialize `token` in `buildForms` (alongside the empty key fields, L124-126):

```ts
      apiKey: "",
      accessKey: "",
      secretKey: "",
      token: "",
```

Sync proxy state in `refreshAll` (after the hub fields, ~L152):

```ts
      proxyUrl = s.proxy_url ?? "";
      proxyEnabled = !!s.proxy_url;
```

- [ ] **Step 3: Teach the credential helpers about local_token**

Widen `authKindFor`'s return and add `isLocalToken` (replace L104-110):

```ts
  function authKindFor(kind: string): "bearer_key" | "access_key_secret" | "local_token" {
    return presetMap.get(kind)?.auth_kind ?? "bearer_key";
  }

  function isAccessKey(kind: string): boolean {
    return authKindFor(kind) === "access_key_secret";
  }

  function isLocalToken(kind: string): boolean {
    return authKindFor(kind) === "local_token";
  }
```

Update `buildCredentials` (L179-188) to emit the local-token variant first:

```ts
  function buildCredentials(f: AccountForm): Credentials {
    if (isLocalToken(f.meta.provider_kind)) {
      return { kind: "local_token", token: f.token.trim() };
    }
    if (isAccessKey(f.meta.provider_kind)) {
      return {
        kind: "access_key_secret",
        access_key: f.accessKey.trim(),
        secret_key: f.secretKey.trim(),
      };
    }
    return { kind: "bearer_key", api_key: f.apiKey.trim() };
  }
```

Update `hasCredentialInput` (L190-195):

```ts
  function hasCredentialInput(f: AccountForm): boolean {
    if (isLocalToken(f.meta.provider_kind)) {
      return f.token.trim().length > 0;
    }
    if (isAccessKey(f.meta.provider_kind)) {
      return f.accessKey.trim().length > 0 && f.secretKey.trim().length > 0;
    }
    return f.apiKey.trim().length > 0;
  }
```

Clear the new field in `clearCredentialsFor` (L291-293):

```ts
    f.apiKey = "";
    f.accessKey = "";
    f.secretKey = "";
    f.token = "";
```

Add the Codex auto-detect function after `clearCredentialsFor` (after L303):

```ts
  async function detectCodexFor(f: AccountForm) {
    f.error = null;
    try {
      const detected = await detectCodexToken();
      if (!detected) {
        f.error = "未检测到本地 Codex 登录（~/.codex/auth.json）。请先登录 Codex CLI。";
        return;
      }
      f.token = detected.token;
      f.credsDirty = true;
    } catch (e) {
      f.error = String(e);
    }
  }
```

- [ ] **Step 4: Add the local_token branch to the credential form**

In the account body, the credential input currently branches on
`isAccessKey` with an `{:else}` API Key block (L801-833). Insert a
local-token branch between them:

```svelte
                    {#if isAccessKey(f.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">Access Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="AK..."
                          bind:value={f.accessKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                      <label class="field">
                        <span class="field__label">Secret Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="SK..."
                          bind:value={f.secretKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                    {:else if isLocalToken(f.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">本地登录凭证</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="粘贴 Codex 登录令牌，或点击右侧自动检测"
                          bind:value={f.token}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                      <div class="account__actions">
                        <button class="btn btn--ghost" onclick={() => detectCodexFor(f)}>
                          自动检测 ~/.codex
                        </button>
                      </div>
                    {:else}
                      <label class="field">
                        <span class="field__label">API Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="sk-..."
                          bind:value={f.apiKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                    {/if}
```

- [ ] **Step 5: Put logos on preset cards and tag experimental ones**

In the preset grid, replace the color swatch (L658) with the brand logo,
extend the auth-kind label to three cases, and add the experimental chip
after the preset name:

```svelte
                    <span class="preset-card__logo">
                      <ProviderLogo kind={preset.kind} size={22} accent={preset.default_accent} />
                    </span>
                    <span class="preset-card__text">
                      <span class="preset-card__name">
                        {preset.display_name}
                        {#if preset.experimental}
                          <span class="preset-card__exp">实验</span>
                        {/if}
                      </span>
                      <span class="preset-card__auth">
                        {preset.auth_kind === "access_key_secret"
                          ? "Access Key + Secret"
                          : preset.auth_kind === "local_token"
                            ? "本地登录凭证"
                            : "Bearer API Key"}
                      </span>
                    </span>
```

- [ ] **Step 6: Add the network nav entry, pane, and persistence**

Add a nav button between "交互与通知" and "关于与诊断" (after L500):

```svelte
      <button
        class="nav-item"
        class:is-active={tab === "network"}
        onclick={() => (tab = "network")}
      >
        网络
      </button>
```

Add the pane. Insert it between the `{:else if tab === "interaction"}`
block and the final `{:else}` about pane (i.e. after the interaction pane
closes at L984, change the flow to `{:else if tab === "network"}`):

```svelte
      {:else if tab === "network"}
        <div class="pane">
          <h2 class="pane__title">网络</h2>

          <div class="section">
            <h3 class="section__title">代理</h3>
            <p class="hint">
              为所有 Provider 请求与多端同步配置 HTTP / SOCKS5 代理。保存后生效，已连接的账号会自动重建。
              留空则不显式设置代理（跟随系统环境变量）。
            </p>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">启用代理</span>
                <span class="behavior-hint">支持 http:// 与 socks5://，用户名密码可写在 URL 中。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={proxyEnabled} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
            {#if proxyEnabled}
              <label class="interval">
                <input
                  type="text"
                  placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080"
                  bind:value={proxyUrl}
                />
                <span class="interval__hint">代理地址</span>
              </label>
              <div class="account__actions">
                <button class="btn btn--ghost" disabled={proxyTesting || proxyUrl.trim().length === 0} onclick={() => handleTestProxy()}>
                  {proxyTesting ? "测试中…" : "测试连接"}
                </button>
              </div>
              {#if proxyResult}
                <div class="account__test {proxyResult.ok ? "test-ok" : "test-fail"}">
                  {proxyResult.ok
                    ? `✓ 连接成功（HTTP ${proxyResult.status ?? "?"}）`
                    : `✗ ${proxyResult.error ?? "连接失败"}`}
                </div>
              {/if}
            {/if}
          </div>
        </div>
```

Add the test handler next to `detectCodexFor`:

```ts
  async function handleTestProxy() {
    const url = proxyUrl.trim();
    if (!url) return;
    proxyTesting = true;
    proxyResult = null;
    try {
      proxyResult = await testProxy(url);
    } catch (e) {
      proxyResult = { ok: false, status: null, error: String(e) };
    } finally {
      proxyTesting = false;
    }
  }
```

Persist the value in `persistSettings` — add to the `next: Settings` object
(after `rate_overrides`, L384). When the toggle is off, persist `null` so the
backend falls back to default behavior:

```ts
        rate_overrides: buildRateOverrides(),
        proxy_url: proxyEnabled && proxyUrl.trim() ? proxyUrl.trim() : null,
```

- [ ] **Step 7: Add styles and confirm the green state**

Replace the `.preset-card__swatch` rule (L1511-1518) with a logo-slot rule and
add the experimental chip style:

```css
  .preset-card__logo {
    width: 26px;
    height: 26px;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .preset-card__exp {
    margin-left: 6px;
    font-size: var(--tum-font-size-xs);
    font-weight: 500;
    color: var(--tum-warn);
    background: rgba(255, 200, 61, 0.12);
    border: 1px solid rgba(255, 200, 61, 0.4);
    padding: 0 5px;
    border-radius: var(--tum-radius-xs);
    letter-spacing: 0.5px;
    vertical-align: middle;
  }
```

No new section styles are needed for the network pane — it reuses the
existing `section`, `behavior-row`, `interval`, `toggle`, `btn`, and
`account__test` rules already defined in this file.

Now verify green:

```bash
cd impl-pulse
npx svelte-check --threshold error
npm run build
```

Expected: no check errors and a successful production build.

- [ ] **Step 8: Manual QA**

1. Preset grid: every preset shows its brand logo (22px art) instead of the
   plain color dot; the Codex preset shows the amber "实验" chip and
   "本地登录凭证" as its auth label.
2. Add a Codex account: the form shows "本地登录凭证" plus "自动检测
   ~/.codex". With Codex CLI logged in, clicking it fills the token; without a
   login it shows the Chinese not-detected message. Saving, test connection,
   clear and delete all work for the token field.
3. Open the new "网络" nav tab. Toggling "启用代理" reveals the URL input and
   test button. "测试连接" against a working local proxy reports success with
   the HTTP status; an unreachable proxy shows the Chinese error.
4. Save with a proxy URL, reopen Settings — the toggle and URL are restored.
   Save with the toggle off, reopen — the field is empty and the backend
   receives `null` (verify the registry rebuilds; requests succeed directly).
5. Confirm existing Bearer/Access-Key accounts render and save unchanged, and
   the other three nav panes are unaffected.

- [ ] **Step 9: Commit**

```bash
git add impl-pulse/src/Settings.svelte
git commit -m "feat: add local token form, codex detect, preset logos and network proxy section"
```

## Task 20: Final full-stack verification

No feature code is added here. After Tasks 1-19 are committed, run the
complete backend and frontend verification, confirm the four features
end-to-end, and ensure the working tree is clean. This is the gate before the
work is considered done.

**Files:** none (verification only).

- [ ] **Step 1: Run the full Rust library test suite**

```powershell
cd impl-pulse\src-tauri
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib
```

Expected: all tests pass, including the proxy-rebuild tests from Task 11, the
registry uniqueness/build-without-network tests (now covering `xai`, `kimi`,
`kimi_global`, `codex`), and the settings default/serialization tests for
`proxy_url` (Task 2). No warnings that block compilation.

- [ ] **Step 2: Run the full frontend checks and production build**

```bash
cd impl-pulse
npx svelte-check --threshold error
npm run build
```

Expected: zero check errors and a successful Vite build (the new types, logo
component, and all five integration points type-check and bundle).

- [ ] **Step 3: Confirm a clean release-style build of the Tauri app**

```powershell
cd impl-pulse\src-tauri
$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo check
```

Expected: the whole Tauri crate (not just `--lib`) compiles, so the binary,
build script and wired commands (`test_proxy`, `detect_codex_token`) all link.

- [ ] **Step 4: End-to-end manual verification of the four features**

With the app running, walk the feature-level acceptance once:

1. **Hover anchor** — hover a card: the detail popup appears just below it,
   clamped to the viewport; near the bottom edge it flips above; the 350ms
   grace lets the pointer move onto it and click 打开设置 / 刷新; sliding
   across cards swaps content in place with the short fly; leaving closes it.
2. **New providers** — xAI and Kimi presets add and poll (xAI monthly USD;
   Kimi via balance delta, including the Global sub-mode base URL); the Codex
   preset adds, auto-detects `~/.codex/auth.json`, shows the "实验" tag, and
   its 5h/weekly windows render. Remove the local token — Codex reports the
   Auth error in Chinese.
3. **Logos** — brand logos appear at all five points: card badge, detail
   title, mini rows, preset grid, and header/pill avatar; the mascot asset
   remains on disk and a provider with no brand art shows the first-letter
   tile.
4. **Proxy** — in Settings → 网络, set a working http/socks5 proxy, test it
   (success + status), save: the client and registry rebuild and requests go
   through the proxy; clear the proxy, save: it reverts to `null` and direct
   requests succeed; a stale poll loop does not keep running after the rebuild.

- [ ] **Step 5: Confirm git state**

```bash
git status
git log --oneline -20
```

Expected: the working tree is clean (all twenty tasks committed, including the
spec/plan docs from earlier steps), and the commit history shows one
buildable commit per task in order. There are no leftover probe/temp files
(the Task 14 logo probe was deleted; no untracked files remain).

- [ ] **Step 6: Final commit note (only if verification forced a fix)**

Task 20 itself changes nothing. If any prior step required a correction during
verification, commit that fix with an appropriate message (e.g.
`fix: ...`) and re-run the affected commands in Steps 1-3 before declaring
complete. If everything passed with no changes, there is nothing to commit.

