<script lang="ts">
  import {
    mostCriticalWindow,
    payAsYouGoLabel,
    percent,
    remainingPercent,
    type UsageSnapshot,
  } from "../types";
  import { brandColorFor } from "../brand-glyphs";
  import PulseDot from "./PulseDot.svelte";
  import ProviderLogo from "./ProviderLogo.svelte";

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
    {@const money = payAsYouGoLabel(s, countdown)}
    {@const tone = usedPct >= 0.95 ? "crit" : usedPct >= 0.8 ? "warn" : "ok"}
    {@const brand = brandColorFor(s.provider_id.split("-")[0])}
    <div
      class="mini__row"
      role="listitem"
      aria-label={`${s.provider_display_name} ${money ?? `${Math.round((countdown ? remain : usedPct) * 100)}%`}`}
    >
      <PulseDot active={!!actives[s.provider_id]} {tone} size={7} accent={brand} />
      <span class="mini__logo" title={s.provider_display_name}>
        <ProviderLogo kind={s.provider_id.split("-")[0]} size={18} />
      </span>
      <span class="mini__track">
        <span
          class="mini__fill"
          style={`width:${(usedPct * 100).toFixed(1)}%;background:${
            tone === "ok" ? brand : `var(--tum-${tone})`
          }`}
        ></span>
      </span>
      <span class="mini__pct">
        {money ?? `${Math.round((countdown ? remain : usedPct) * 100)}%`}
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
    padding: 2px 2px 8px;
  }

  .mini__row--empty {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
  }

  .mini__row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
  }

  .mini__logo {
    width: 18px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .mini__track {
    flex: 1;
    min-width: 20px;
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }

  .mini__fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    transition: width 420ms var(--tum-ease-ring);
  }

  .mini__pct {
    width: 34px;
    flex: none;
    text-align: right;
    font-family: var(--tum-font-mono);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-muted);
  }
</style>