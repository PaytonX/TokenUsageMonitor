// Tauri invoke() and event listen() wrappers for the IPC commands
// defined in `src-tauri/src/ipc.rs`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountMeta,
  BurnInfo,
  Credentials,
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
