// 共享趋势数据工具（C7）：供嵌入面板（TrendPanel）与独立窗口（TrendWindow）
// 复用。负责拉取逐 provider 的 heatmap、按日期构建趋势序列、区间定义、刻度
// 稀疏策略、数值/颜色格式化。
import { getHeatmap } from "./api";
import type { UsageSnapshot } from "./types";

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

/** 会话级逐 provider 逐日序列缓存：每个 provider 一次拉满 90 天，之后
 * 任意区间/视图切换都只做前端切片，不再重复 IPC + 数据库查询。 */
const seriesCache = new Map<string, { series: Record<string, number> }>();

/**
 * 只重拉最近 `days` 天（默认今天/昨天）并入缓存，供周期刷新"当天"用量使用。
 * 仅更新已有缓存的日期条目，避免整表重拉。
 */
export async function refreshProviderSeries(
  providerIds: string[],
  days = 2,
): Promise<void> {
  await Promise.all(
    providerIds.map(async (id) => {
      const cached = seriesCache.get(id);
      if (!cached) return; // 尚未拉取过则无需刷新
      try {
        const cells = await getHeatmap(id, Math.max(1, days));
        for (const c of cells) cached.series[c.date] = c.value;
      } catch {
        /* 忽略单次刷新失败，保留旧值 */
      }
    }),
  );
}

/**
 * 拉取每个 provider 的逐日用量，按 provider -> date -> value。
 * 内部总是获取最大 90 天并缓存；`days` 仅作为向上对齐参考（取 max(days,90)），
 * 调用方再用 buildTrendDays(rangeDays) 切片到当前区间。
 */
export async function fetchProviderSeries(
  providerIds: string[],
  days: number,
): Promise<Record<string, Record<string, number>>> {
  void days; // 预留：统一按 90 天拉取以覆盖所有区间（7/30/90）
  const all: Record<string, Record<string, number>> = {};
  await Promise.all(
    providerIds.map(async (id) => {
      let cached = seriesCache.get(id);
      if (!cached) {
        const byDate: Record<string, number> = {};
        try {
          const cells = await getHeatmap(id, 90);
          for (const c of cells) byDate[c.date] = c.value;
        } catch {
          /* 热力图拉取失败时留空，面板不中断 */
        }
        cached = { series: byDate };
        seriesCache.set(id, cached);
      }
      all[id] = cached.series;
    }),
  );
  return all;
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

/** 未配置强调色时各 provider kind 的回退色（key 为 kind 首段）。 */
const PROVIDER_FALLBACK_COLORS: Record<string, string> = {
  minimax: "#ff5c5c",
  minimax_api: "#ff5c5c",
  deepseek: "#4d6bfe",
  // 与 brand-glyphs.ts 登记的 volcengine 品牌色保持一致（tile 底色为 #006aff，
  // 折线取其略亮一档的同色系，在深色图表底上对比更足）。此处只是"账户未设
  // 强调色"时的回退色——账户自定义强调色优先级更高。
  volcengine: "#1664ff",
  volcengine_api: "#1664ff",
  openai: "#10a37f",
  opencode: "#8b5cf6",
  gemini: "#4285f4",
  anthropic: "#d97757",
  qwen: "#8b5cf6",
  kimi: "#5e5ce6",
  doubao: "#1c7ee0",
  spark: "#f6a21c",
  xiaomi_plan: "#ff6a00",
  xiaomi_api: "#ff6a00",
};

/** 构建 provider_id -> 颜色 映射：账户强调色优先，回退到 kind 色/灰。 */
export function providerColors(
  snapshots: UsageSnapshot[],
  accentById: Record<string, string>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const s of snapshots) {
    const kind = s.provider_id.split("-")[0];
    out[s.provider_id] =
      accentById[s.provider_id] ??
      PROVIDER_FALLBACK_COLORS[kind] ??
      "#8a8f98";
  }
  return out;
}