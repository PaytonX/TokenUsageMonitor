// 共享趋势数据工具（C7）：供嵌入面板（TrendPanel）与独立窗口（TrendWindow）
// 复用。负责按维度拉取逐日序列、按日期构建趋势序列、区间定义、刻度稀疏策略、
// 数值/颜色格式化。
//
// 数据口径（2026-10 重构）：趋势历史统一来自 `usage_daily` 账本——本机模式 =
// 各工具的 token 日序列（getUsageHistory），全端汇总模式 = 各设备的工具日序列
// （get_hub_devices）。provider 差分序列（cny）不再进入趋势视图，也不再跨单位
// 相加。
//
// 唯一的例外是**百分比制额度**（OpenCode Go / MiniMax Token Plan）：它们没有
// token 也没有金额，只有"已用额度百分比"。这类日值由 provider 侧双写进账本
// （kind='provider'、unit='percent'），永远不与 token 相加 —— 见
// docs/adr/0001-percent-quota-usage.md。`fetchQuotaGroups` 是它的数据源，
// **唯一消费者是账户卡片的详情（DetailCard）**：趋势页是纯 token 页面，
// 额度不占它的版面（2026-10-24 方案 C 拍板）。
import { getUsageHistory, getUsage, loadInstanceKinds } from "./api";
import {
  providerForLedgerRow,
  providerForModel,
  providerForRoutedSlice,
  providerKeyForAccount,
  providerLabel,
  providerPalette,
} from "./model-provider";
import {
  buildQuotaSeries,
  groupQuotaSeries,
  subscriptionFingerprint,
  type QuotaGroup,
} from "./quota-usage";
import type { UsageSnapshot } from "./types";
import {
  collectRouterRowsByDate,
  routedAttribution,
  ROUTED_MODEL,
} from "./router-attribution";

export type RangeKey = "7d" | "30d" | "90d";
export interface RangeDef {
  key: RangeKey;
  label: string;
  days: number;
}

export const RANGES: RangeDef[] = [
  { key: "7d", label: "range.7d", days: 7 },
  { key: "30d", label: "range.30d", days: 30 },
  { key: "90d", label: "range.90d", days: 90 },
];

export interface TrendPart {
  id: string;
  value: number;
}
export interface TrendDay {
  date: string;
  label: string;
  parts: TrendPart[];
  total: number;
}

export interface NamedSeries {
  ids: string[];
  series: Record<string, Record<string, number>>;
  names: Record<string, string>;
}

/**
 * 本机模式数据源：账本中各**工具**的逐日 token 总量（kind='tool' 且
 * model='' 的总量行；分模型行由模型页消费）。
 */
export async function fetchToolSeries(days: number): Promise<NamedSeries> {
  const { rows } = await getUsageHistory(days);
  const ids: string[] = [];
  const series: Record<string, Record<string, number>> = {};
  const names: Record<string, string> = {};
  for (const r of rows) {
    if (r.kind !== "tool" || r.model !== "") continue;
    if (!series[r.source]) {
      series[r.source] = {};
      ids.push(r.source);
      names[r.source] = r.source;
    }
    series[r.source][r.date] = (series[r.source][r.date] ?? 0) + r.total;
  }
  return { ids, series, names };
}

/**
 * 本机模式趋势数据源：**按 Provider 跨工具**聚合的日序列。
 *
 * 归因顺序（见 model-provider.ts）：模型名前缀 → 单 Provider 工具兜底 →
 * other；**经路由的行改由计费账户决定**——`glm-5.3-flash` 走火山方舟时必须
 * 记在火山名下，不能按模型名记到智谱。
 *
 * ids 按窗口内总量降序（图例与堆叠顺序一致），颜色取 provider 品牌色。
 */
export async function fetchProviderCrossToolSeries(
  days: number,
): Promise<NamedSeries & { colors: Record<string, string> }> {
  const [{ rows }, kindByInstance] = await Promise.all([
    getUsageHistory(days),
    loadInstanceKinds(),
  ]);
  const byProvider = new Map<string, Map<string, number>>();
  const add = (key: string, date: string, v: number) => {
    let m = byProvider.get(key);
    if (!m) {
      m = new Map();
      byProvider.set(key, m);
    }
    m.set(date, (m.get(date) ?? 0) + v);
  };
  // 经路由的哨兵行先攒着，遍历完统一按路由台账替换成真实模型。
  const routed = new Map<
    string,
    { total: number; sources: Map<string, number>; accounts: Map<string, string> }
  >();
  const routerRows = collectRouterRowsByDate(rows);
  for (const r of rows) {
    if (r.kind !== "tool") continue;
    // 来源总量行（model=''）是**工具**口径，不能与分模型行相加——否则同一个
    // token 会被算两遍（此前本函数漏了这层过滤，趋势值约为真实值的两倍）。
    if (r.model === "") continue;
    if (r.model === ROUTED_MODEL) {
      let e = routed.get(r.date);
      if (!e) {
        e = { total: 0, sources: new Map(), accounts: new Map() };
        routed.set(r.date, e);
      }
      e.total += r.total;
      e.sources.set(r.source, (e.sources.get(r.source) ?? 0) + r.total);
      if (r.provider) e.accounts.set(r.source, r.provider);
      continue;
    }
    // 账户优先、模型名兜底：账本记了计费账户（`opencode-go` / `ark-cn`…）就按
    // 账户归，否则退回模型名前缀。只改「有账户」的行，直连行行为不变。
    add(providerForLedgerRow(r), r.date, r.total);
  }
  for (const [date, e] of routed) {
    const { slices, fallback } = routedAttribution(e.total, routerRows.get(date) ?? []);
    // 按**计费账户**归因，而不是模型名——这正是"我明明调的是火山、趋势却
    // 显示智谱"的根因。
    for (const s of slices) add(providerForRoutedSlice(s.model, s.source, kindByInstance), date, s.total);
    if (fallback > 0) {
      for (const [source, v] of e.sources) {
        // 路由台账没覆盖到的那部分：同样账户优先，再退回模型名/工具默认。
        add(
          providerKeyForAccount(e.accounts.get(source)) ?? providerForModel(ROUTED_MODEL, source),
          date,
          fallback * (v / e.total),
        );
      }
    }
  }
  const totals = [...byProvider.entries()]
    .map(([key, m]) => ({ key, total: [...m.values()].reduce((s, v) => s + v, 0) }))
    // 区间内零用量的 Provider 不进序列（否则图例出现 0M 的空条）。
    .filter((t) => t.total > 0);
  totals.sort((a, b) => b.total - a.total);
  // ids 已按用量降序 → providerPalette 据此区分主次配色。
  const ids = totals.map((t) => t.key);
  const series: Record<string, Record<string, number>> = {};
  const names: Record<string, string> = {};
  for (const { key } of totals) {
    series[key] = Object.fromEntries(byProvider.get(key)!);
    names[key] = providerLabel(key);
  }
  const colors = providerPalette(ids);
  return { ids, series, names, colors };
}

