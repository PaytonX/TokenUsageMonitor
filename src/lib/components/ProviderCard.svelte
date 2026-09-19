<script lang="ts">
  import {
    formatUsage,
    hexToRgb,
    percent,
    remainingPercent,
    type BurnInfo,
    type UsageSnapshot,
  } from "../types";
  import UsageBar from "./UsageBar.svelte";
  import ResetCountdown from "./ResetCountdown.svelte";
  import ProgressRing from "./ProgressRing.svelte";
  import PulseDot from "./PulseDot.svelte";

  interface Props {
    snapshot: UsageSnapshot;
    error?: string | null;
    burn?: BurnInfo | null;
    active?: boolean;
    lastRefreshAt?: number;
    /** Provider whose card is currently focused in the header ring. */
    focused?: boolean;
    /** Per-account accent (#RRGGBB). Injected as `--acct-accent*` CSS vars on
     *  the card root and passed to the ring; falls back to --tum-accent. */
    accent?: string;
    /** true = the ring/label show REMAINING; false = USED (see `countdown_mode`). */
    countdown?: boolean;
    /** Called when the user clicks this card; App links it to focus + heatmap. */
    onSelect?: () => void;
    /** Reported as the pointer enters/leaves the card; drives the floating
     *  detail overlay in App (small cards no longer clip the detail). */
    onHover?: (id: string, hovering: boolean) => void;
  }

  let {
    snapshot,
    error = null,
    burn = null,
    active = false,
    lastRefreshAt = Date.now(),
    focused = false,
    accent,
    countdown = true,
    onSelect,
    onHover,
  }: Props = $props();

  let w = $derived(snapshot.windows);
  let balanceLabel = $derived.by(() => {
    if (!w.balance) return null;
    return `${formatUsage(w.balance.total, "cny")}`;
  });

  // Ring/label flip: `countdown` true shows REMAINING, false shows USED.
  // Tone thresholds stay on used % regardless of display mode.
  let remaining = $derived(remainingPercent(snapshot));
  let usedPct = $derived(1 - remaining);
  let ringLabel = $derived(
    countdown
      ? `${Math.round(remaining * 100)}%`
      : `${Math.round(usedPct * 100)}%`,
  );
  let ringValue = $derived(countdown ? remaining : usedPct);
  let tone = $derived<"ok" | "warn" | "crit">(
    usedPct >= 0.95 ? "crit" : usedPct >= 0.8 ? "warn" : "ok",
  );

  // Dual-ring (feedback #8): providers with both a quota'd 5h window and a
  // longer window show TWO windows on one ring — inner = 5h short window,
  // outer = the long window (weekly, falling back to monthly). Providers
  // without a 5h window keep a single ring.
  let shortRemain = $derived(
    w.five_hour && w.five_hour.quota > 0 ? 1 - percent(w.five_hour) : null,
  );
  let longWin = $derived(
    (w.weekly && w.weekly.quota > 0
      ? w.weekly
      : w.monthly && w.monthly.quota > 0
        ? w.monthly
        : null),
  );
  let useDual = $derived(shortRemain !== null && longWin !== null);
  let longRemain = $derived(longWin ? 1 - percent(longWin) : null);
  let hasWeeklyQuota = $derived(Boolean(w.weekly && w.weekly.quota > 0));
  let dualLegend = $derived(
    useDual
      ? hasWeeklyQuota
        ? "内环：5 小时窗口 · 外环：周用量窗口"
        : "内环：5 小时窗口 · 外环：月度窗口"
      : "",
  );

  // Per-account accent → override CSS vars on the card root (fall back to the
  // global --tum-accent scheme when absent).
  let accentStyle = $derived.by(() => {
    if (!accent) return undefined;
    const rgb = hexToRgb(accent);
    if (!rgb) return undefined;
    return [
      `--acct-accent:${accent}`,
      `--acct-accent-stroke:rgba(${rgb},0.45)`,
      `--acct-accent-fill:rgba(${rgb},0.12)`,
      `--acct-accent-glow:rgba(${rgb},0.35)`,
    ].join(";");
  });

  let expanded = $state(false);
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events:
     whole-card quick-select (body click anchors focus+heatmap). Keyboard
     access is provided by the title button (see card__titlebtn), which also
     selects the provider. -->
<article
  class="card"
  class:card--expanded={expanded}
  class:card--focused={focused}
  style={accentStyle}
  data-tauri-drag-region={false}
  onpointerenter={() => onHover?.(snapshot.provider_id, true)}
  onpointerleave={() => onHover?.(snapshot.provider_id, false)}
  onclick={(e) => {
    // Title button already toggles expansion (and selects); a click
    // elsewhere anchors the header ring + heatmap to this provider.
    if ((e.target as HTMLElement).closest("button")) return;
    onSelect?.();
  }}
>
  <header class="card__head">
    <button
      type="button"
      class="card__titlebtn"
      aria-expanded={expanded}
      aria-label={`${snapshot.provider_display_name}，用量详情${active ? "（正在请求）" : ""}`}
      onclick={() => {
        expanded = !expanded;
        onSelect?.();
      }}
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
      <span class="card__ring" title={useDual ? dualLegend : undefined}>
        {#if useDual}
          <ProgressRing value={countdown ? shortRemain! : 1 - shortRemain!} outerValue={countdown ? longRemain! : 1 - longRemain!} label={ringLabel} size={36} stroke={4} {countdown} {accent} />
        {:else}
          <ProgressRing value={ringValue} label={ringLabel} size={36} stroke={4} {countdown} {accent} />
        {/if}
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

  .card--focused {
    border-color: var(--acct-accent-stroke, var(--tum-accent-stroke));
    box-shadow: inset 0 0 0 1px var(--acct-accent-stroke, var(--tum-accent-stroke));
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
    cursor: pointer;
  }

  .card:has(:focus-visible) {
    outline: 2px solid var(--acct-accent, var(--tum-accent));
    outline-offset: 2px;
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
    flex: none;
    white-space: nowrap;
    font-size: var(--tum-font-size-xs);
    font-weight: 500;
    color: var(--acct-accent, var(--tum-accent));
    background: var(--acct-accent-fill, var(--tum-accent-fill));
    border: 1px solid var(--acct-accent-stroke, var(--tum-accent-stroke));
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
