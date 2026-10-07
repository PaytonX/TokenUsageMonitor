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
import {
  collectRouterRowsByDate,
  routedAttribution,
  ROUTED_MODEL,
} from "./router-attribution";

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

/**
 * 有**自己的**服务端日账的 kind（双写进 usage_daily，kind='provider'）。
 *
 * 合并日历后它的职责收窄：不再决定"能不能进日历"（那由行的 `unit` 决定，见
 * buildUnifiedBreakdown），也不再决定"能不能高亮"（见 highlightKeyForKind），
 * 只用来回答"该 kind 是否有一份按官方口径回放的日账可看"——DetailCard 的
 * 近 7 日小柱据此走服务端分支（火山按 tokens、OpenAI/xAI 按 USD 各自呈现）。
 * 注意它与**单位**不是一回事：volcengine 在此集合内且单位是 tokens；
 * openai/xai 在此集合内但单位是 USD。
 */
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
 * 与 {@link providerKeyForKind} 相同——这个别名保留是因为"高亮"是调用侧语义，
 * 而 {@link ledgerSeriesForKind} 答的是另一个问题（该 kind 有没有**自己的**日账
 * 可看）。合并日历时这两者一度分叉（这里曾对账户日账 kind 返回 null），
 * 那是"日历可切换口径"时代的遗留：那时服务端日账与本机 token 不同量纲，
 * 并排切换等于跨单位相加，故把账户日账 kind 排除在总量外。
 * 日历恒为合并口径后不再成立：
 *   - volcengine 的服务端日明细本身就是 tokens，已并入日历 → 应可高亮；
 *   - openai/xai 的服务端日账是 USD，**不**并入日历（见 buildUnifiedBreakdown），
 *     但它们在日历里仍有本机工具归因层（gpt/o1/o3/o4 → openai）→ 也应可高亮。
 * 即：凡是注册表里有的 kind 都能在日历里出现，判据不再是"它有没有账户日账"。
 */
export function highlightKeyForKind(kind: string): string | null {
  return providerKeyForKind(kind);
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
  // 经路由的哨兵行先攒着、不直接归因：真实模型在路由台账里，遍历结束后统一
  // 做替换（见 router-attribution）。按 (date → 工具) 记，是为了差额回退时
  // 仍能落回**各自**工具的兜底 Provider。
  const routed = new Map<string, Map<string, number>>();
  const routerRows = collectRouterRowsByDate(rows);
  for (const r of rows) {
    if (r.kind !== "tool") continue;
    const e = slot(r.date);
    if (r.model === "") {
      e.total += r.total;
    } else if (r.model === ROUTED_MODEL) {
      const bySource = routed.get(r.date) ?? new Map<string, number>();
      bySource.set(r.source, (bySource.get(r.source) ?? 0) + r.total);
      routed.set(r.date, bySource);
    } else {
      const key = providerForModel(r.model, r.source);
      e.byProvider[key] = (e.byProvider[key] ?? 0) + r.total;
    }
  }
  for (const [date, bySource] of routed) {
    const e = slot(date);
    let sentinel = 0;
    for (const v of bySource.values()) sentinel += v;
    const { slices, fallback } = routedAttribution(sentinel, routerRows.get(date) ?? []);
    for (const s of slices) {
      const key = providerForModel(s.model);
      e.byProvider[key] = (e.byProvider[key] ?? 0) + s.total;
    }
    // 路由台账覆盖不足（或没有）：差额按各工具占比回退到工具兜底归因，
    // 保证「分模型合计 == 来源总量」不破，也不用猜。
    if (fallback > 0) {
      for (const [source, v] of bySource) {
        const share = fallback * (v / sentinel);
        const key = providerForModel(ROUTED_MODEL, source);
        e.byProvider[key] = (e.byProvider[key] ?? 0) + share;
      }
    }
  }
  return out;
}

/**
 * provider kind → 账本里的 provider key，**不看**该 kind 是不是账户日账类型。
 *
 * 与 {@link highlightKeyForKind} 的区别：那个函数对 account-daily kind 返回
 * null（它们不进工具 token 口径的总量）；这个是纯命名空间查询，合并日历时
 * 需要用它把 kind='provider' 的服务端行挂到对应的 provider 层上。
 */
