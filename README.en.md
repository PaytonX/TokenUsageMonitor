<div align="center">

<img src="assets/generated/token-usage-monitor-hero-ui-21x9-artB.png" alt="TokenUsageMonitor — transparent AI token usage widget for Windows" width="100%">

# TokenUsageMonitor

**Aggregated AI token usage on a transparent always-on-top Windows widget**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT_OR_Apache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-lightgrey.svg)](#build)
[![Built with Tauri 2](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB.svg)](https://tauri.app)

中文文档：[README.md](README.md)

</div>

---

TokenUsageMonitor is a lightweight desktop widget that aggregates AI provider
quotas, real token consumption from local AI coding tools, and a local model
routing proxy into a single 400×680 transparent, always-on-top panel — so you
always know how much quota is left and how fast it is burning, without opening
a single vendor console.

> The application UI is currently Chinese-only. The docs below are in English,
> but expect Chinese text on screen.

## Features

### Multi-provider quota monitoring (multi-account)

Add several independent accounts per provider, each with its own label,
accent color and enable switch:

| Provider | Data | Auth |
|---|---|---|
| DeepSeek | balance | API key |
| Kimi (Moonshot) | monthly balance | API key |
| MiniMax | Token Plan quota | API key |
| Volcengine Ark | AgentPlan quota / API inference usage | AK/SK (HMAC signed) |
| OpenAI | subscription & billing usage | API key |
| xAI | usage | API key |
| ChatGPT / Codex | subscription usage | local `~/.codex/auth.json` |
| opencode Zen | usage | API key |

### Local AI tool usage ledger

Scans local session logs of **Claude Code, Codex, ZCode, DeepSeek Harness
(dsh), Cherry Studio, MiniMax Code and Hermes** and folds their real token
consumption (input / cache read / output) into one daily ledger, viewable by
calendar, trend and model.

> Only usage statistics fields are read — **conversation content is never
> read**. Relocated dot-directories (tools moved to another drive) are
> supported and resolved by freshness to avoid double counting.

### TokenRouter — local model routing proxy

A reverse proxy bound to `127.0.0.1:43211`, speaking both Anthropic and
OpenAI-compatible protocols (`/v1/messages`, `/v1/chat/completions`,
`/v1/responses`):

- Point Claude Code / Codex / any OpenAI-compatible client at the local port;
- Each route picks a line via **four-layer evaluation**: proactive low-quota
  warning → hard wall → passive observation (429/404…) → recovery probe;
- Client credentials are **never forwarded upstream** — the router injects
  the candidate account's own key;
- Every routed request is metered into the same ledger.

### Multi-device aggregation

Machines on the same LAN can aggregate usage automatically (hub / agent /
lan roles + mDNS discovery + shared-secret auth), see
[Sync](#sync) below.

### Desktop interaction

Transparent always-on-top, edge snapping, collapse-to-pill, usage heatmap,
burn rate with ETA prediction, tiered threshold notifications (default 80%
warn / 95% critical), multi-currency cost estimates with overridable rates.

## Privacy & data

Design baseline: **usage data stays on your machine. Zero telemetry.**

- **No telemetry**: no analytics, no crash reporting, no tracking SDK.
- **Data location**: everything lives in `%APPDATA%\com.tokenmonitor.app\`
  (SQLite + config.toml). Uninstall and it is gone.
- **Credentials**: provider API keys are stored in the Windows Credential
  Manager (DPAPI-encrypted).
- **Logs**: tracing output goes to stderr only (development diagnostics) —
  never written to disk, never contains credentials.

The only network traffic the app ever initiates:

| Purpose | Target |
|---|---|
| Usage queries for the providers you configure | vendor official APIs |
| Upstream of the local routing proxy | base URLs you configure |
| Multi-device sync (opt-in) | LAN peers you run or discover |
| Exchange rates for cost estimates | open.er-api.com (free, keyless, 24h cache) |
| Update check (off by default, opt-in) | api.github.com |

Three things you should know:

1. **Local scan scope**: the app reads session log files of the 7 tools
   listed above (usage fields only). On Windows it also enumerates WSL
   distributions and scans matching directories inside them (no separate
   toggle; silently skipped when WSL is absent).
2. **The hub shared secret and router tokens are stored in plaintext** in
   config.toml (provider keys are not — they sit in DPAPI-protected
   credential storage). Do not share your config.toml.
3. **Multi-device sync runs plaintext HTTP over the LAN**, authenticated by
   a Bearer shared secret (auto-generated 128-bit when you enable it; you
   can clear it to disable auth). Do not enable it on untrusted networks.

See [SECURITY.md](SECURITY.md) for details.

## Build

Prerequisites (Windows 10/11):

- **Node.js** ≥ 20 and npm
- **Rust** stable — `rust-toolchain.toml` pins the
  `stable-x86_64-pc-windows-gnu` toolchain (no MSVC Build Tools needed);
  rustup installs it automatically. Linking uses `rust-lld` with
  self-contained import libs (`src-tauri/.cargo/config.toml`); you need
  LLVM-MinGW / MinGW-w64 on the system
- **WebView2** runtime (preinstalled on Win11; the installer bundles a
  bootstrapper)
- NSIS is fetched automatically by Tauri at packaging time

```bash
npm install

# Develop
npm run tauri:dev

# Package (NSIS installer + MSI)
npm run tauri:build
```

Tests and static checks:

```bash
npm test            # vitest
npx svelte-check    # frontend type check
cd src-tauri && cargo test   # Rust unit + integration tests
```

## Sync

Settings → Network:

- **hub**: listen on `0.0.0.0:43210` and accept reports from other devices;
- **agent**: push this machine's usage summary (aggregate numbers only —
  no credentials, no settings) to a hub every 30 s;
- **lan**: hub + mDNS auto-discovery of same-subnet peers, full mesh;
- **off**: fully offline (default).

## Acknowledgements

Design and implementation drew inspiration from:

- [qunqin24/Pulse](https://github.com/qunqin24/Pulse) — macOS edge quota
  monitor; the split-pane settings layout follows its design language
- [Javis603/token-monitor](https://github.com/Javis603/token-monitor) and
  [junhoyeo/tokscale](https://github.com/junhoyeo/tokscale) — references
  for local tool usage collection and ledger design
- [Tauri](https://tauri.app), [Svelte](https://svelte.dev)

## Disclaimer

- This is an independently developed third-party tool, **not affiliated
  with or endorsed by any of the listed AI vendors**. Vendor names and
  logos in the UI are used solely to identify the corresponding data
  source.
- Quota/usage numbers come from vendor APIs and local session log parsing;
  they are informational only — always defer to the official consoles.

## License

Dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) (SPDX: `MIT OR Apache-2.0`), at your option.
