// TypeScript types mirroring the Rust `UsageSnapshot` family.
// Keep field names in sync with `src-tauri/src/providers/mod.rs` snake_case
// serde serialization.

export type UsageUnit = "tokens" | "afp" | "cny" | "credits" | "usd" | "percent";

export interface WindowUsage {
  used: number;
  quota: number;
  unit: UsageUnit;
  reset_at?: string; // ISO8601 UTC
  over_quota: boolean;
  /** Provenance of the reported value. Mirrors Rust `CostSource`. */
  cost_source?: CostSource;
  /** Detailed token split (input/cache/output) when the provider reports it. */
  tokens?: TokenBreakdown;
}

/** Per-window token usage split. Mirrors Rust `TokenBreakdown`. */
export interface TokenBreakdown {
  /** Input tokens consumed (cache-read reuse counted at input rate). */
  input: number;
  /** Input tokens served from the provider's prompt cache ("cache hit"). */
  cache_read: number;
  /** Output (completion) tokens generated. */
  output: number;
  /** Which model this breakdown pertains to, when known. */
  model_id?: string;
}

/** Where a usage/cost figure comes from. `estimated` lets UI label it as an
 * approximation rather than presenting it as exact. */
export type CostSource = "unknown" | "provider_reported" | "estimated";

export interface BalanceInfo {
  total: number;
  granted: number;
  topped_up: number;
  currency: string;
  is_available: boolean;
}

export interface UsageWindows {
  five_hour?: WindowUsage;
  daily?: WindowUsage;
  weekly?: WindowUsage;
  monthly?: WindowUsage;
  balance?: BalanceInfo;
}

export interface HeatmapCell {
  date: string; // YYYY-MM-DD
  value: number;
  unit: UsageUnit;
}

/** Mirrors Rust `ipc::DeviceInfo` (B8 device view, multi-device sync later). */
export interface DeviceInfo {
  device_id: string;
  hostname: string;
  os: string;
  arch: string;
  version: string;
  pid: number;
}

/** Mirrors Rust `hub::HubDay`: one day of a device's token series. */
export interface HubDay {
  date: string; // YYYY-MM-DD
  total: number;
}

/** Mirrors Rust `hub::HubDevice`: one device's report to the hub. */
export interface HubDevice {
  device_id: string;
  hostname: string;
  os: string;
  arch: string;
  version: string;
  reported_at: string;
  tool_tokens: number;
  provider_count: number;
  tool_count: number;
  daily: HubDay[];
}

/** Mirrors Rust `ipc::HubDevicesResult`: device list + optional refresh warning. */
export interface HubDevicesResult {
  devices: HubDevice[];
  warning: string | null;
}

export interface UsageSnapshot {
  provider_id: string;
  provider_display_name: string;
  plan_tier?: string;
  timestamp: string; // ISO8601 UTC
  windows: UsageWindows;
  heatmap?: HeatmapCell[];
}

/** Mirrors Rust `BurnInfo` (providers/mod.rs). */
export interface BurnInfo {
  rate_per_min: number;
  unit: UsageUnit;
  /** Seconds until quota exhaustion at the current rate; null = unknown. */
  eta_seconds: number | null;
}

/** Payload of the `usage-updated` event. Mirrors Rust `UsageUpdate`. */
export interface UsageUpdate {
  snapshot: UsageSnapshot;
  burn: BurnInfo | null;
  active: boolean;
}

/** Mirrors Rust `AccountMeta` (providers/mod.rs). A user-configured account
 * bound to one provider kind. `instance_id` is the stable key used everywhere
 * the old `provider_id` singleton was: keyring entry, snapshot provider_id,
 * heatmap keyspace and the frontend card key. */
export interface AccountMeta {
  instance_id: string;
  provider_kind: string;
  label: string;
  /** Per-account accent colour, `#RRGGBB`. */
  accent_color: string;
  enabled: boolean;
  note?: string;
}

/** Mirrors Rust `PresetSubMode` (providers/mod.rs): a billable mode selectable
 * under a multi-mode preset. Maps to an account `kind`. */