export function providerKeyForKind(kind: string): string | null {
  return KIND_TO_PROVIDER_KEY[normalizeKind(kind)] ?? null;
}

/** 该账本行的单位是否计入 token 口径的总量。 */
function isTokenUnit(unit: string): boolean {
  return unit === "tokens" || unit === "";
}

/**
 * 同一**账户**下的本机归因层 → 持有该账户服务端日账的规范层。
 *
 * `providerForModel` 按**模型名**分层，账户 API 按**账户**回账，两者粒度不同，
 * 于是同一个账户的调用会被拆到两个层上。火山账户尤其明显：方舟的模型叫
 * `ark-*`（→ volcengine 层），豆包的模型叫 `doubao-*`（→ doubao 层），可它们
 * 都是火山账户下的调用。本机没有配过其它能提供 doubao 模型的 provider，
 * 所以 doubao 层实际就是火山账户的一部分——官方日账自然覆盖它。
 *
 * 表里没列的层不合并：没有服务端日账的 provider（glm/kimi/minimax…）保持
 * 本机归因原样。
 */
const ACCOUNT_ABSORBED_LAYERS: Record<string, string[]> = {
  volcengine: ["doubao"],
};

/** 某个规范层要吃掉哪些本机归因层（含自身）。 */
function absorbedLayers(canonical: string): string[] {
  return [canonical, ...(ACCOUNT_ABSORBED_LAYERS[canonical] ?? [])];
}

/**
 * 统一日账：**本机工具用量 + 服务端日账合并成一个日历，服务端为准**。
 *
 * 合并规则（每个账户独立判定）：
 * - 该账户当天有服务端日账 → 层值 = 服务端值（官方口径优先，本机归因被吸收）
 * - 否则 → 层值 = 本机跨工具归因值
 * 总量 = 本机来源总量行之和 + Σ(有服务端数据的账户的「服务端值 − 被吸收的本地值之和」)
 * 即**替换**而非**相加**，所以同一账户两边都有记录时不会双计。
 *
 * 单位为非 token 的服务端行（OpenAI/xAI 的 USD、Kimi 的 CNY）不并入——那是
 * 另一个量纲，折进 token 总量就是「跨单位相加」（docs/usage-ledger.md 明令禁止）。
 * 它们只在 DetailCard 的近 7 日小柱里按各自单位单独呈现，不影响日历的总量、
 * 色阶与高亮。
 */
