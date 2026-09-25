// Tauri invoke() and event listen() wrappers for the IPC commands
// defined in `src-tauri/src/ipc.rs`.

import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountMeta,
  BurnInfo,
  Credentials,
  DeviceInfo,
  HeatmapCell,
  ProviderCatalog,
  ProviderError,
  ProviderState,
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
