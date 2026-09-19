<script lang="ts">
  import {
    mostCriticalWindow,
    percent,
    providerShortName,
    remainingPercent,
    type UsageSnapshot,
  } from "../types";
  import PulseDot from "./PulseDot.svelte";

  interface Props {
    snapshots: UsageSnapshot[];
    /** provider_id -> active flag from the latest UsageUpdate. */
    actives?: Record<string, boolean>;
    /** true = show REMAINING %; false = show USED % (see `countdown_mode`).
     *  The fill bar always visualizes consumed amount either way. */
    countdown?: boolean;
  }

  let { snapshots, actives = {}, countdown = true }: Props = $props();
</script>

<div
  class="mini"
  data-tauri-drag-region={false}
  role="list"
  aria-label="已启用来源用量"
>
  {#each snapshots as s (s.provider_id)}
    {@const critical = mostCriticalWindow(s)}
    {@const usedPct = critical ? percent(critical.window) : 0}
    {@const remain = remainingPercent(s)}
    {@const tone = usedPct >= 0.95 ? "crit" : usedPct >= 0.8 ? "warn" : "ok"}
    <div class="mini__row" role="listitem">
      <PulseDot active={!!actives[s.provider_id]} {tone} size={7} />
      <span class="mini__name">
        {providerShortName(s.provider_id, s.provider_display_name)}
      </span>
      <span class="mini__track">
        <span
          class={`mini__fill mini__fill--${tone}`}
          style={`width:${(usedPct * 100).toFixed(1)}%`}
        ></span>
      </span>
      <span class="mini__pct">
        {Math.round((countdown ? remain : usedPct) * 100)}%
      </span>
    </div>
  {:else}
    <div class="mini__row mini__row--empty" role="listitem">暂无已启用来源</div>
  {/each}
</div>

<style>
  .mini {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 2px 0 8px;
  }

  .mini__row--empty {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
  }

  .mini__row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 18px;
  }

  .mini__name {
    max-width: 52px;
    min-width: 0;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mini__track {
    flex: 1;
    min-width: 20px;
    height: 3px;
    border-radius: var(--tum-radius-pill);
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }

  .mini__fill {
    display: block;
    height: 100%;
    border-radius: var(--tum-radius-pill);
    transition: width 0.4s ease;
  }

  .mini__fill--ok {
    background: var(--tum-grad-ok);
  }

  .mini__fill--warn {
    background: var(--tum-grad-warn);
    box-shadow: 0 0 5px rgba(255, 200, 61, 0.4);
  }

  .mini__fill--crit {
    background: var(--tum-grad-crit);
    box-shadow: 0 0 5px rgba(255, 95, 86, 0.45);
  }

  .mini__pct {
    width: 30px;
    flex: none;
    text-align: right;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-primary);
  }
</style>
