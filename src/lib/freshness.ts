/** Freshness buckets for the per-card "data as of" badge.
 *
 * A card renders whatever the last poll returned, so it can sit on screen for
 * many minutes showing numbers that were true when they arrived. The error
 * badge only appears on the *next* poll failure, which may be minutes away - in
 * the meantime the user cannot tell "50% used" from "50% used twenty minutes
 * ago while the provider has been unreachable since". The badge closes that
 * gap.
 *
 * Buckets are expressed as multiples of the poll interval rather than absolute
 * seconds: at a 30s interval three minutes is already very stale, and at a ten
 * minute interval it is not. The buckets are deliberately coarse - this is a
 * nudge, not a stopwatch.
 */

import { tFor, type Locale } from "./i18n/dicts";

export type FreshnessTone = "fresh" | "recent" | "stale" | "expired";

/** Floor on the period used for bucketing, so a misconfigured 1s interval
 *  cannot collapse every boundary onto the current instant. */
const MIN_PERIOD_SEC = 30;

/** Bucket a data age (ms) against the poll period (seconds). */
export function freshnessTone(ageMs: number, intervalSec: number): FreshnessTone {
  const periodMs = Math.max(intervalSec, MIN_PERIOD_SEC) * 1000;
  if (ageMs < periodMs * 1.5) return "fresh";
  if (ageMs < periodMs * 3) return "recent";
  if (ageMs < periodMs * 6) return "stale";
  return "expired";
}

/** Human-readable age. Kept separate from the tone so the wording stays in one
 *  place. Negative ages (clock skew, restored session) read as "just now".
 *  `locale` 由调用方传入（响应式：组件里写 $locale，语言切换后重算）；
 *  缺省中文，供纯逻辑调用方兜底。 */
export function freshnessLabel(ageMs: number, locale: Locale = "zh-CN"): string {
  const s = Math.max(0, Math.floor(ageMs / 1000));
  if (s < 10) return tFor(locale, "freshness.justNow");
  if (s < 60) return tFor(locale, "freshness.secondsAgo", { n: s });
  const m = Math.floor(s / 60);
  if (m < 60) return tFor(locale, "freshness.minutesAgo", { n: m });
  const h = Math.floor(m / 60);
  if (h < 24) return tFor(locale, "freshness.hoursAgo", { n: h });
  return tFor(locale, "freshness.daysAgo", { n: Math.floor(h / 24) });
}
