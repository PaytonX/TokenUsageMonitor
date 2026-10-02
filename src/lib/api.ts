// Tauri invoke() and event listen() wrappers for the IPC commands
// defined in `src-tauri/src/ipc.rs`.

import { invoke } from "@tauri-apps/api/core";
import { emit, emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountMeta,
  BurnInfo,
  Credentials,
  DetectedCodexToken,
  DeviceInfo,
  HeatmapCell,
  PeekSide,
  PeekState,
  ProviderCatalog,
  ProviderError,
  ProviderState,
  ProxyTestResult,
  Settings,
  TestResult,
  UsageSnapshot,
  UsageUpdate,
  WindowMode,
} from "./types";

/** Returns the provider catalogue (built-in presets + configured accounts) for
 * the Settings UI. */
export async function getProviders(): Promise<ProviderCatalog> {
  return invoke<ProviderCatalog>("get_providers");
}

/** Add a new account (or update an existing one by `instance_id`). A brand-new
 * account is passed with an empty `instance_id`; the backend assigns one and
 * it is returned here. */
export async function upsertAccount(account: AccountMeta): Promise<string> {
  return invoke<string>("upsert_account", { account });
}

/** Remove an account and its credentials. Polling for it stops. */
export async function removeAccount(instanceId: string): Promise<void> {
  await invoke<void>("remove_account", { instanceId });
}

export async function getUsage(): Promise<UsageSnapshot[]> {
  return invoke<UsageSnapshot[]>("get_usage");
}

export async function getProviderStates(): Promise<
  Record<string, ProviderState>
> {
  return invoke<Record<string, ProviderState>>("get_provider_states");
}

export async function getHeatmap(
  providerId: string,
  days = 31,
): Promise<HeatmapCell[]> {
  return invoke<HeatmapCell[]>("get_heatmap", {
    providerId,
    days,
  });
}

export async function forceRefresh(
  providerId?: string,
): Promise<void> {
  await invoke<void>("force_refresh", { providerId });
}

export async function togglePolling(paused: boolean): Promise<boolean> {
  return invoke<boolean>("toggle_polling", { paused });
}

export async function setWindowMode(mode: WindowMode): Promise<void> {
  await invoke<void>("set_window_mode", { mode });
}

/** 贴边把手与主胶囊的会话状态切换。三态：
 * `floating` —— 常态浮动：主窗可交互、不移动位置、不建把手窗口；
 * `revealed` —— 胶囊已贴边滑入：把手存在但不捕获、主窗捕获鼠标；
 * `docked` —— 胶囊收起贴边：先把主窗贴死边缘，再让把手捕获鼠标、主窗穿透。
 * 返回胶囊贴靠的水平边，供前端决定滑入方向。 */
export async function syncPeekWindow(state: PeekState): Promise<PeekSide> {
  return invoke<PeekSide>("sync_peek_window", { state });
}

/** 拖拽松手时问 Rust 该贴哪条边；null = 不贴边（保持浮动）。 */
export async function dockSideOf(
  x: number,
  y: number,
  w: number,
  h: number,
  drag: [number, number] | null,
): Promise<PeekSide | null> {
  return invoke<PeekSide | null>("dock_side_of", {
    x,
    y,
    w,
    h,
    dragX: drag?.[0] ?? null,
    dragY: drag?.[1] ?? null,
  });
}

/** 告知后端「用户开始/结束原生拖拽」：拖拽结束判定在 Rust 侧完成。 */
export async function setPillDragging(active: boolean): Promise<void> {
  await invoke<void>("set_pill_dragging", { active });
}

/** Rust → 主窗：窗口停止移动、判定为拖拽结束，可以结算贴边/浮动了。 */
export function onPillDragSettled(cb: () => void): Promise<UnlistenFn> {
  return listen("pill-drag-settled", () => cb());
}

/** 把手 → 主窗：指针进入（60ms 防抖后由把手页发出）。 */
export function onPeekHover(cb: () => void): Promise<UnlistenFn> {
  return listen("peek-hover", () => cb());
}

/** 把手 → 主窗：指针离开。 */
export function onPeekLeave(cb: () => void): Promise<UnlistenFn> {
  return listen("peek-leave", () => cb());
}

/** Rust → 主窗：胶囊必须常显可交互的兜底信号（托盘恢复 compact 主窗、或把手
 *  捕获鼠标失败已无法唤醒胶囊时发出）。收到即调用 revealPill()。 */
export function onPeekReveal(cb: () => void): Promise<UnlistenFn> {
  return listen("peek-reveal", () => cb());
}

/** 主窗 → 把手：滑出完成，把手可复现；payload 是此刻胶囊贴靠的边（用于
 *  纠正把手的圆角朝向，否则拖到屏幕另一侧后把手会一直朝错方向）。 */
export function emitPeekShow(side: PeekSide): Promise<void> {
  return emitTo("peek", "peek-show", side);
}

export async function openSettings(): Promise<void> {
  await invoke<void>("open_settings");
}

/** Open (or focus) the standalone, resizable trend window. */
export async function openTrendWindow(): Promise<void> {
  await invoke<void>("open_trend_window");
}

/** Open (or focus) the standalone, resizable local-tool window. */
export async function openToolWindow(): Promise<void> {
  await invoke<void>("open_tool_window");
}

export async function closeSettings(): Promise<void> {
  await invoke<void>("close_settings");
}

