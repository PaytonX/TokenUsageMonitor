<script lang="ts">
  import {
    formatUsage,
    remainingPercent,
    type BurnInfo,
    type UsageSnapshot,
  } from "../types";
  import UsageBar from "./UsageBar.svelte";
  import ResetCountdown from "./ResetCountdown.svelte";
  import ProgressRing from "./ProgressRing.svelte";
  import PulseDot from "./PulseDot.svelte";
  import DetailCard from "./DetailCard.svelte";

  interface Props {
    snapshot: UsageSnapshot;
    error?: string | null;
    burn?: BurnInfo | null;
    active?: boolean;
    lastRefreshAt?: number;
  }

  let {
    snapshot,
    error = null,
    burn = null,
    active = false,
    lastRefreshAt = Date.now(),
  }: Props = $props();

  let w = $derived(snapshot.windows);
  let balanceLabel = $derived.by(() => {
    if (!w.balance) return null;
    return `${formatUsage(w.balance.total, "cny")}`;
  });

  // Ring shows REMAINING; tone thresholds are on used % (80 / 95).
  let remaining = $derived(remainingPercent(snapshot));
  let usedPct = $derived(1 - remaining);
  let ringLabel = $derived(`${Math.round(remaining * 100)}%`);
  let tone = $derived<"ok" | "warn" | "crit">(
    usedPct >= 0.95 ? "crit" : usedPct >= 0.8 ? "warn" : "ok",
  );

  let expanded = $state(false);
</script>

<article class="card" class:card--expanded={expanded} data-tauri-drag-region={false}>
  <header class="card__head">
    <button
      type="button"
      class="card__titlebtn"
      aria-expanded={expanded}
      aria-label={`${snapshot.provider_display_name}${active ? "，正在请求" : ""}用量详情`}
      onclick={() => (expanded = !expanded)}
    >
      <PulseDot {active} {tone} size={8} />
      <span class="card__name">{snapshot.provider_display_name}</span>
    </button>
    {#if snapshot.plan_tier}
      <span class="card__tier">{snapshot.plan_tier}</span>
    {/if}
    <div class="card__head-right">
      {#if balanceLabel}
        <span class="card__balance" title="账户余额">{balanceLabel}</span>
      {/if}
      <span class="card__ring">
        <ProgressRing value={remaining} label={ringLabel} size={36} stroke={4} />
      </span>
    </div>
  </header>

  {#if error}
    <div class="card__error">
      <span class="card__error-dot"></span>
      <span class="card__error-text">{error}</span>
    </div>
  {/if}

  <div class="card__bars">
    {#if w.five_hour}
      <UsageBar usage={w.five_hour} label="5 小时" />
      {#if w.five_hour.reset_at}
        <ResetCountdown resetAt={w.five_hour.reset_at} label="5h" />
      {/if}
    {/if}
    {#if w.weekly}
      <UsageBar usage={w.weekly} label="周用量" />
      {#if w.weekly.reset_at}
        <ResetCountdown resetAt={w.weekly.reset_at} label="周" />
      {/if}
    {/if}
    {#if w.monthly}
      <UsageBar usage={w.monthly} label={w.monthly.quota > 0 ? "月度总量" : "本月消费"} />
      {#if w.monthly.reset_at}
        <ResetCountdown resetAt={w.monthly.reset_at} label="月" />
      {/if}
    {/if}
  </div>

  <DetailCard {snapshot} {burn} {lastRefreshAt} />
</article>

<style>
  .card {
    position: relative;
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    padding: var(--tum-space-3) var(--tum-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
    cursor: default;
    transition: background 0.2s ease;
  }

  .card:hover {
    background: var(--tum-surface-hover);
  }

  .card--expanded {
    z-index: 6;
  }

  .card__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .card__titlebtn {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: default;
  }

  .card__titlebtn:focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: 2px;
    border-radius: var(--tum-radius-xs);
  }

  .card__name {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .card__tier {
    font-size: var(--tum-font-size-xs);
    font-weight: 500;
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    border: 1px solid var(--tum-accent-stroke);
    padding: 1px 6px;
    border-radius: var(--tum-radius-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .card__head-right {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
    flex: none;
  }

  .card__balance {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    color: var(--tum-success);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.4px;
  }

  .card__ring {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .card__bars {
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
  }

  .card__error {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: var(--tum-danger-fill);
    border-left: 2px solid var(--tum-danger);
    border-radius: var(--tum-radius-xs);
  }

  .card__error-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--tum-danger);
    box-shadow: 0 0 6px var(--tum-danger);
    flex-shrink: 0;
  }

  .card__error-text {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-secondary);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }
</style>
