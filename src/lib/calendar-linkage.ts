// 日历 ↔ Provider 卡片联动的**唯一数据源**。
//
// 存在的理由：2026-10 之前 App / DetailCard 各维护一份 kind→序列映射表，
// 内容还不一致（App 有 deepseek，DetailCard 没有），且 App 那份映射到工具 id
// 而下拉已改用 provider key —— 两者在同一 `p:` 命名空间里对不上，点击卡片后
// 日历恒为空。单一数据源 + 纯函数是这类漂移的唯一根治办法。
//
// 全部为纯函数：不 import api.ts，不发 IPC，可直接单测。

import { normalizeKind } from "./brand-glyphs";

/** 某个 provider kind 在统一账本里对应的日序列。 */
export type LedgerSeries =
  /** 跨工具归因的 token 序列：按 provider key 跨工具聚合。 */
  | { mode: "cross-tool"; providerKey: string }
  /** 真实服务端日账：单位非 token（火山 AFP / OpenAI USD / xAI USD）。 */
  | { mode: "account-daily"; providerId: string }
  /** 差分类：无按日数据，联动不生效（不猜、不静默兜底）。 */
  | null;

/** kind → 账本里的 provider key。key 空间必须与 `providerForModel()` 的返回值一致。 */
const KIND_TO_PROVIDER_KEY: Record<string, string> = {
  minimax: "minimax",
  deepseek: "deepseek",
  anthropic: "anthropic",
  gemini: "gemini",
  qwen: "qwen",
  kimi: "kimi",
  doubao: "doubao",
  glm: "glm",
  xai: "xai",
  openai: "openai",
  volcengine: "volcengine",
  // 品牌注册表里有 spark（types.ts 的 SHORT_KIND_NAMES / brand-glyphs 均已登记），
  // 且 PREFIX_RULES 含 ["spark","spark"]，即 `p:spark` 序列真实可达。漏掉它会让
  // Spark 卡片点击后无联动。此表须覆盖**全部**可产出 p: key 的 kind（"other"
  // 除外——它是归因兜底桶，不对应任何卡片）。
  spark: "spark",
};

/** 有真实服务端日账的 kind（双写进 usage_daily，kind='provider'）。 */
const ACCOUNT_DAILY_KINDS = new Set(["volcengine", "openai", "xai"]);

/**
 * provider kind → 它该看哪条账本序列。
 *
 * `kind` 来自 `instance_id.split("-")[0]`，带子模式后缀（如 `volcengine_api`），
 * 故先 `normalizeKind` 归一再判定——`brand-glyphs.ts:68` 已有该函数，
 * 含 `kimi_global → kimi` 特判，此前这两处集合没用它是遗漏。
 */
export function ledgerSeriesForKind(kind: string, providerId: string): LedgerSeries {
  const base = normalizeKind(kind);
  if (ACCOUNT_DAILY_KINDS.has(base)) {
    return { mode: "account-daily", providerId };
  }
  const providerKey = KIND_TO_PROVIDER_KEY[base];
  return providerKey ? { mode: "cross-tool", providerKey } : null;
}

/**
 * provider kind → 日历高亮用的 provider key；不可高亮返回 null。
 *
 * 与 `ledgerSeriesForKind` 的区别：账户日账虽有按日数据，但单位与本机工具
 * token 不同量纲，混进同一张日历的总量会犯「跨单位相加」的错误（历史教训见
 * docs/usage-ledger.md）。故日历恒为工具 token 总量，只有跨工具 kind 可高亮。
 */
export function highlightKeyForKind(kind: string): string | null {
  const base = normalizeKind(kind);
  if (ACCOUNT_DAILY_KINDS.has(base)) return null;
  return KIND_TO_PROVIDER_KEY[base] ?? null;
}
