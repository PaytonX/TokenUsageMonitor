// 前端 UI 偏好持久化（localStorage）。
//
// 1) 各页签内部的选择（工具/模型选中项、时间区间）在组件因 `{#if}` 切换而卸载重
//    建时丢失——这里把它们写到 localStorage，挂载时恢复、变更时写回。
// 2) 页签显隐开关：除"总量"页签固定常驻外，趋势/工具/模型页签可由用户在设置窗
//    口启用/禁用。关闭的页签不让其显示，也不渲染其组件（功能不启用）。

export type PageTab = "trend" | "tools" | "models" | "devices";

export const TAB_KEYS: PageTab[] = ["trend", "tools", "models", "devices"];
export const TAB_LABELS: Record<PageTab, string> = {
  trend: "趋势",
  tools: "工具",
  models: "模型",
  devices: "设备",
};

const TABS_KEY = "tum.tabs";

function readLocal(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function writeLocal(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
}

/** 读取一个面板选择偏好，非法值回退到 `fallback`。 */
export function readPref(key: string, fallback: string): string {
  const v = readLocal(key);
  return v === null ? fallback : v;
}

/** 写回一个面板选择偏好。 */
export function writePref(key: string, value: string) {
  writeLocal(key, value);
}

/** 被禁用的页签集合（JSON 数组存于 `tum.tabs`）。 */
export function readHiddenTabs(): PageTab[] {
  const raw = readLocal(TABS_KEY);
  if (!raw) return [];
  try {
    const arr: unknown = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter((x): x is PageTab => (TAB_KEYS as string[]).includes(x as string));
  } catch {
    return [];
  }
}

/** 设某个页签开关。`enabled=false` 表示禁用（隐藏）。 */
export function setTabEnabled(tab: PageTab, enabled: boolean) {
  const cur = new Set(readHiddenTabs());
  if (enabled) cur.delete(tab);
  else cur.add(tab);
  writeLocal(TABS_KEY, JSON.stringify([...cur]));
}

/** 该页签是否启用。 */
export function pageTabEnabled(tab: PageTab): boolean {
  return !readHiddenTabs().includes(tab);
}

/** 当前可见的页签（键列表），总览始终在前。 */
export function visibleTabKeys(): PageTab[] {
  return TAB_KEYS.filter(pageTabEnabled);
}

// --- 全端汇总模式（趋势/工具/模型页签显示全部设备合并用量）---
// 纯前端展示偏好，存 localStorage（`tum.agg`）；写侧由设置页负责广播
// tabs-changed，面板监听后重读本开关并换数据源。

const AGG_KEY = "tum.agg";

/** 全端汇总模式是否开启。 */
export function readAggMode(): boolean {
  return readLocal(AGG_KEY) === "1";
}

/** 写全端汇总开关（不广播；调用方负责 emitTabsChanged 通知面板）。 */
export function writeAggMode(on: boolean) {
  writeLocal(AGG_KEY, on ? "1" : "0");
}

// --- 总量页卡片顺序（拖拽排序，存 provider_id 数组于 `tum.cardOrder`）---

const CARD_ORDER_KEY = "tum.cardOrder";

/** 已保存的卡片顺序（provider_id 数组）。未保存/损坏时返回空数组。 */
export function readCardOrder(): string[] {
  const raw = readLocal(CARD_ORDER_KEY);
  if (!raw) return [];
  try {
    const arr: unknown = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter((x): x is string => typeof x === "string");
  } catch {
    return [];
  }
}

/** 写回卡片顺序。 */
export function writeCardOrder(ids: string[]) {
  writeLocal(CARD_ORDER_KEY, JSON.stringify(ids));
}