<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow, currentMonitor } from "@tauri-apps/api/window";
  import {
    getUsage,
    getProviderStates,
    getSettings,
    onUsageUpdated,
    onProviderError,
    onSettingsChanged,
    setWindowMode,
    forceRefresh,
    openSettings,
    type UsageSnapshot,
    type ProviderState,
    type ProviderError,
    type Settings,
    remainingPercent,
  } from "./lib";
  import ProviderCard from "./lib/components/ProviderCard.svelte";
  import ProgressRing from "./lib/components/ProgressRing.svelte";
  import HeatmapGrid from "./lib/components/HeatmapGrid.svelte";

  type Mode = "dashboard" | "compact";

  let snapshots = $state<UsageSnapshot[]>([]);
  let errors = $state<Record<string, string>>({});
  let mode = $state<Mode>("dashboard");
  let heatmapTabId = $state<string | null>(null);
  const PROVIDER_COLORS: Record<string, string> = {
    minimax: "#ff5c5c",
    deepseek: "#4d6bfe",
    volcengine: "#12b76a",
  };
  const FOCUS_FALLBACK_COLOR = "#8a8f98";
  const FOCUS_KEY = "tum.focus";

  // Global focus: "all" (aggregate min) or one provider_id. Drives the
  // header ring, the chips row and the heatmap panel.
  let focus = $state(localStorage.getItem(FOCUS_KEY) ?? "all");
  const PILL_DRAG_THRESHOLD_PX = 4;
  const PILL_FADE_DELAY_MS = 2500;
  const PILL_EDGE_THRESHOLD_PX = 24;
  let settings = $state<Settings | null>(null);

  let now = $state(new Date());
  // Wall-clock at the moment we last received an `usage-updated` (or did a
  // manual refresh). Drives the "next refresh in Ns" countdown in the footer.
  let lastRefreshAt = $state<number>(Date.now());
  let unlistenFns: Array<() => void> = [];

  onMount(async () => {
    // Initial pull.
    snapshots = await getUsage();
    const states = await getProviderStates();
    errors = extractErrors(states);
    settings = await getSettings();
    lastRefreshAt = Date.now();

    if (snapshots.length > 0 && !heatmapTabId) {
      heatmapTabId = snapshots[0].provider_id;
    }

    // Live updates.
    unlistenFns.push(
      await onUsageUpdated((snap) => {
        snapshots = snapshots
          .filter((s) => s.provider_id !== snap.provider_id)
          .concat(snap)
          .sort((a, b) => a.provider_id.localeCompare(b.provider_id));
        if (!heatmapTabId) heatmapTabId = snap.provider_id;
        errors = { ...errors, [snap.provider_id]: "" };
        lastRefreshAt = Date.now();
      }),
    );

    unlistenFns.push(
      await onProviderError(({ id, error }) => {
        errors = { ...errors, [id]: formatError(error) };
        // An error still represents a poll attempt — reset the countdown so
        // users see "next refresh in 5m" rather than a stale negative number.
        lastRefreshAt = Date.now();
      }),
    );

    // Settings saved (from the Settings window): the backend has already
    // dropped disabled providers from its cache and stopped their pollers,
    // so re-pulling usage here removes their cards immediately.
    unlistenFns.push(
      await onSettingsChanged(async (next) => {
        settings = next;
        snapshots = await getUsage();
        const states = await getProviderStates();
        errors = extractErrors(states);
        lastRefreshAt = Date.now();
      }),
    );

    // 1-second clock tick for the timestamp header.
    const tick = setInterval(() => (now = new Date()), 1000);
    unlistenFns.push(() => clearInterval(tick));
  });

  // Aggregate progress ring shows the worst remaining % across all providers.
  let aggregateRemaining = $derived(
    snapshots.length === 0
      ? 0
      : Math.min(...snapshots.map(remainingPercent)),
  );
  let aggregateLabel = $derived(
    snapshots.length === 0
      ? "--"
      : `${Math.round(aggregateRemaining * 100)}%`,
  );

  // Snapshot behind the global focus ("all" → aggregate; one provider → it).
  let focusedSnapshot = $derived(
    focus === "all"
      ? null
      : (snapshots.find((s) => s.provider_id === focus) ?? null),
  );
  let ringPercent = $derived(
    focusedSnapshot ? remainingPercent(focusedSnapshot) : aggregateRemaining,
  );
  let ringLabel = $derived(
    focusedSnapshot
      ? `${Math.round(ringPercent * 100)}%`
      : aggregateLabel,
  );

  // The active snapshot whose heatmap is shown in the bottom panel.
  let activeSnapshot = $derived(
    focusedSnapshot ??
    snapshots.find((s) => s.provider_id === heatmapTabId) ??
    snapshots[0] ??
    null,
  );

  // Snapshots that actually carry heatmap data — drives the tab list.
  let snapshotsWithHeatmap = $derived(
    snapshots.filter(
      (s) => s.heatmap && s.heatmap.length > 0,
    ),
  );
  let hasHeatmap = $derived(snapshotsWithHeatmap.length > 0);

  // Prefer the heatmap cells' own unit (e.g. Volcano windows are AFP but its
  // heatmap cells are Tokens); fall back to the windows' unit.
  let heatmapUnit = $derived(
    activeSnapshot?.heatmap?.[0]?.unit ??
    activeSnapshot?.windows?.monthly?.unit ??
    activeSnapshot?.windows?.weekly?.unit ??
    activeSnapshot?.windows?.five_hour?.unit ??
    "tokens"
  );

  // Ensure heatmapTabId always points at a snapshot that exists &
  // (preferably) has heatmap data, so the heatmap tab is never orphaned.
  $effect(() => {
    if (snapshots.length === 0) return;
    const stillExists = snapshots.some((s) => s.provider_id === heatmapTabId);
    if (!stillExists) {
      heatmapTabId = snapshotsWithHeatmap[0]?.provider_id ?? snapshots[0].provider_id;
    }
  });

  // Persist focus across restarts.
  $effect(() => {
    localStorage.setItem(FOCUS_KEY, focus);
  });

  // If the focused provider disappears (disabled in Settings), fall back to
  // "all". Skipped while snapshots is still empty (startup race).
  $effect(() => {
    if (snapshots.length === 0) return;
    if (focus !== "all" && !snapshots.some((s) => s.provider_id === focus)) {
      focus = "all";
    }
  });

  // Mini pill (compact mode) — focused provider color/name.
  const focusedColor = $derived(
    focusedSnapshot
      ? (PROVIDER_COLORS[focusedSnapshot.provider_id] ?? FOCUS_FALLBACK_COLOR)
      : FOCUS_FALLBACK_COLOR,
  );
  const focusedName = $derived(
    focusedSnapshot ? focusedSnapshot.provider_display_name : "全部",
  );

  // Pill fade machine (spec §3.4): fade to 22% after 2.5s while parked near
  // a screen edge in compact mode; pointer enter / leaving the edge /
  // dashboard mode restores full opacity.
  let pillHovered = $state(false);
  let pillFaded = $state(false);
  let pillFadeTimer: ReturnType<typeof setTimeout> | null = null;
  let pillDragStart: { x: number; y: number } | null = null;
  let pillDidDrag = false;

  function clearPillFadeTimer() {
    if (pillFadeTimer !== null) {
      clearTimeout(pillFadeTimer);
      pillFadeTimer = null;
    }
  }

  async function pillNearEdge(): Promise<boolean> {
    const win = getCurrentWindow();
    const [pos, size, monitor] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      currentMonitor(),
    ]);
    if (!monitor) return false;
    const threshold = PILL_EDGE_THRESHOLD_PX * monitor.scaleFactor;
    const area = monitor.workArea ?? { position: monitor.position, size: monitor.size };
    const left = pos.x - area.position.x;
    const top = pos.y - area.position.y;
    const right = area.position.x + area.size.width - (pos.x + size.width);
    const bottom = area.position.y + area.size.height - (pos.y + size.height);
    return Math.min(left, top, right, bottom) < threshold;
  }

  async function refreshPillFade() {
    clearPillFadeTimer();
    if (mode !== "compact" || pillHovered) {
      pillFaded = false;
      return;
    }
    if (!(await pillNearEdge())) {
      pillFaded = false;
      return;
    }
    pillFadeTimer = setTimeout(() => {
      pillFaded = !pillHovered;
    }, PILL_FADE_DELAY_MS);
  }

  function onPillPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    pillDragStart = { x: event.clientX, y: event.clientY };
    pillDidDrag = false;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function onPillPointerMove(event: PointerEvent) {
    if (!pillDragStart) return;
    const dx = event.clientX - pillDragStart.x;
    const dy = event.clientY - pillDragStart.y;
    if (!pillDidDrag && Math.hypot(dx, dy) > PILL_DRAG_THRESHOLD_PX) {
      pillDidDrag = true;
      clearPillFadeTimer();
      pillFaded = false;
      void getCurrentWindow().startDragging();
    }
  }

  function onPillPointerUp() {
    const didDrag = pillDidDrag;
    pillDragStart = null;
    pillDidDrag = false;
    if (!didDrag) {
      void toggleMode();
    } else {
      void refreshPillFade();
    }
  }

  // Window moved (OS drag or external) → re-evaluate the pill fade state.
  $effect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void win
      .onMoved(() => {
        void refreshPillFade();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  });

  let timeLabel = $derived(
    now.toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }),
  );

  // Per-provider default interval is 5 minutes (300s). User can override via
  // Settings. 0 means "use default".
  let pollIntervalSec = $derived(
    settings && settings.poll_interval_seconds > 0
      ? settings.poll_interval_seconds
      : 300,
  );
  let nextRefreshMs = $derived(lastRefreshAt + pollIntervalSec * 1000);
  let nextRefreshLabel = $derived.by(() => {
    const remaining = Math.max(0, nextRefreshMs - now.getTime());
    const totalSec = Math.floor(remaining / 1000);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    if (m > 0) return `${m}m ${s.toString().padStart(2, "0")}s`;
    return `${s}s`;
  });

  async function toggleMode() {
    const next: Mode = mode === "dashboard" ? "compact" : "dashboard";
    await setWindowMode(next);
    mode = next;
    pillHovered = false;
    pillFaded = false;
    clearPillFadeTimer();
    if (next === "compact") {
      void refreshPillFade();
    }
  }

  async function refresh() {
    await forceRefresh();
    snapshots = await getUsage();
    lastRefreshAt = Date.now();
  }

  async function closeApp() {
    await getCurrentWindow().close();
  }

  function formatError(err: ProviderError): string {
    if (err.Network) return `网络错误 · ${err.Network.message}`;
    if (err.Auth) return `鉴权失败 · ${err.Auth.message}`;
    if (err.Parse) return `解析错误 · ${err.Parse.message}`;
    if (err.RateLimited) return "被限流，稍后重试";
    if (err.NotConfigured) return "未配置";
    if (err.Internal) return `内部错误 · ${err.Internal.message}`;
    return "未知错误";
  }

  function extractErrors(states: Record<string, ProviderState>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [id, st] of Object.entries(states)) {
      if (st.last_error) out[id] = formatError(st.last_error);
    }
    return out;
  }
