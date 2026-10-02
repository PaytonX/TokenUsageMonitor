// 共享趋势数据工具（C7）：供嵌入面板（TrendPanel）与独立窗口（TrendWindow）
// 复用。负责按维度拉取逐日序列、按日期构建趋势序列、区间定义、刻度稀疏策略、
// 数值/颜色格式化。
//
// 数据口径（2026-10 重构）：趋势历史统一来自 `usage_daily` 账本——本机模式 =
// 各工具的 token 日序列（getUsageHistory），全端汇总模式 = 各设备的工具日序列
// （get_hub_devices）。provider 差分序列（percent/cny）不再进入趋势视图，
// 也不再跨单位相加。
import { getUsageHistory } from "./api";
import { providerForModel, providerLabel, providerPalette } from "./model-provider";

export type RangeKey = "7d" | "30d" | "90d";
export interface RangeDef {
  key: RangeKey;
  label: string;
  days: number;
}

export const RANGES: RangeDef[] = [
  { key: "7d", label: "近 7 天", days: 7 },
  { key: "30d", label: "近 30 天", days: 30 },
  { key: "90d", label: "近 90 天", days: 90 },
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
 * 归因顺序：模型名前缀 → 单 Provider 工具兜底 → other（见 model-provider.ts）。
 * ids 按窗口内总量降序（图例与堆叠顺序一致），颜色取 provider 品牌色。
 */
export async function fetchProviderCrossToolSeries(
  days: number,
): Promise<NamedSeries & { colors: Record<string, string> }> {
  const { rows } = await getUsageHistory(days);
  const byProvider = new Map<string, Map<string, number>>();
  for (const r of rows) {
    if (r.kind !== "tool") continue;
    const key = providerForModel(r.model, r.source);
    let m = byProvider.get(key);
    if (!m) {
      m = new Map();
      byProvider.set(key, m);
    }
    m.set(r.date, (m.get(r.date) ?? 0) + r.total);
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
