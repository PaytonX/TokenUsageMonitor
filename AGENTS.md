# AGENTS.md

Instructions for AI coding agents working in this repository.

## Project

**TokenUsageMonitor** — a lightweight, transparent desktop widget for monitoring
AI token usage. Tauri v2 shell with a Svelte 5 + Vite frontend (`src/`) and a
Rust backend (`src-tauri/`).

## Commands

```bash
npm install
npm run tauri:dev     # run the app
npm run tauri:build   # produce a release build

npm test                     # vitest
cd src-tauri && cargo test   # Rust unit + integration tests
npx svelte-check             # frontend type check (expect 0 error 0 warning)
```

All three must pass before a PR. See `CONTRIBUTING.md` for the full
conventions: comment language, tracing targets, provider implementation, and
credential handling rules.

## Agent skills

### Issue tracker

Issues live as markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary, recorded as a `Status:` line in each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: one root `GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.