</script>

<main class="shell" class:shell--compact={mode === "compact"} data-tauri-drag-region oncontextmenu={(e) => e.preventDefault()}>
  {#if mode === "compact"}
    <div
      class="pill"
      class:is-faded={pillFaded}
      onpointerenter={() => {
        pillHovered = true;
        clearPillFadeTimer();
        pillFaded = false;
      }}
      onpointerleave={() => {
        pillHovered = false;
        void refreshPillFade();
      }}
      onpointerdown={onPillPointerDown}
      onpointermove={onPillPointerMove}
      onpointerup={onPillPointerUp}
      oncontextmenu={(e) => e.preventDefault()}
    >
      <div class="pill__ring">
        <ProgressRing value={ringPercent} label="" size={24} stroke={3} />
        <span class="pill__percent">{ringLabel}</span>
      </div>
      <span class="pill__divider"></span>
      <span class="pill__dot" style:background={focusedColor}></span>
      <span class="pill__name">{focusedName}</span>
      <button
        class="pill__close"
        title="关闭应用"
        onpointerdown={(e) => e.stopPropagation()}
        onclick={(e) => {
          e.stopPropagation();
          void closeApp();
        }}
      >✕</button>
    </div>
  {:else}
    <header class="shell__header" data-tauri-drag-region>
      <div class="shell__brand">
        <span class="shell__dot"></span>
        <span class="shell__title">TokenUsageMonitor</span>
      </div>
      <div class="shell__actions" data-tauri-drag-region={false}>
        <ProgressRing value={ringPercent} label={ringLabel} size={28} stroke={3} />
        <button class="shell__btn" onclick={refresh} title="立即刷新">↻</button>
        <button class="shell__btn" onclick={() => openSettings()} title="设置">⚙</button>
        <button class="shell__btn" onclick={toggleMode} title="折叠到迷你态">⤢</button>
        <button class="shell__btn shell__btn--close" onclick={closeApp} title="关闭">×</button>
      </div>
    </header>

    {#if snapshots.length > 0}
      <div class="focus-row" data-tauri-drag-region={false}>
        <button
          class="heatmap__tab"
          class:heatmap__tab--active={focus === "all"}
          onclick={() => (focus = "all")}
        >全部</button>
        {#each snapshots as snap (snap.provider_id)}
          <button
            class="heatmap__tab"
            class:heatmap__tab--active={focus === snap.provider_id}
            onclick={() => (focus = snap.provider_id)}
          >
            <span
              class="focus-dot"
              style:background={PROVIDER_COLORS[snap.provider_id] ?? FOCUS_FALLBACK_COLOR}
            ></span>
            {snap.provider_display_name}
          </button>
        {/each}
      </div>
    {/if}

    <section class="shell__cards">
      {#if snapshots.length === 0}
        <div class="shell__empty">
          <p>正在拉取最新用量…</p>
          <p class="shell__hint">首次启动可能需要 1-2 秒</p>
        </div>
      {:else}
        {#each snapshots as snap (snap.provider_id)}
          <ProviderCard
            snapshot={snap}
            error={errors[snap.provider_id] ?? null}
          />
        {/each}
      {/if}
    </section>

    {#if snapshots.length > 0 && (hasHeatmap || focus !== "all")}
      <section class="shell__heatmap" data-tauri-drag-region={false}>
        <div class="heatmap__head">
          <span class="heatmap__title">日历热力图</span>
          {#if focus === "all"}
            <div class="heatmap__tabs">
              {#each snapshotsWithHeatmap as snap (snap.provider_id)}
                <button
                  class="heatmap__tab {snap.provider_id === heatmapTabId ? 'heatmap__tab--active' : ''}"
                  onclick={() => (heatmapTabId = snap.provider_id)}
                  title={snap.provider_display_name}
                >{snap.provider_display_name}</button>
              {/each}
            </div>
          {/if}
        </div>
        {#if activeSnapshot && activeSnapshot.heatmap}
          <HeatmapGrid cells={activeSnapshot.heatmap} unit={heatmapUnit} />
        {:else if focus !== "all"}
          <div class="heatmap__empty">该来源暂无热力图数据</div>
        {/if}
      </section>
    {/if}

    <footer class="shell__footer" data-tauri-drag-region>
      <span class="shell__time">{timeLabel}</span>
      <span class="shell__next">
        <span class="shell__next-label">下次刷新</span>
        <span class="shell__next-value">{nextRefreshLabel}</span>
      </span>
      <span class="shell__count">共 {snapshots.length} 个 Provider</span>
    </footer>
  {/if}
</main>

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--tum-bg);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-lg);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    padding: var(--tum-space-4);
    gap: var(--tum-space-3);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    overflow: hidden;
    cursor: grab;
  }

  /* In compact mode, remove the shell's padding and gap so the mini pill
     gets the full window area (150x44). */
  .shell--compact {
    padding: 0;
    gap: 0;
    border: none;
    border-radius: 0;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }

  .shell:active {
    cursor: grabbing;
  }

  .shell__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: var(--tum-space-2);
    border-bottom: 1px solid var(--tum-border);
  }

  .shell__brand {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .shell__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--tum-accent);
    box-shadow: 0 0 8px var(--tum-accent-glow), 0 0 2px var(--tum-accent);
  }

  .shell__title {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    font-family: var(--tum-font-mono);
  }

  .shell__actions {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .shell__btn {
    width: 22px;
    height: 22px;
    border: 1px solid var(--tum-border);
    background: var(--tum-surface);
    color: var(--tum-text-secondary);
    border-radius: var(--tum-radius-sm);
    cursor: pointer;
    font-size: 11px;
    line-height: 1;
    transition: all 0.15s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--tum-font-mono);
  }

  .shell__btn:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .shell__cards {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
    scrollbar-width: thin;
    scrollbar-color: var(--tum-border) transparent;
  }

  .shell__cards::-webkit-scrollbar {
    width: 4px;
  }

  .shell__cards::-webkit-scrollbar-thumb {
    background: var(--tum-border);
    border-radius: 2px;
  }

  .shell__empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    text-align: center;
    gap: 6px;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.4px;
  }

  .shell__hint {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .shell__heatmap {
    padding: var(--tum-space-2) 0;
    border-top: 1px solid var(--tum-border);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
  }

  .heatmap__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--tum-space-2);
    flex-wrap: wrap;
  }

  .heatmap__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .heatmap__tabs {
    display: flex;
    gap: 2px;
    flex-wrap: wrap;
  }

  .heatmap__tab {
    padding: 2px 8px;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    background: transparent;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    cursor: pointer;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
    transition: all 0.15s ease;
  }

  .heatmap__tab:hover {
    color: var(--tum-text-secondary);
    background: var(--tum-surface-hover);
  }

  .heatmap__tab--active {
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    border-color: var(--tum-accent-stroke);
  }

  .focus-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .focus-row .heatmap__tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .focus-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }

  .heatmap__empty {
    padding: var(--tum-space-4) 0;
    text-align: center;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
  }

  .shell__footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-top: var(--tum-space-2);
    border-top: 1px solid var(--tum-border);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.4px;
    gap: var(--tum-space-2);
  }

  .shell__time {
    color: var(--tum-accent);
    letter-spacing: 0.8px;
  }

  .shell__next {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--tum-text-muted);
  }

  .shell__next-label {
    text-transform: uppercase;
    letter-spacing: 0.8px;
  }

  .shell__next-value {
    color: var(--tum-text-secondary);
    letter-spacing: 0.4px;
  }

  /* Mini pill (compact) — single row 150x44 (see ipc::set_window_mode
     "compact"). Background matches the old compact__btn fill. */
  .pill {
    height: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: 22px;
    background: rgba(10, 14, 26, 0.9);
    border: 1px solid var(--tum-border);
    transition: opacity 0.35s ease, transform 0.35s ease;
    cursor: default;
    overflow: hidden;
  }

  .pill.is-faded {
    opacity: 0.22;
    transform: scale(0.9);
  }

  .pill__ring {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .pill__percent {
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font-mono);
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
  }

  .pill__divider {
    width: 1px;
    height: 20px;
    background: var(--tum-border-strong);
    flex-shrink: 0;
  }

  .pill__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .pill__name {
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font);
    color: var(--tum-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .pill__close {
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity 0.2s ease, background 0.2s ease, color 0.2s ease;
  }

  .pill:hover .pill__close {
    opacity: 1;
  }

  .pill__close:hover {
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
  }
</style>
