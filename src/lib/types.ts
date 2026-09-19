// TypeScript types mirroring the Rust `UsageSnapshot` family.
// Keep field names in sync with `src-tauri/src/providers/mod.rs` snake_case
// serde serialization.

export type UsageUnit = "tokens" | "afp" | "cny" | "credits" | "percent";

export interface WindowUsage {
  used: number;
  quota: number;
  unit: UsageUnit;
  reset_at?: string; // ISO8601 UTC
  over_quota: boolean;
}

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

/** Mirrors Rust `Preset`. A built-in provider kind the user can add accounts
 * from. */
export interface Preset {
  kind: string;
  display_name: string;
  auth_kind: "bearer_key" | "access_key_secret";
  default_accent: string;
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

export type WindowMode = "dashboard" | "compact";

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
  autostart_hint_shown: boolean;
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
}

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

/** Short display name for compact surfaces (capsule pill / quick rows).
 * `provider_id` is an `instance_id` shaped like `"<kind>-<ts>-<n>"`, so the
 * kind is the prefix before the first `-`. Falls back to the account label. */
const SHORT_KIND_NAMES: Record<string, string> = {
  minimax: "MiniMax",
  deepseek: "DeepSeek",
  volcengine: "Volcano",
  openai: "OpenAI",
  gemini: "Gemini",
  anthropic: "Anthropic",
  qwen: "Qwen",
  kimi: "Kimi",
  doubao: "豆包",
  spark: "Spark",
};

export function providerShortName(providerId: string, displayName: string): string {
  const dash = providerId.indexOf("-");
  const kind = dash > 0 ? providerId.slice(0, dash) : providerId;
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

/** Lighten a "#RRGGBB" color by `amt` (0..1) toward white — used to build a
 * two-stop arc gradient from a single per-account accent. */
export function lightenHex(hex: string, amt = 0.35): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const c = (v: number) => Math.round(v + (255 - v) * amt);
  return `#${[c(r), c(g), c(b)]
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("")}`;
}

/** Helper to format a usage value with the right unit suffix. */
export function formatUsage(value: number, unit: UsageUnit): string {
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
