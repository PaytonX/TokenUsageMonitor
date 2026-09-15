// Tauri invoke() and event listen() wrappers for the IPC commands
// defined in `src-tauri/src/ipc.rs`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Credentials,
  HeatmapCell,
  ProviderError,
  ProviderInfo,
  ProviderState,
  Settings,
  TestResult,
  UsageSnapshot,
  WindowMode,
} from "./types";

export async function getProviders(): Promise<ProviderInfo[]> {
  return invoke<ProviderInfo[]>("get_providers");
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
  days = 90,
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

export async function setWindowMode(mode: WindowMode): Promise<void> {
  await invoke<void>("set_window_mode", { mode });
}

export async function openSettings(): Promise<void> {
  await invoke<void>("open_settings");
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
 * snapshot preview on success, or an error string on failure. */
export async function testProvider(
  providerId: string,
  creds: Credentials,
): Promise<TestResult> {
  try {
    const snapshot = await invoke<UsageSnapshot>("test_provider", {
      providerId,
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
  payload: UsageSnapshot;
}

export interface ProviderErrorEvent {
  event: string;
  payload: { id: string; error: ProviderError };
}

export function onUsageUpdated(
  cb: (snapshot: UsageSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<UsageSnapshot>("usage-updated", (e) => cb(e.payload));
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