function localDateKey(d: Date): string {
  const y = d.getFullYear();
  const m = `${d.getMonth() + 1}`.padStart(2, "0");
  const dd = `${d.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${dd}`;
}

/**
 * 额度轨道数据源（百分比制 Provider 的独立轨道）。
 *
 * 与 token 序列同源（同一份 `usage_daily` 账本），但只取 `unit='percent'`
 * 的行，并按 kind 汇总 —— 多 Key 的去重与容量归一在 `quota-usage.ts` 里完成
 * （见该文件头与 docs/adr/0001-percent-quota-usage.md）。
 *
 * 订阅指纹需要实时快照里的 `reset_at`：同一订阅的多个 Key 看到同一批重置
 * 时刻。拿不到快照时（后端未就绪/聚合模式）退化为按账户独立订阅，宁可少合并
 * 也不把两份额度当成一份。
 */
export async function fetchQuotaGroups(days: number): Promise<QuotaGroup[]> {
  const [{ rows }, kinds, snapshots] = await Promise.all([
    getUsageHistory(days),
    loadInstanceKinds(),
    getUsage().catch(() => [] as UsageSnapshot[]),
  ]);
  const fingerprints = new Map<string, string>();
  for (const snap of snapshots) {
    fingerprints.set(snap.provider_id, subscriptionFingerprint(snap));
  }
  const series = buildQuotaSeries(rows, {
    kindOf: (source) => kinds.get(source),
    subscriptionOf: (source) => fingerprints.get(source),
    days,
    end: localDateKey(new Date()),
  });
  return groupQuotaSeries(series);
}

/** 按今天锚定的最近 `rangeDays` 天构建有序序列（旧→新）。
 * 只有 value>0 的 provider 进入 parts，便于图表只画实际有数据的序列。 */
export function buildTrendDays(
  series: Record<string, Record<string, number>>,
  providerIds: string[],
  rangeDays: number,
): TrendDay[] {
  const today = new Date();
  const out: TrendDay[] = [];
  for (let i = rangeDays - 1; i >= 0; i--) {
    const d = new Date(today);
    d.setDate(today.getDate() - i);
    const key = localDateKey(d);
    const parts = providerIds
      .filter((id) => (series[id]?.[key] ?? 0) > 0)
      .map((id) => ({ id, value: series[id]?.[key] ?? 0 }));
    const total = parts.reduce((s, p) => s + p.value, 0);
    out.push({
      date: key,
      label: `${d.getMonth() + 1}/${d.getDate()}`,
      parts,
      total,
    });
  }
  return out;
}

/** X 轴刻度稀疏倍数：7d 全标，30d 隔 3，90d 隔 7，避免标签重叠。 */
export function tickEveryFor(rangeDays: number): number {
  if (rangeDays <= 7) return 1;
  if (rangeDays <= 30) return 3;
  return 7;
}

/** 大数压缩显示（用于统计行）。 */
export function formatCompact(v: number): string {
  if (v >= 1_000_000) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1_000) return `${(v / 1e3).toFixed(1)}K`;
  return `${v}`;
}

/**
 * 高亮某一 provider 时，把其余序列压暗。
 *
 * 焦点在趋势页只有一个含义，日历与堆叠柱必须一致表达：否则「点 GLM 之后
 * 日历变了但柱子没变」会让用户怀疑高亮只对一半生效。压暗用与日历
 * `DIM_FACTOR` 相同的 0.22，保证两处的视觉衰减量级一致。
 *
 * 返回新的 colors 映射；`highlightKey` 为空时原样返回（不产生无谓重渲染）。
 */
export function dimOthers(
  colors: Record<string, string>,
  highlightKey: string | null,
  factor = 0.22,
): Record<string, string> {
  if (!highlightKey) return colors;
  const out: Record<string, string> = {};
  for (const [id, hex] of Object.entries(colors)) {
    if (id === highlightKey) {
      out[id] = hex;
      continue;
    }
    if (/^#[0-9a-f]{6}$/i.test(hex)) {
      const r = parseInt(hex.slice(1, 3), 16);
      const g = parseInt(hex.slice(3, 5), 16);
      const b = parseInt(hex.slice(5, 7), 16);
      out[id] = `rgba(${r},${g},${b},${factor})`;
    } else {
      // 已是 rgba() 或颜色名：降不透明度只能整体替换，退回中性灰的压暗版本。
      out[id] = `rgba(138,143,152,${factor})`;
    }
  }
  return out;
}
