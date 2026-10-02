// 全端聚合：把多台设备的报告去重合并，合成各页签已经消费的数据形状
// （LocalToolsPayload / LocalModelUsage / 逐设备日序列），使趋势/工具/模型
// 三个页签在「全端汇总模式」下复用原有渲染路径。
//
// 去重口径（与设备页的全端汇总一致）：
// 1. 设备维度：同一 device_id 只计一次（get_hub_devices 已按 id 去重，这里
//    再防御一次；每台设备只上报自己的本地扫描数据，设备之间天然不重叠）；
// 2. 日期维度：单设备序列内同日多条取末条（上游为 Map 构建，正常不重复）。
// 跨设备同日用量是不同机器的真实消耗，按加法合并——不是重复。
//
// 已知边界：
// - date 是各上报方的本地日期，跨时区组网时同一天可能 ±1 偏移；
// - 旧版本对端的报告没有 tools/models 字段：该设备只参与日总量聚合，
//   分工具/分模型视图缺失，调用方用 staleDetailDevices 提示版本差异；
// - 成本不跨设备聚合（各端价格表/币种口径不同），合并后 cost 恒为 0。
import type {
  HubDayIo,
  HubDevice,
  LocalModelUsage,
  LocalToolsPayload,
} from "./types";

/** 全端汇总的设备配色（稳定哈希，与设备页图例共用一套）。 */
const DEV_COLORS = ["#4cc2ff", "#f2b35b", "#5fd4a2", "#f27b9b", "#b58cf5", "#6fd1d1"];

export function deviceColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
  return DEV_COLORS[Math.abs(h) % DEV_COLORS.length];
}

/** 堆叠面积图的配色：按**堆叠顺序**（即传入 ids 的顺序）分配，相邻层必然
 *  不同色相——哈希分配会让颜色相近的两层叠在一起无法分辨。
 *  色板按蓝→琥珀→粉→绿→紫→橙→青设计，相邻高对比。 */
const STACK_COLORS = ["#4cc2ff", "#f2b35b", "#f27b9b", "#5fd4a2", "#b58cf5", "#e8934a", "#6fd1d1"];

export function stackColors(ids: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  ids.forEach((id, i) => {
    out[id] = STACK_COLORS[i % STACK_COLORS.length];
  });
  return out;
}

/** 同一设备只保留一条；保持原顺序（本机在首位）。 */
export function dedupeDevices(devices: HubDevice[]): HubDevice[] {
  const seen = new Set<string>();
  return devices.filter((d) => {
    if (seen.has(d.device_id)) return false;
    seen.add(d.device_id);
    return true;
  });
}

/** 参与聚合但未上报分工具/分模型明细的设备（旧版本），供调用方提示。 */
export function staleDetailDevices(devices: HubDevice[]): string[] {
  return dedupeDevices(devices)
    .filter((d) => !d.tools || Object.keys(d.tools).length === 0)
    .map((d) => d.hostname);
}

/** 单设备的 (date → total) 折叠：同日多条取末条。 */
function foldTotals(daily: { date: string; total: number }[]): Map<string, number> {
  const m = new Map<string, number>();
  for (const day of daily) m.set(day.date, day.total);
  return m;
}

/** 单设备的 (date → 带拆分用量) 折叠：同日多条取末条。 */
function foldIo(daily: HubDayIo[]): Map<string, HubDayIo> {
  const m = new Map<string, HubDayIo>();
  for (const day of daily) m.set(day.date, day);
  return m;
}

/**
 * 合并各设备的分工具序列为一个 LocalToolsPayload（合成形状）：
 * 工具 id 取并集，逐日 input/cache_read/output/total 跨设备求和，
 * 会话数跨设备求和（各端只统计自己的会话，不重叠）。
 * 工具/模型页在汇总态直接消费该 payload，渲染路径不变。
 */
export function buildAggregateToolsPayload(devices: HubDevice[]): LocalToolsPayload {
  const byId = new Map<
    string,
    { name: string; sessions: number; days: Map<string, HubDayIo> }
  >();
  for (const d of dedupeDevices(devices)) {
    for (const [toolId, series] of Object.entries(d.tools ?? {})) {
      let agg = byId.get(toolId);
      if (!agg) {
        agg = { name: series.name || toolId, sessions: 0, days: new Map() };
        byId.set(toolId, agg);
      }
      agg.sessions += series.sessions ?? 0;
      if (!agg.name && series.name) agg.name = series.name;
      for (const [date, io] of foldIo(series.daily ?? [])) {
        const cur = agg.days.get(date) ?? {
          date, input: 0, cache_read: 0, output: 0, total: 0,
        };
        cur.input += io.input ?? 0;
        cur.cache_read += io.cache_read ?? 0;
        cur.output += io.output ?? 0;
        cur.total += io.total ?? 0;
        agg.days.set(date, cur);
      }
    }
  }
  const tools = [...byId.entries()].map(([id, agg]) => {
    const daily = [...agg.days.values()]
      .sort((a, b) => a.date.localeCompare(b.date))
      .map((io) => ({
        date: io.date,
        input: io.input,
        cache_read: io.cache_read,
        output: io.output,
        total: io.total,
      }));
    return {
      id,
      name: agg.name,
      daily,
      total_tokens: daily.reduce((s, d) => s + d.total, 0),
      session_count: agg.sessions,
      project_count: 0,
      scanned_at: new Date().toISOString(),
      models: [] as LocalModelUsage[],
    };
  });
  tools.sort((a, b) => b.total_tokens - a.total_tokens);
  return { tools, sessions_parsed: 0 };
}

/**
 * 合并各设备的分模型序列（仅总量）：模型名取并集，逐日求和。
 * cost 恒为 0（成本不跨设备同步），模型页在汇总态自动隐藏成本位。
 */
export function buildAggregateModels(devices: HubDevice[]): LocalModelUsage[] {
  const byModel = new Map<string, Map<string, number>>();
  for (const d of dedupeDevices(devices)) {
    for (const [model, series] of Object.entries(d.models ?? {})) {
      let days = byModel.get(model);
      if (!days) {
        days = new Map();
        byModel.set(model, days);
      }
      for (const [date, total] of foldTotals(series.daily ?? [])) {
        days.set(date, (days.get(date) ?? 0) + total);
      }
    }
  }
  const out: LocalModelUsage[] = [];
  for (const [model, days] of byModel) {
    const daily = [...days.entries()]
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([date, total]) => ({ date, input: 0, cache_read: 0, output: 0, total }));
    const total_tokens = daily.reduce((s, d) => s + d.total, 0);
    if (total_tokens > 0) {
      out.push({ model, total_tokens, cost: 0, currency: "", cost_estimated: false, daily });
    }
  }
  out.sort((a, b) => b.total_tokens - a.total_tokens);
  return out;
}

/** 趋势页用：每台设备的 (date → total) 序列 + 名称映射（按 id 查主机名）。 */
export function buildDeviceSeries(devices: HubDevice[]): {
  ids: string[];
  series: Record<string, Record<string, number>>;
  names: Record<string, string>;
} {
  const ids: string[] = [];
  const series: Record<string, Record<string, number>> = {};
  const names: Record<string, string> = {};
  for (const d of dedupeDevices(devices)) {
    ids.push(d.device_id);
    names[d.device_id] = d.hostname;
    series[d.device_id] = Object.fromEntries(foldTotals(d.daily ?? []));
  }
  return { ids, series, names };
}
