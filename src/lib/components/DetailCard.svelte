<script lang="ts">
  import { getHeatmap } from "../api";
  import {
    formatUsage,
    mostCriticalWindow,
    type BurnInfo,
    type HeatmapCell,
    type UsageSnapshot,
    type UsageUnit,
    type WindowKey,
  } from "../types";

  interface Props {
    snapshot: UsageSnapshot;
    burn?: BurnInfo | null;
    /** Wall-clock ms of the last usage update; drives the "上次刷新" row. */
    lastRefreshAt?: number;
  }

  let { snapshot, burn = null, lastRefreshAt = Date.now() }: Props = $props();

  const WINDOW_LABELS: Record<WindowKey, string> = {
    five_hour: "5 小时窗口",
    daily: "当日窗口",
    weekly: "本周窗口",
    monthly: "月度窗口",
  };

  let critical = $derived(mostCriticalWindow(snapshot));

  let burnLabel = $derived(
    burn ? `${formatUsage(burn.rate_per_min, burn.unit)}/min` : "—",
  );

  let etaLabel = $derived.by(() => {
    const secs = burn?.eta_seconds;
    if (secs === null || secs === undefined) return "—";
    if (secs < 60) return `${secs}s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}m`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ${mins % 60}m`;
    const days = Math.floor(hours / 24);
    return `${days}d ${hours % 24}h`;
  });

  let resetLabel = $derived.by(() => {
    const at = critical?.window.reset_at;
    if (!at) return "—";
    return new Date(at).toLocaleString("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  });

  let nowTs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (nowTs = Date.now()), 1000);
    return () => clearInterval(id);
  });

  let refreshedLabel = $derived.by(() => {
    const secs = Math.max(0, Math.floor((nowTs - lastRefreshAt) / 1000));
    if (secs < 60) return `${secs}s 前`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}m 前`;
    return `${Math.floor(mins / 60)}h 前`;
  });

  // --- 近 7 日迷你柱状：自拉 get_heatmap；provider 切换时作废在途请求 ---
  let weekCells = $state<HeatmapCell[]>([]);
  let weekSeq = 0;

  $effect(() => {
    const providerId = snapshot.provider_id;
    const seq = ++weekSeq;
    getHeatmap(providerId, 7)
      .then((rows) => {
        if (seq === weekSeq) weekCells = rows;
      })
      .catch(() => {
        if (seq === weekSeq) weekCells = [];
      });
  });

  function localDateKey(d: Date): string {
    const y = d.getFullYear();
    const m = `${d.getMonth() + 1}`.padStart(2, "0");
    const day = `${d.getDate()}`.padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  let bars = $derived.by(() => {
    const map = new Map(weekCells.map((c) => [c.date, c]));
    const today = new Date();
    const out: { date: string; value: number }[] = [];
    for (let i = 6; i >= 0; i--) {
      const d = new Date(today);
      d.setDate(today.getDate() - i);
      const key = localDateKey(d);
      out.push({ date: key, value: map.get(key)?.value ?? 0 });
    }
    return out;
  });

  let weekMax = $derived(Math.max(1, ...bars.map((b) => b.value)));
  let barUnit = $derived<UsageUnit>(
    weekCells[0]?.unit ?? critical?.window.unit ?? "tokens",
  );
</script>

<div class="detail" data-tauri-drag-region={false}>
  <div class="detail__head">
    <span class="detail__title">{snapshot.provider_display_name}</span>
    <span class="detail__window">
      {critical ? WINDOW_LABELS[critical.key] : "暂无窗口"}
    </span>
  </div>

  <dl class="detail__rows">
    <dt>燃烧率</dt>
    <dd>{burnLabel}</dd>
    <dt>预计耗尽</dt>
    <dd>{etaLabel}</dd>
    <dt>窗口重置</dt>
    <dd>{resetLabel}</dd>
    <dt>上次刷新</dt>
    <dd>{refreshedLabel}</dd>
  </dl>

  <div class="detail__chart">
    <span class="detail__chart-label">近 7 日</span>
    <div class="detail__bars">
      {#each bars as b (b.date)}
        <div class="detail__bar-wrap" title={`${b.date} · ${formatUsage(b.value, barUnit)}`}>
          <div
            class="detail__bar"
            class:detail__bar--zero={b.value === 0}
            style={`height:${((b.value / weekMax) * 100).toFixed(1)}%`}
          ></div>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-radius: var(--tum-radius-md);
    border: 1px solid var(--tum-border-strong);
    background: rgba(36, 38, 42, 0.95);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }

  .detail__head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    flex: none;
  }

  .detail__title {
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    color: var(--tum-text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .detail__window {
    flex: none;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-accent);
    letter-spacing: 0.4px;
  }

  .detail__rows {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 10px;
    row-gap: 3px;
    margin: 0;
  }

  .detail dt {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .detail dd {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-secondary);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .detail__chart {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: none;
  }

  .detail__chart-label {
    font-family: var(--tum-font-mono);
    font-size: 9px;
    color: var(--tum-text-muted);
    letter-spacing: 0.8px;
    text-transform: uppercase;
  }

  .detail__bars {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    height: 34px;
  }

  .detail__bar-wrap {
    flex: 1;
    height: 100%;
    display: flex;
    align-items: flex-end;
  }

  .detail__bar {
    width: 100%;
    border-radius: 2px 2px 0 0;
    background: linear-gradient(180deg, rgba(76, 194, 255, 0.9), rgba(76, 194, 255, 0.45));
    min-height: 2px;
  }

  .detail__bar--zero {
    height: 2px !important;
    background: rgba(255, 255, 255, 0.08);
  }
</style>