export interface PresetSubMode {
  kind: string;
  label: string;
  note?: string;
  /** true = skeleton / not-yet-implemented (UI greys it out + tags it). */
  limited?: boolean;
}

/** Mirrors Rust `Preset` (providers/mod.rs): a built-in provider the user can
 * add an account from. When `sub_modes` is present, the UI shows a chooser;
 * otherwise the preset adds directly. */
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

/** Mirrors Rust `AccountWithInfo` (ipc.rs): an account plus its credential
 * and registry state for the Settings UI. */
export interface AccountWithInfo extends AccountMeta {
  has_credentials: boolean;
  /** Whether a polling task is live for this account right now. */
  live: boolean;
}

/** Mirrors Rust `ProviderCatalog` (ipc.rs): the built-in presets a user can
 * add + the accounts they've already created. */
export interface ProviderCatalog {
  presets: Preset[];
  accounts: AccountWithInfo[];
}

export interface ProviderState {
  snapshot: UsageSnapshot | null;
  last_error: ProviderError | null;
  last_updated_at: string | null;
}

export interface ProviderError {
  Network?: { message: string };
  Auth?: { message: string };
  Parse?: { message: string };
  RateLimited?: Record<string, never>;
  NotConfigured?: Record<string, never>;
  Internal?: { message: string };
}

/** Mirrors Rust `LocalDay` (local/mod.rs): one day of aggregated tool usage. */
export interface LocalDay {
  date: string; // YYYY-MM-DD (local)
  input: number;
  cache_read: number;
  output: number;
  total: number;
}

/** Mirrors Rust `LocalModelUsage` (local/mod.rs): per-model usage for a tool. */
export interface LocalModelUsage {
  model: string;
  total_tokens: number;
  /** Provider-reported cost when the tool records it (0 if unknown). */
  cost: number;
  currency: string;
  /** True when `cost` was estimated from the pricing table (not provider-reported). */
  cost_estimated: boolean;
  /** Per-day token series (ascending by date). */
  daily: LocalDay[];
}

/** Mirrors Rust `LocalToolReport` (local/mod.rs). */
export interface LocalToolReport {
  id: string;
  name: string;
  daily: LocalDay[];
  total_tokens: number;
  session_count: number;
  project_count: number;
  scanned_at: string; // ISO8601 UTC
  models: LocalModelUsage[];
}

/** Mirrors Rust `LocalToolsPayload`. */
export interface LocalToolsPayload {
  tools: LocalToolReport[];
  sessions_parsed: number;
}

export type WindowMode = "dashboard" | "compact";

/** 贴边把手所在的水平边。 */
export type PeekSide = "left" | "right";

/** 贴边把手的三种会话状态：浮动（无把手）/ 贴边但胶囊已滑入 / 贴边且把手可唤回。 */
export type PeekState = "floating" | "revealed" | "docked";

