// 日历 ↔ Provider 卡片联动的**唯一数据源**。
//
// 存在的理由：2026-10 之前 App / DetailCard 各维护一份 kind→序列映射表，
// 内容还不一致（App 有 deepseek，DetailCard 没有），且 App 那份映射到工具 id
// 而下拉已改用 provider key —— 两者在同一 `p:` 命名空间里对不上，点击卡片后
// 日历恒为空。单一数据源 + 纯函数是这类漂移的唯一根治办法。
//
// 全部为纯函数：不 import api.ts，不发 IPC，可直接单测。

import { normalizeKind } from "./brand-glyphs";
import type { UsageDailyRow } from "./api";
import { providerForModel } from "./model-provider";

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

/** 某一天的用量拆解。 */
export interface DayBreakdown {
  /** 当日全部本机工具 token 总量。 */
  total: number;
  /** 当日按 provider key 的拆解。 */
  byProvider: Record<string, number>;
}

/** 某个 provider 在窗口内的口径统计，用于日历统计行的第二行。 */
export interface FocusStats {
  /** 窗口内有该 provider 用量的天数。 */
  days: number;
  sum: number;
  peak: number;
  /** 占窗口总量的百分比（0-100，整数）。 */
  share: number;
}

/**
 * 账本行 → 按日、按 Provider 拆解。
 *
 * 关键口径（两行互不重叠，这是修掉旧实现翻倍 bug 的核心）：
 * - `model === ""` 的**来源总量行** → 只计入 `total`；
 * - `model !== ""` 的**分模型行** → 只计入 `byProvider`。
 *
 * 理由：provider 归因信息只存在于模型行（总量行没有模型名）；而总量必须用
 * 总量行，否则分模型行求和会因未覆盖模型而漏计。旧实现（HeatmapGrid.svelte:57-65）
 * 对 `p:<key>` 不按 model 过滤，两类行同时累加 → 翻倍。
 *
 * 仅消费 `kind === 'tool'` 行：provider 账本行单位非 token（火山 AFP / OpenAI
 * USD），混算即错——这一层过滤是承重的，不是装饰。
 *
 * ⚠️ 例外：旧数据迁移（`storage.rs:354` `migrate_legacy_usage_ledger`）写的
 * 是**单形态**行——minimax-code 只有总量行没有模型行，hermes 只有模型行没有
 * 总量行。因此「分模型合计 == 来源总量」这个不变式对迁移来的旧行**不成立**：
 * 工具已卸载时其 token 会长期滞留在 total 里而无任何 provider 归因，hermes
 * 独有日则会让分模型合计 > 总量。仍安装的工具在首次全量回放后自愈。
 * 结论：正常账本满足不变式；迁移遗留行不满足，属已知数据层瑕疵，不在前端修。
 */
export function buildLedgerBreakdown(
  rows: UsageDailyRow[],
): Map<string, DayBreakdown> {
  const out = new Map<string, DayBreakdown>();
  const slot = (date: string): DayBreakdown => {
    let e = out.get(date);
    if (!e) {
      e = { total: 0, byProvider: {} };
      out.set(date, e);
    }
    return e;
  };
  for (const r of rows) {
    if (r.kind !== "tool") continue;
    const e = slot(r.date);
    if (r.model === "") {
      e.total += r.total;
    } else {
      const key = providerForModel(r.model, r.source);
      e.byProvider[key] = (e.byProvider[key] ?? 0) + r.total;
    }
  }
  return out;
}

/** 某 provider 在 `windowDates` 窗口内的口径统计。 */
export function focusStats(
  byDate: Map<string, DayBreakdown>,
  windowDates: string[],
  providerKey: string,
  windowTotal: number,
): FocusStats {
  let days = 0;
  let sum = 0;
  let peak = 0;
  for (const d of windowDates) {
    const v = byDate.get(d)?.byProvider[providerKey] ?? 0;
    if (v > 0) days += 1;
    sum += v;
    if (v > peak) peak = v;
  }
  return {
    days,
    sum,
    peak,
    // 夹到 100：迁移遗留行（hermes 只有模型行没有总量行）会让分模型合计超过
    // 总量，算出 100% 以上的"占比"。那是数据层瑕疵，不该以百分比外溢到 UI。
    share: windowTotal > 0 ? Math.min(100, Math.round((sum / windowTotal) * 100)) : 0,
  };
}