export function buildUnifiedBreakdown(
  rows: UsageDailyRow[],
): Map<string, DayBreakdown> {
  const out = buildLedgerBreakdown(rows);
  // 服务端行按 (规范层, date) 归集。别名层不做反向映射：若将来豆包自己出了
  // 服务端日账，那**是另一个账户**，不该并进火山。
  const server = new Map<string, Map<string, number>>();
  for (const r of rows) {
    if (r.kind !== "provider" || !isTokenUnit(r.unit)) continue;
    const key = providerKeyForKind(r.source.split("-")[0]);
    if (!key) continue;
    const per = server.get(key) ?? new Map<string, number>();
    per.set(r.date, (per.get(r.date) ?? 0) + r.total);
    server.set(key, per);
  }
  if (server.size === 0) return out;

  const slot = (date: string): DayBreakdown => {
    let e = out.get(date);
    if (!e) {
      e = { total: 0, byProvider: {} };
      out.set(date, e);
    }
    return e;
  };

  const merged = new Set<string>();
  for (const [date, e] of out) {
    for (const [key, per] of server) {
      const official = per.get(date);
      if (official === undefined || official <= 0) continue;
      // 官方值替换**整个账户**的本地层之和（含被吸收的别名层），
      // 逐层相减而不是逐层相加，这样 doubao 那部分不会被重复计入总量。
      let local = 0;
      for (const layer of absorbedLayers(key)) {
        local += e.byProvider[layer] ?? 0;
        delete e.byProvider[layer];
      }
      e.byProvider[key] = official; // 官方口径优先
      e.total += official - local; // 替换而非相加，避免双计
      merged.add(`${key}\u0000${date}`);
    }
  }
  // 再扫一遍服务端日期：那些**本机完全没有工具行**的日子在 out 里没有条目，
  // 只遍历 out 会把它们整段丢掉——而那正是"合并"最该补上的日子（实测本机
  // GLM 有账的 4 天与服务端 20 天只重叠 2 天）。逐格补，本机归因为 0，
  // 故 total 直接等于官方值。已合并过的格由 merged 跳过，不重复加。
  for (const [key, per] of server) {
    for (const [date, official] of per) {
      if (official <= 0 || merged.has(`${key}\u0000${date}`)) continue;
      const e = slot(date);
      e.byProvider[key] = official;
      e.total += official;
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
  for (const d of windowDates) {    const v = byDate.get(d)?.byProvider[providerKey] ?? 0;
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

/** 强调色（系统蓝），与 HeatmapGrid 原有色阶一致。 */
const ACCENT_RGB: [number, number, number] = [76, 194, 255];

/** 总量色阶的 5 档 alpha。0 = 无用量。 */
export const LEVEL_ALPHA = [0, 0.22, 0.42, 0.66, 0.94] as const;

/** 非焦点格子的压暗系数：保留可辨的色相层次，不至于抹平成黑。 */
export const DIM_FACTOR = 0.22;

export type PaintMode = "total" | "highlight";

export interface CellPaint {
  bg: string;
  shadow: string;
}

const rgba = (rgb: [number, number, number], a: number): string =>
  `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${a})`;

/**
 * 某一天的格子该画成什么样子。
 *
 * `total` 模式 = 修复后的基线行为：按当日**总量**分级，全系统蓝。
 * `highlight` 模式 = P2 方案二：总量色阶不变，只改色相与透明度——
 *   该 provider 当天有用量 → 品牌色 + 亮描边（alpha 仍由总量分级决定，
 *   所以"那天烧了多少"的信息不丢）；
 *   否则 → 同一色相压暗到 {@link DIM_FACTOR}，退到背景层。
 *
 * 「今天」的琥珀描边在两种模式下都保留——它是时间锚点，不该被联动吞掉。
 */
export function paintCell(args: {
  mode: PaintMode;
  level: 0 | 1 | 2 | 3 | 4;
  isToday: boolean;
  /** 当日该 provider 的用量；0 表示当天没有它的消耗。 */
  focusValue: number;
  brandRgb: [number, number, number];
}): CellPaint {
  const { mode, level, isToday, focusValue, brandRgb } = args;
  const today = isToday ? "inset 0 0 0 1.5px var(--tum-amber)" : "";

  if (mode === "highlight" && focusValue > 0) {
    return {
      bg: rgba(brandRgb, LEVEL_ALPHA[level]),
      shadow: [`inset 0 0 0 1px ${rgba(brandRgb, 0.95)}`, today]
        .filter(Boolean)
        .join(","),
    };
  }
  if (mode === "highlight") {
    return {
      bg: rgba(ACCENT_RGB, LEVEL_ALPHA[level] * DIM_FACTOR),
      shadow: today || "none",
    };
  }
  return {
    bg: rgba(ACCENT_RGB, LEVEL_ALPHA[level]),
    shadow:
      [level === 4 ? `0 0 4px ${rgba(ACCENT_RGB, 0.45)}` : "", today]
        .filter(Boolean)
        .join(",") || "none",
  };
}

/** `#RRGGBB` / `rgb(r,g,b)` / `rgba(r,g,b,a)` → `[r,g,b]`（rgb 形态为 brand-glyphs
 *  兜底灰 rgb(138,143,152) 等 design-token 合规写法准备，与 hexToRgb 同规）；
 *  非法输入回落到强调色。 */
export function hexToRgbTriplet(hex: string): [number, number, number] {
  const s = hex.trim();
  if (/^#[0-9a-f]{6}$/i.test(s)) {
    return [
      parseInt(s.slice(1, 3), 16),
      parseInt(s.slice(3, 5), 16),
      parseInt(s.slice(5, 7), 16),
    ];
  }
  const m = /^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(?:,[^)]*)?\)$/i.exec(s);
  if (m) return [+m[1], +m[2], +m[3]];
  return ACCENT_RGB;
}
