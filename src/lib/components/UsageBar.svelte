<script lang="ts">
  import { percent, formatUsage, type WindowUsage } from "../types";

  interface Props {
    usage: WindowUsage;
    label: string;
  }

  let { usage, label }: Props = $props();

  let pct = $derived(percent(usage));
  let hasQuota = $derived(usage.quota > 0);
  let tone = $derived(pct >= 0.95 ? "crit" : pct >= 0.8 ? "warn" : "ok");
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
        class={`bar__fill bar__fill--${tone}`}
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
    height: 4px;
    background: rgba(255, 255, 255, 0.10);
    border-radius: var(--tum-radius-pill);
    overflow: hidden;
    position: relative;
  }

  .bar__fill {
    height: 100%;
    border-radius: var(--tum-radius-pill);
    transition: width 0.4s ease, background 0.4s ease;
  }

  .bar__fill--ok {
    background: var(--tum-grad-ok);
  }

  .bar__fill--warn {
    background: var(--tum-grad-warn);
    box-shadow: 0 0 6px rgba(255, 200, 61, 0.45);
  }

  .bar__fill--crit {
    background: var(--tum-grad-crit);
    box-shadow: 0 0 6px rgba(255, 95, 86, 0.5);
  }
</style>
