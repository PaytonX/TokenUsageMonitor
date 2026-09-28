<script lang="ts">
  import { getHeatmap, forceRefresh } from "../api";
  import {
    displayCurrency,
    estimateCostUsd,
    formatCost,
  } from "../currency";
  import {
    formatUsage,
    hexToRgb,
    mostCriticalWindow,
    percent,
    type BurnInfo,
    type HeatmapCell,
    type UsageSnapshot,
    type UsageUnit,
    type WindowKey,
    type WindowUsage,
  } from "../types";
  import ProviderLogo from "./ProviderLogo.svelte";

  interface Props {
    snapshot: UsageSnapshot;
    burn?: BurnInfo | null;
    /** Wall-clock ms of the last usage update; drives the "上次刷新" row. */
    lastRefreshAt?: number;
    /** true = quota rows show REMAINING %; false = USED % (see `countdown_mode`). */
    countdown?: boolean;
    /** Per-account accent (#RRGGBB); tints the chart bars + window label. */
    accent?: string;
    /** Latest error string for this provider, if any. */
    error?: string | null;
  }

  let {
    snapshot,
    burn = null,
    lastRefreshAt = Date.now(),
    countdown = true,
    accent,
    error = null,
  }: Props = $props();

  const WINDOW_LABELS: Record<WindowKey, string> = {
    five_hour: "5 小时窗口",
    daily: "当日窗口",
    weekly: "本周窗口",
    monthly: "月度窗口",
  };

  // Per-account accent → override CSS vars on the detail root.
  let accentStyle = $derived.by(() => {
    if (!accent) return undefined;
    const rgb = hexToRgb(accent);
    if (!rgb) return undefined;
    return [
      `--acct-accent:${accent}`,
      `--acct-accent-rgb:${rgb}`,
    ].join(";");
  });

  let critical = $derived(mostCriticalWindow(snapshot));
  let kind = $derived(snapshot.provider_id.split("-")[0]);

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

  // Quota pool (额度池): every quota'd window with its used/quota amounts +
  // the used/remaining fraction depending on `countdown`.
  let quotaRows = $derived.by(() => {
    const wins: { label: string; win: WindowUsage }[] = [];
    const push = (key: WindowKey, label: string) => {
      const w = snapshot.windows[key];
      if (w && w.quota > 0) wins.push({ label, win: w });
    };
    push("five_hour", "5 小时");
    push("daily", "当日");
    push("weekly", "周用量");
    push("monthly", "月度");
    return wins.map(({ label, win }) => {
      const pct = percent(win);
      return {
        label,
        amount: `${formatUsage(win.used, win.unit)} / ${formatUsage(win.quota, win.unit)}`,
        pctLabel: `${Math.round((countdown ? 1 - pct : pct) * 100)}%`,
      };
    });
  });

  // Estimated cost: base it on the most critical window's token split, else the
  // monthly window (covers pay-as-you-go providers like volcano API). Renders
  // only when estimation is enabled and the window reports tokens for a known
  // model; never guesses. Currency follows the user's display setting rather
  // than being hardcoded to CNY.
  let costLabel = $derived.by(() => {
    const src = (critical?.window ?? snapshot.windows.monthly) as
      | WindowUsage
      | undefined;
    const usd = src?.tokens ? estimateCostUsd(src.tokens) : null;
    return usd === null ? null : formatCost(usd, displayCurrency(), true);
  });
</script>

<div class="detail" style={accentStyle} data-tauri-drag-region={false}>
  <div class="detail__head">
    <span class="detail__title-group">
      <ProviderLogo {kind} size={16} accent={accent ?? null} />
      <span class="detail__title">{snapshot.provider_display_name}</span>
    </span>
    <span class="detail__window">
      {critical ? WINDOW_LABELS[critical.key] : "暂无窗口"}
    </span>
  </div>

  {#if quotaRows.length > 0}
    <div class="detail__pool">
      <span class="detail__pool-label">额度池</span>
      {#each quotaRows as row (row.label)}
        <div class="detail__pool-row">
          <span class="detail__pool-name">{row.label}</span>
          <span class="detail__pool-amount">{row.amount}</span>
          <span class="detail__pool-pct">{row.pctLabel}</span>
        </div>
      {/each}
      {#if critical?.window.reset_at}
        <div class="detail__pool-row">
          <span class="detail__pool-name">重置</span>
          <span class="detail__pool-amount">下一窗口</span>
          <span class="detail__pool-pct">{resetLabel}</span>
        </div>
      {/if}
    </div>
  {/if}

  <dl class="detail__rows">
    {#if costLabel}
      <dt>估算成本</dt>
      <dd>
        <span class="detail__cost">{costLabel}</span>
        <span class="detail__tag">估算</span>
      </dd>
    {/if}
    <dt>燃烧率</dt>
    <dd>{burnLabel}</dd>
    <dt>预计耗尽</dt>
    <dd>{etaLabel}</dd>
    <dt>当前窗口重置</dt>
    <dd>{resetLabel}</dd>
    <dt>上次刷新</dt>
    <dd>{refreshedLabel}</dd>
    {#if error}
      <dt class="detail__err-label">诊断</dt>
      <dd class="detail__err">{error}</dd>
    {/if}
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

  <div class="detail__actions">
    <button
      type="button"
      class="detail__btn"
      onclick={() => void forceRefresh(snapshot.provider_id)}
      title="立即刷新该账户"
    >↻ 刷新该账户</button>
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

  .detail__title-group {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .detail__title {
    min-width: 0;
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
    color: var(--acct-accent, var(--tum-accent));
    letter-spacing: 0.4px;
  }

  .detail__pool {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-sm);
    flex: none;
  }

  .detail__pool-label {
    font-family: var(--tum-font-mono);
    font-size: 9px;
    color: var(--tum-text-muted);
    letter-spacing: 0.8px;
    text-transform: uppercase;
  }

  .detail__pool-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
  }

  .detail__pool-name {
    color: var(--tum-text-muted);
    flex: none;
  }

  .detail__pool-amount {
    color: var(--tum-text-secondary);
    flex: 1 1 auto;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .detail__pool-pct {
    flex: none;
    text-align: right;
    color: var(--acct-accent, var(--tum-accent));
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.3px;
  }

  .detail__err-label {
    color: var(--tum-danger) !important;
  }

  .detail__err {
    color: var(--tum-danger) !important;
  }

  .detail__actions {
    display: flex;
    gap: 6px;
    flex: none;
  }

  .detail__btn {
    flex: 1;
    padding: 3px 8px;
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font);
    color: var(--tum-text-secondary);
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s ease;
  }

  .detail__btn:hover {
    color: var(--acct-accent, var(--tum-accent));
    border-color: var(--acct-accent-stroke, var(--tum-accent-stroke));
    background: var(--acct-accent-fill, var(--tum-accent-fill));
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

  .detail__cost {
    color: var(--acct-accent, var(--tum-accent));
  }

  .detail__tag {
    margin-left: 4px;
    padding: 0 3px;
    font-family: var(--tum-font);
    font-size: 8px;
    line-height: 1.4;
    color: var(--tum-text-muted);
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: 3px;
    letter-spacing: 0.3px;
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
    background: linear-gradient(180deg, rgba(var(--acct-accent-rgb, 76, 194, 255), 0.9), rgba(var(--acct-accent-rgb, 76, 194, 255), 0.45));
    min-height: 2px;
  }

  .detail__bar--zero {
    height: 2px !important;
    background: rgba(255, 255, 255, 0.08);
  }
</style>
