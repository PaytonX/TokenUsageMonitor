// Multi-currency cost formatting + an offline token→USD estimator.
//
// C6 ("成本多币种换算"): a tiny frontend twin of the backend `pricing.rs`.
// - `formatCost` renders a USD cost in a chosen currency (USD/CNY/TWD/HKD/...),
//   with an optional "≈" (approx) marker via `approximate`.
// - `estimateCostUsd` turns a `TokenBreakdown` into an estimated USD figure via
//   an in-repo price table. It mirrors the tokscale safeguard: an unknown or
//   overly-generic model returns `null` rather than guessing, so the UI never
//   renders a wrong number.
//
// Exchange rates start from offline defaults (1 USD → ???) and are then kept
// in sync with the backend (`getExchangeRates` + `rates-updated` events):
// user overrides beat cached live rates, which beat the built-in defaults.
// Network failures are ignored so monitoring keeps working offline (the plan
// flags network dependence as a risk).

import { getExchangeRates, onRatesUpdated } from "./api";
import type { RatesSnapshot } from "./types";

export type Currency = "USD" | "CNY" | "TWD" | "HKD" | "JPY" | "EUR" | "GBP";

export const CURRENCIES: { code: Currency; label: string }[] = [
  { code: "CNY", label: "currency.CNY" },
  { code: "USD", label: "currency.USD" },
  { code: "TWD", label: "currency.TWD" },
  { code: "HKD", label: "currency.HKD" },
  { code: "JPY", label: "currency.JPY" },
  { code: "EUR", label: "currency.EUR" },
  { code: "GBP", label: "currency.GBP" },
];

/** Units of the target currency per 1 USD. Starts from offline defaults and
 * is updated in place by {@link applyRatesSnapshot}; treat as read-only. */
export const USD_RATES: Record<Currency, number> = {
  USD: 1,
  CNY: 7.13,
  TWD: 31.9,
  HKD: 7.79,
  JPY: 142.3,
  EUR: 0.92,
  GBP: 0.79,
};

export function convertUsd(usd: number, to: Currency): number {
  return usd * USD_RATES[to];
}

const CURRENCY_CODES = new Set<string>(
  CURRENCIES.map((c) => c.code),
);

/**
 * Normalize a backend-reported currency string to a known code. Empty or
 * unknown values fall back to "USD" — the backend semantics: every local
 * scanner except cherry (which reads `cost_currency` from its DB) reports USD.
 */
export function normalizeCurrency(raw: string): Currency {
  const code = (raw || "").trim().toUpperCase();
  return CURRENCY_CODES.has(code) ? (code as Currency) : "USD";
}

/** Convert an amount denominated in `from` currency into USD (effective rates). */
export function toUsd(amount: number, from: Currency): number {
  if (from === "USD" || !Number.isFinite(amount)) return amount;
  const rate = USD_RATES[from];
  return rate > 0 ? amount / rate : amount;
}

/**
 * Resolve the display currency for costs. Reads the persisted choice from
 * `localStorage` (mirrored by the Settings UI); "auto" or an unknown value
 * falls back to the default (CNY).
 */
export function displayCurrency(): Currency {
  try {
    const raw = localStorage.getItem("tum.currency");
    if (raw && raw !== "auto" && CURRENCY_CODES.has(raw)) return raw as Currency;
  } catch {
    /* ignore */
  }
  return "CNY";
}

/**
 * Format a USD amount (f64 from the backend) in the selected currency.
 * When `approximate` is true the leading "≈" is added and the value is shown at
 * reduced precision — use it for `cost_source === "estimated"` figures.
 */
export function formatCost(usd: number, currency: Currency, approximate = false): string {
  const v = convertUsd(Math.max(0, usd), currency);
  const symbolMap: Record<Currency, string> = {
    USD: "$",
    CNY: "¥",
    TWD: "NT$",
    HKD: "HK$",
    JPY: "¥",
    EUR: "€",
    GBP: "£",
  };
  const symbol = symbolMap[currency];
  const digits = currency === "JPY" ? 0 : 2;
  const base = `${symbol}${v.toFixed(digits)}`;
  return approximate ? `≈ ${base}` : base;
}

// --- Effective rates: backend snapshot (overrides > cached live > defaults) ---

/**
 * Apply a backend `RatesSnapshot` to the in-memory rate table. Only
 * "override" and "live" rows are written ("default" rows mirror the built-in
 * values above); unknown codes and non-positive / non-finite rates are
 * ignored, and the function never throws.
 */
export function applyRatesSnapshot(snapshot: RatesSnapshot): void {
  for (const row of snapshot?.rates ?? []) {
    if (row.source !== "override" && row.source !== "live") continue;
    if (!CURRENCY_CODES.has(row.code)) continue;
    if (!Number.isFinite(row.rate) || row.rate <= 0) continue;
    USD_RATES[row.code as Currency] = row.rate;
  }
}

// --- Offline token → USD estimator (mirrors backend `pricing::compute_cost`) ---

interface Tier {
  inputPer1m: number;
  outputPer1m: number;
}

/** (model-prefix, tier). First matching prefix wins. Keep in sync with
 * `src-tauri/src/pricing.rs` for the families that matter here. */