/** User-facing settings persisted in config.toml. Mirrors Rust `Settings`. */
export interface Settings {
  /** Legacy enable list (pre multi-account). Seeded into `accounts` on first
   * load; kept for backward compatibility. */
  enabled_providers: string[];
  /** User-configured accounts (the source of truth since multi-account). */
  accounts: AccountMeta[];
  /** Polling interval seconds. 0 = use per-provider default (5 min). */
  poll_interval_seconds: number;
  /** Persisted dashboard position. Restored on startup. */
  dashboard_x: number | null;
  dashboard_y: number | null;
  compact_mode: boolean;
  /** Whether to launch automatically at system boot (Windows HKCU Run key). */
  autostart: boolean;
  /** Close button hides to tray instead of quitting (B4). */
  close_to_tray: boolean;
  /** Whether to fire OS notifications on threshold breach. */
  notify_enabled: boolean;
  /** Notification thresholds in percent (B4 settings UI edits these). */
  notify_warn_percent: number;
  notify_crit_percent: number;
  /** Window the ring gauges show: "auto" | "five_hour" | "daily" | "weekly" |
   *  | "monthly". "auto" = each provider's most critical window. */
  ring_window: string;
  /** Snap the dashboard to screen edges when dragged near one. */
  edge_snap: boolean;
  /** Ring gauge display mode: false = show `used`; true = show `remaining`
   * (countdown). */
  countdown_mode: boolean;
  /** Currency used to render estimated costs: "auto" | "USD" | "CNY" | ... */
  display_currency: string;
  /** Multi-device hub role: "off" | "hub" | "agent". */
  hub_mode: string;
  /** Local port the hub listener binds to when `hub_mode == "hub"`. */
  hub_port: number;
  /** Remote hub base URL to report to when `hub_mode == "agent"`. */
  hub_base: string;
  /** Whether an agent actually reports its own usage. */
  report_on: boolean;
  /** Shared secret the hub requires (Bearer). Empty = no auth. */
  hub_token: string;
  /** Whether the user has ever explicitly set `hub_token` (even to "").
   *  Absent in older configs, which means "never set" → the backend mints a
   *  secret on first hub start. Saved as `true` on every settings write. */
  hub_token_configured?: boolean;
  /** 用户手动覆盖的汇率（币种代码 → 每 1 USD 兑该币种数值）。非法值后端忽略。 */
  rate_overrides: Record<string, number>;
  /** Optional outbound proxy (http/https/socks5). Null/empty/blank = direct.
   * Changing this rebuilds the shared HTTP client and the provider registry. */
  proxy_url?: string | null;
}

/** One effective exchange-rate row (backend `exchange::RateRow`). `source`:
 * "override" = manual value, "live" = fetched from the rate source,
 * "default" = built-in offline fallback. */
export interface RateRow {
  code: string;
  rate: number;
  /** ISO8601 of when this value was captured; "" for override/default rows. */
  updated_at: string;
  source: "override" | "live" | "default";
}

/** Full snapshot of effective rates (backend `exchange::RatesSnapshot`). */
export interface RatesSnapshot {
  rates: RateRow[];
  /** ISO8601 of the most recent successful network fetch; "" when never. */
  fetched_at: string;
  /** Set when the refresh failed (offline etc.) — snapshot is still valid. */
  warning: string | null;
}

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

/** Short display name for compact surfaces (capsule pill / quick rows).
 * `provider_id` is an `instance_id` shaped like `"<kind>-<ts>-<n>"`, so the
 * kind is the whole `-`-free leading segment (`minimax_api`, `minimax`, ...).
 * Falls back to the account label. */
const SHORT_KIND_NAMES: Record<string, string> = {
  minimax: "MiniMax",
  minimax_api: "MiniMax API",
  deepseek: "DeepSeek",
  volcengine: "Volcano",
  volcengine_api: "Volcano API",
  openai: "OpenAI",
  opencode: "OpenCode",
  gemini: "Gemini",
  anthropic: "Anthropic",
  qwen: "Qwen",
  kimi: "Kimi",
  kimi_global: "Kimi Global",
  xai: "xAI",
  codex: "Codex",
  doubao: "豆包",
  spark: "Spark",
  xiaomi_plan: "MiMo Plan",
  xiaomi_api: "MiMo API",
};

export function providerShortName(providerId: string, displayName: string): string {
  // kind never contains `-`, so the leading segment is the full kind (even
  // when it contains `_` like `minimax_api`).
  const [kind] = providerId.split("-");
  return SHORT_KIND_NAMES[kind] ?? displayName;
}

/** Parse a "#RRGGBB" hex color into a "r,g,b" CSS triplet (so callers can use
 * `rgba(var(--x-rgb), a)`). Returns null when the string isn't 6-digit hex. */
export function hexToRgb(hex: string): string | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return `${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}`;
}

