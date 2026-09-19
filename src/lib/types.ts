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

export interface ProviderInfo {
  id: string;
  display_name: string;
  auth_kind: "bearer_key" | "access_key_secret";
  /** True if credentials are stored in the OS credential manager. */
  has_credentials: boolean;
  /** True if the user has enabled this provider in Settings. */
  enabled: boolean;
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
  enabled_providers: string[];
  /** Polling interval seconds. 0 = use per-provider default (5 min). */
  poll_interval_seconds: number;
  /** Persisted dashboard position. Restored on startup. */
  dashboard_x: number | null;
  dashboard_y: number | null;
  compact_mode: boolean;
  autostart_hint_shown: boolean;
  /** Close button hides to tray instead of quitting (B4). */
  close_to_tray: boolean;
  /** Notification thresholds in percent (B4 settings UI edits these). */
  notify_warn_percent: number;
  notify_crit_percent: number;
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

export type WindowKey = "five_hour" | "daily" | "weekly" | "monthly";

/**
 * Mirror of the backend `most_critical_window` rule (scheduler.rs):
 * among windows with quota > 0, pick the one with the greatest used percent.
 * Ties keep the fixed diff order (five_hour -> daily -> weekly -> monthly).
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
      if (p > bestPct) {
        bestPct = p;
        best = { key, window: win };
      }
    }
  }
  return best;
}
