<script lang="ts">
  import {
    formatUsage,
    type UsageSnapshot,
  } from "../types";
  import UsageBar from "./UsageBar.svelte";
  import ResetCountdown from "./ResetCountdown.svelte";

  interface Props {
    snapshot: UsageSnapshot;
    error?: string | null;
  }

  let { snapshot, error = null }: Props = $props();

  let w = $derived(snapshot.windows);
  let balanceLabel = $derived.by(() => {
    if (!w.balance) return null;
    return `${formatUsage(w.balance.total, "cny")}`;
  });
</script>

<article class="card" data-tauri-drag-region={false}>
  <header class="card__head">
    <div class="card__title">
      <span class="card__name">{snapshot.provider_display_name}</span>
      {#if snapshot.plan_tier}
        <span class="card__tier">{snapshot.plan_tier}</span>
      {/if}
    </div>
    {#if balanceLabel}
      <span class="card__balance" title="账户余额">{balanceLabel}</span>
    {/if}
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
</article>

<style>
  .card {
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

  .card__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .card__title {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .card__name {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
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

  .card__balance {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    color: var(--tum-success);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.4px;
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