/** Helper to format a usage value with the right unit suffix. */
export function formatUsage(value: number, unit: UsageUnit): string {
  if (unit === "usd") {
    return `$${value.toFixed(2)}`;
  }
  if (unit === "cny") {
    return `¥${value.toFixed(2)}`;
  }
  if (unit === "percent") {
    return `${value.toFixed(0)}%`;
  }
  // Tokens / AFP / Credits - humanize large numbers.
  if (value >= 1_000_000_000) return `${(value / 1e9).toFixed(2)}B`;
  if (value >= 1_000_000) return `${(value / 1e6).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1e3).toFixed(1)}K`;
  return value.toFixed(0);
}

export function unitLabel(unit: UsageUnit): string {
  switch (unit) {
    case "tokens":
      return "tokens";
    case "afp":
      return "AFP";
    case "cny":
      return "¥";
    case "usd":
      return "$";
    case "credits":
      return "credits";
    case "percent":
      return "%";
  }
}

export function percent(w: WindowUsage): number {
  if (w.quota <= 0) return 0;
  return Math.min(1, Math.max(0, w.used / w.quota));
}

export function remainingPercent(snap: UsageSnapshot): number {
  const windows = [
    snap.windows.five_hour,
    snap.windows.daily,
    snap.windows.weekly,
    snap.windows.monthly,
  ].filter((w): w is WindowUsage => Boolean(w));
  if (windows.length === 0) return 1;
  return Math.min(
    ...windows.map((w) => 1 - percent(w)),
  );
}

/**
 * Resolve the ring gauge for a provider according to the user's `ring_window`
 * setting. "auto" → the provider's most critical (highest-used) quota window,
 * so all providers report their single most urgent number; explicit keys show
 * that specific window. Falls back to the aggregate remaining % when the
 * requested window has no quota.
 */
export function ringWindowRemaining(
  snap: UsageSnapshot,
  ringWindow: string,
): number {
  if (ringWindow === "auto") {
    const c = mostCriticalWindow(snap);
    return c ? 1 - percent(c.window) : remainingPercent(snap);
  }
  const w = snap.windows[ringWindow as WindowKey];
  return w && w.quota > 0 ? 1 - percent(w) : remainingPercent(snap);
}

export type WindowKey = "five_hour" | "daily" | "weekly" | "monthly";

/**
 * Mirror of the backend `most_critical_window` rule (scheduler.rs):
 * among windows with quota > 0, pick the one with the greatest used percent.
 * Ties resolve to the later window in diff order, matching Rust
 * `Iterator::max_by`, which returns the last maximum (monthly direction).
 */
export function mostCriticalWindow(
  snap: UsageSnapshot,
): { key: WindowKey; window: WindowUsage } | null {
  const order: WindowKey[] = ["five_hour", "daily", "weekly", "monthly"];
  let best: { key: WindowKey; window: WindowUsage } | null = null;
  let bestPct = -1;
  for (const key of order) {
    const win = snap.windows[key];
    if (win && win.quota > 0) {
      const p = percent(win);
      if (p >= bestPct) {
        bestPct = p;
        best = { key, window: win };
      }
    }
  }
  return best;
}

/** 是否为按量付费（非配额制）：没有任何 quota>0 的窗口。 */
export function isPayAsYouGo(snap: UsageSnapshot): boolean {
  return mostCriticalWindow(snap) === null;
}

/**
 * 按量付费 provider 的圆环/数值标签。
 * countdown=true → 账户余额；false → 当月消费。
 * 配额制 provider 或缺少对应数据时返回 null（调用方回退到原百分比）。
 */
export function payAsYouGoLabel(
  snap: UsageSnapshot,
  countdown: boolean,
): string | null {
  if (!isPayAsYouGo(snap)) return null;
  if (countdown) {
    const b = snap.windows.balance;
    if (b && b.is_available && b.total != null) {
      const unit: UsageUnit = b.currency?.toLowerCase() === "cny" ? "cny" : "usd";
      if (unit === "cny" || unit === "usd") return formatUsage(b.total, unit);
    }
  } else {
    const m = snap.windows.monthly;
    if (m && m.used > 0) return formatUsage(m.used, m.unit);
  }
  return null;
}