const PRICE_TABLE: [string, Tier][] = [
  ["claude-3-7-sonnet", { inputPer1m: 3.0, outputPer1m: 15.0 }],
  ["claude-3-5-sonnet", { inputPer1m: 3.0, outputPer1m: 15.0 }],
  ["claude-3-5-haiku", { inputPer1m: 0.8, outputPer1m: 4.0 }],
  ["claude-sonnet", { inputPer1m: 3.0, outputPer1m: 15.0 }],
  ["claude-haiku", { inputPer1m: 0.8, outputPer1m: 4.0 }],
  ["claude-3-opus", { inputPer1m: 15.0, outputPer1m: 75.0 }],
  ["gpt-5", { inputPer1m: 1.25, outputPer1m: 10.0 }],
  ["gpt-4o", { inputPer1m: 2.5, outputPer1m: 10.0 }],
  ["gpt-4.1", { inputPer1m: 2.0, outputPer1m: 8.0 }],
  ["gpt-4", { inputPer1m: 30.0, outputPer1m: 60.0 }],
  ["o3", { inputPer1m: 2.0, outputPer1m: 8.0 }],
  ["o1", { inputPer1m: 15.0, outputPer1m: 60.0 }],
  ["deepseek-reasoner", { inputPer1m: 0.55, outputPer1m: 2.19 }],
  ["deepseek-chat", { inputPer1m: 0.27, outputPer1m: 1.1 }],
  ["minimax-m3", { inputPer1m: 5.0, outputPer1m: 20.0 }],
  ["minimax", { inputPer1m: 2.0, outputPer1m: 8.0 }],
  ["qwen3", { inputPer1m: 0.6, outputPer1m: 2.4 }],
  ["qwen", { inputPer1m: 0.5, outputPer1m: 2.0 }],
  ["doubao", { inputPer1m: 0.3, outputPer1m: 1.2 }],
  ["grok-4", { inputPer1m: 2.0, outputPer1m: 12.0 }],
  ["grok-3", { inputPer1m: 3.0, outputPer1m: 15.0 }],
  ["mimo-v2.6-pro", { inputPer1m: 0.43, outputPer1m: 0.86 }],
  ["mimo-v2.6-flash", { inputPer1m: 0.14, outputPer1m: 0.29 }],
  ["gemini-2.5-pro", { inputPer1m: 1.25, outputPer1m: 10.0 }],
  ["gemini-2.5-flash", { inputPer1m: 0.3, outputPer1m: 2.5 }],
];

/** Aliases normalised before prefix matching (vendor router spellings). */
const ALIASES: [string, string][] = [
  ["anthropic/claude-3", "claude-3"],
  ["openai/gpt-4o", "gpt-4o"],
  ["openai/gpt-4", "gpt-4"],
  ["openai/gpt-5", "gpt-5"],
  ["deepseek/deepseek-chat", "deepseek-chat"],
  ["deepseek/deepseek-reasoner", "deepseek-reasoner"],
];

/** Bare brand/generic tokens that carry no model identity → refuse to price. */
function isGeneric(model: string): boolean {
  return [
    "claude",
    "gpt",
    "gemini",
    "deepseek",
    "minimax",
    "qwen",
    "mimo",
    "grok",
    "kimi",
    "model",
    "default",
    "router",
  ].includes(model);
}

/**
 * Cache-read tokens bill at a fraction of the input rate. Mirrors the backend's
 * `CACHE_READ_DISCOUNT` — the two must stay in sync or the same breakdown would
 * price differently on each side.
 */
const CACHE_READ_DISCOUNT = 0.1;

/**
 * Master switch for price-based cost estimation — mirrors the backend's
 * `COST_ESTIMATION_ENABLED`; the two must agree or the detail card and the
 * model panel would disagree about the same tokens.
 *
 * Off by default: the price table has to be hand-maintained against vendors who
 * ship new models and reprice old ones, so any estimate silently rots. Costs a
 * tool or provider reports about itself do not go through here and are
 * unaffected.
 */
export const COST_ESTIMATION_ENABLED = false;

/**
 * Estimate the USD cost of a `TokenBreakdown`. Returns `null` when estimation is
 * disabled, or when the model is unknown, missing, or bare (no price identity) —
 * callers then render nothing rather than a guess. Cache reads bill at
 * `CACHE_READ_DISCOUNT` × the input rate, so they are passed separately rather
 * than folded into `input`.
 */
export function estimateCostUsd(tokens: {
  input: number;
  cache_read: number;
  output: number;
  model_id?: string;
}): number | null {
  if (!COST_ESTIMATION_ENABLED) return null;
  const raw = tokens.model_id ?? "";
  let norm = raw.trim().toLowerCase();
  if (norm.includes("(")) norm = norm.slice(0, norm.indexOf("("));
  for (const [alias, canonical] of ALIASES) {
    if (norm === alias || norm.startsWith(alias)) {
      norm = canonical;
      break;
    }
  }
  if (!norm || isGeneric(norm)) return null;
  const pair = PRICE_TABLE.find(([prefix]) => norm.startsWith(prefix));
  if (!pair) return null;
  const [_, tier] = pair;
  const inputCost = tokens.input / 1_000_000 * tier.inputPer1m;
  const cacheCost = (tokens.cache_read ?? 0) / 1_000_000 * tier.inputPer1m * CACHE_READ_DISCOUNT;
  const outputCost = tokens.output / 1_000_000 * tier.outputPer1m;
  return inputCost + cacheCost + outputCost;
}

// Rate bootstrap: preload the effective table once per window, then follow
// backend broadcasts (scheduled refresh / manual refresh from Settings).
// Failures are silent — offline defaults keep monitoring working (plan risk E).
void getExchangeRates()
  .then(applyRatesSnapshot)
  .catch(() => {});
void onRatesUpdated(applyRatesSnapshot).catch(() => {});