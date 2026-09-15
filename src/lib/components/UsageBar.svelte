<script lang="ts">
  import { percent, formatUsage, unitLabel, type WindowUsage } from "../types";

  interface Props {
    usage: WindowUsage;
    label: string;
  }

  let { usage, label }: Props = $props();

  let pct = $derived(percent(usage));
  let hasQuota = $derived(usage.quota > 0);
  let colorClass = $derived(
    pct < 0.7 ? "good" : pct < 0.9 ? "warn" : "danger",
  );
</script>

<div class="bar">
  <div class="bar__head">
    <span class="bar__label">{label}</span>
    <span class="bar__value">
      {formatUsage(usage.used, usage.unit)}
      {#if hasQuota}
        <span class="bar__sep">/</span>
        {formatUsage(usage.quota, usage.unit)}
      {/if}
    </span>
  </div>
  {#if hasQuota}
    <div class="bar__track">
      <div
        class={`bar__fill bar__fill--${colorClass}`}
        style={`width: ${(pct * 100).toFixed(1)}%`}
      ></div>
    </div>
  {/if}
</div>

<style>
  .bar {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .bar__head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .bar__label {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-family: var(--tum-font-mono);
  }

  .bar__value {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-primary);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.3px;
  }

  .bar__sep {
    color: var(--tum-text-muted);
    margin: 0 4px;
  }

  .bar__track {
    height: 3px;
    background: rgba(148, 163, 184, 0.12);
    border-radius: var(--tum-radius-xs);
    overflow: hidden;
    position: relative;
  }

  .bar__fill {
    height: 100%;
    border-radius: var(--tum-radius-xs);
    transition: width 0.4s ease;
  }

  /* Calm telemetry colors - amber stays primary, success muted, danger sharp */
  .bar__fill--good {
    background: var(--tum-accent);
    box-shadow: 0 0 6px var(--tum-accent-glow);
  }

  .bar__fill--warn {
    background: var(--tum-warning);
    box-shadow: 0 0 6px rgba(251, 146, 60, 0.4);
  }

  .bar__fill--danger {
    background: var(--tum-danger);
    box-shadow: 0 0 6px rgba(248, 113, 113, 0.4);
  }
</style>