// --- Settings & credentials ----------------------------------------------

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export async function saveSettings(settings: Settings): Promise<void> {
  await invoke<void>("save_settings", { newSettings: settings });
}

export async function saveCredentials(
  providerId: string,
  creds: Credentials,
): Promise<void> {
  await invoke<void>("save_credentials", {
    providerId,
    creds,
  });
}

export async function deleteCredentials(
  providerId: string,
): Promise<void> {
  await invoke<void>("delete_credentials", { providerId });
}

/** Test a provider connection without persisting credentials. Returns a
 * snapshot preview on success, or an error string on failure. Takes a provider
 * *kind* (works even before the account is registered). */
export async function testProvider(
  providerKind: string,
  creds: Credentials,
): Promise<TestResult> {
  try {
    const snapshot = await invoke<UsageSnapshot>("test_provider", {
      providerKind,
      creds,
    });
    return { ok: true, snapshot };
  } catch (e) {
    return { ok: false, message: String(e) };
  }
}

/** Test an HTTP/SOCKS proxy without persisting it. Always resolves to a
 * structured result; only rejects if the backend itself fails to build a
 * client (e.g. unsupported proxy scheme). */
export async function testProxy(url: string): Promise<ProxyTestResult> {
  return invoke<ProxyTestResult>("test_proxy", { url });
}

/** Read the local ChatGPT Codex login (`~/.codex/auth.json`). Resolves to
 * null when the file is absent; rejects only on an unreadable/invalid file. */
export async function detectCodexToken(): Promise<DetectedCodexToken | null> {
  return invoke<DetectedCodexToken | null>("detect_codex_token");
}

// Event listeners ----------------------------------------------------------

export interface UsageUpdatedEvent {
  event: string;
  payload: UsageUpdate;
}

export interface ProviderErrorEvent {
  event: string;
  payload: { id: string; error: ProviderError };
}

export function onUsageUpdated(
  cb: (update: UsageUpdate) => void,
): Promise<UnlistenFn> {
  return listen<UsageUpdate>("usage-updated", (e) => cb(e.payload));
}

export function onProviderError(
  cb: (err: { id: string; error: ProviderError }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; error: ProviderError }>(
    "provider-error",
    (e) => cb(e.payload),
  );
}

export function onSettingsChanged(
  cb: (settings: Settings) => void,
): Promise<UnlistenFn> {
  return listen<Settings>("settings-changed", (e) => cb(e.payload));
}

/** 页签显隐开关变更：设置窗口写入 localStorage 后广播，让主面板重读。 */
export function onTabsChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("tabs-changed", () => cb());
}
export function emitTabsChanged(): Promise<void> {
  return emit("tabs-changed");
}

/** 本地工具日志增量扫描完成（watch 触发）→ 让工具/模型/设备页重载。 */
export function onToolsUpdated(cb: () => void): Promise<UnlistenFn> {
  return listen("tools-updated", () => cb());
}

import type { LocalToolsPayload } from "./types";

/** Aggregated local-tool usage (Claude Code logs). `force` re-scans on disk. */
export async function getLocalTools(force = false): Promise<LocalToolsPayload> {
  return invoke<LocalToolsPayload>("get_local_tools", { force });
}

/** Local device metadata for the Devices view (B8, multi-device sync later). */
export async function getDeviceReport(): Promise<DeviceInfo> {
  return invoke<DeviceInfo>("get_device_report");
}

/** Devices known to the hub (self first), plus an optional refresh warning. */
export async function getHubDevices(): Promise<HubDevicesResult> {
  return invoke<HubDevicesResult>("get_hub_devices");
}

/** 移除一台设备的上报记录。设备若仍在同步，会在下个周期重新出现。 */
export async function removeHubDevice(deviceId: string): Promise<void> {
  return invoke<void>("remove_hub_device", { deviceId });
}

/** 手动添加远端设备：从指定 hub 地址拉取设备列表并入本地存储，返回并入条数。 */
export async function addRemoteDevice(base: string): Promise<number> {
  return invoke<number>("add_remote_device", { base });
}

/** 统一历史视图数据源：usage_daily 账本原始行（三视图共用的唯一口径）。 */
export interface UsageDailyRow {
  source: string;
  kind: "tool" | "provider";
  date: string;
  model: string;
  input: number;
  cache_read: number;
  output: number;
  total: number;
  unit: string;
  cost: number | null;
  currency: string | null;
  cost_estimated: boolean;
}
export interface UsageHistoryResult {
  rows: UsageDailyRow[];
}
export async function getUsageHistory(days: number): Promise<UsageHistoryResult> {
  return invoke<UsageHistoryResult>("get_usage_history", { days });
}

import type { HubDevicesResult } from "./types";

import type { RatesSnapshot } from "./types";

/** 当前生效的汇率快照（覆盖 > 缓存 > 内置默认）；缓存陈旧时后端顺带刷新。 */
export async function getExchangeRates(): Promise<RatesSnapshot> {
  return invoke<RatesSnapshot>("get_exchange_rates");
}

/** 强制重新拉取汇率并落库，返回刷新后的合并快照。 */
export async function refreshExchangeRates(): Promise<RatesSnapshot> {
  return invoke<RatesSnapshot>("refresh_exchange_rates");
}

/** 后端汇率刷新（定时任务 / 设置页手动刷新）→ 各窗口同步生效表。 */
export function onRatesUpdated(
  cb: (snapshot: RatesSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<RatesSnapshot>("rates-updated", (e) => cb(e.payload));
}
