<script lang="ts">
  import {
    formatUsage,
    hexToRgb,
    isPayAsYouGo,
    percent,
    remainingPercent,
    type BurnInfo,
    type UsageSnapshot,
  } from "../types";
  import UsageBar from "./UsageBar.svelte";
  import ResetCountdown from "./ResetCountdown.svelte";
  import ProgressRing from "./ProgressRing.svelte";
  import PulseDot from "./PulseDot.svelte";
  import ProviderLogo from "./ProviderLogo.svelte";
  import { brandColorFor, EXPERIMENTAL_KINDS } from "../brand-glyphs";
  import { freshnessLabel, freshnessTone } from "../freshness";

  interface Props {
    snapshot: UsageSnapshot;
    error?: string | null;
    burn?: BurnInfo | null;
    active?: boolean;
    lastRefreshAt?: number;
    /** Wall-clock ms, owned by App (it already runs one 1s tick for the
     *  countdown header). Passed in rather than re-ticked per card, so N cards
     *  still mean one timer. */
    now?: number;
    /** Poll period in seconds. Freshness buckets are multiples of this, so the
     *  badge adapts instead of hard-coding "5 minutes is stale". */
    pollIntervalSec?: number;
    /** Provider whose card is currently focused in the header ring. */
    focused?: boolean;
    /** Per-account accent (#RRGGBB). Injected as `--acct-accent*` CSS vars on
     *  the card root and passed to the ring; falls back to --tum-accent. */
    accent?: string;
    /** true = the ring/label show REMAINING; false = USED (see `countdown_mode`). */
    countdown?: boolean;
    /** Called when the user clicks this card; App links it to focus + heatmap. */
    onSelect?: () => void;
    /** Reported as the pointer enters/leaves the card; drives the bottom-docked
     *  detail overlay in App, which covers the calendar-heatmap area. No rect
     *  is needed: the overlay is docked to the window, not anchored to the card. */
    onHover?: (id: string, hovering: boolean) => void;
  }

  let {
    snapshot,
    error = null,
    burn = null,
    active = false,
    lastRefreshAt = Date.now(),
    now = Date.now(),
    pollIntervalSec = 300,
    focused = false,
    accent,
    countdown = true,
    onSelect,
    onHover,
  }: Props = $props();

  let w = $derived(snapshot.windows);
  // Brand/logo normalization mirrors App.svelte header avatars: the first
  // segment of a credential-scoped provider_id is the provider kind.
  let kind = $derived(snapshot.provider_id.split("-")[0]);
  // 实验性来源由 kind 推导；集合与 brand-glyphs 的 EXPERIMENTAL_KINDS 同源，
  // 避免 ProviderCard 与焦点胶囊两处各维护一份而漂移。
  let isExperimental = $derived(EXPERIMENTAL_KINDS.has(kind));
  // 实验小标：品牌色 22% 底（与焦点胶囊的 .ps__exp 同口径）。色源与卡片其余
  // 品牌视觉一致：账户自定义色优先，否则用品牌注册表的品牌色。
  const expStyle = $derived.by(() => {
    const rgb = hexToRgb(accent ?? brandColorFor(kind));
    return rgb ? `background: rgba(${rgb}, 0.22)` : "";
  });
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
  // 按量付费 provider：无配额窗口，百分比无意义，不渲染圆环（余额已由
  // 头部 balance 展示，本月消费由底部 UsageBar 展示，避免信息重复）。
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

  // 数据新鲜度：卡片画的是上次轮询的结果，屏幕上的数字在到达时就已经定格。
  // 错误角标要等下一次轮询失败才亮，这中间的空窗里用户无从判断数字有多旧。
  // 仅在离开 fresh 档时渲染——平时不占视觉预算，数字开始变旧才提示。
  let freshAgeMs = $derived(Math.max(0, now - lastRefreshAt));
  let freshTone = $derived(freshnessTone(freshAgeMs, pollIntervalSec));
  let freshLabel = $derived(freshnessLabel(freshAgeMs));
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
      <span class="card__status">
        <span class="card__halo" class:card__halo--on={active} aria-hidden="true"></span>
        <span class="card__logo"><ProviderLogo {kind} size={32} accent={accent ?? null} /></span>
        <span class="card__status-dot" title={active ? "正在请求" : undefined}>
          <PulseDot {active} {tone} size={8} />
        </span>
      </span>
      <span class="card__name">{snapshot.provider_display_name}</span>
      {#if isExperimental}
        <span class="card__exp" style={expStyle} title="实验性支持：数据可能不完整或口径调整中">实验</span>
      {/if}
    </button>
    {#if snapshot.plan_tier}
      <span class="card__tier">{snapshot.plan_tier}</span>
    {/if}
    <div class="card__head-right">
      {#if freshTone !== "fresh"}
        <span class="card__fresh" data-tone={freshTone} title={`用量数据更新于 ${freshLabel}`}
          >{freshLabel}</span
        >
      {/if}
      {#if balanceLabel && !isPayAsYouGo(snapshot)}
        <span class="card__balance" title="账户余额">{balanceLabel}</span>
      {/if}
      <span class="card__ring" title={useDual ? dualLegend : undefined}>
        {#if !isPayAsYouGo(snapshot)}
          {#if useDual}
            <ProgressRing value={countdown ? shortRemain! : 1 - shortRemain!} outerValue={countdown ? longRemain! : 1 - longRemain!} label={ringLabel} size={36} stroke={4} {countdown} {accent} />
          {:else}
            <ProgressRing value={ringValue} label={ringLabel} size={36} stroke={4} {countdown} {accent} />
          {/if}
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
    {#if isPayAsYouGo(snapshot)}
      <!-- 按量付费：统一对齐金额块（账户余额 / 本月消费），等宽右对齐。
           替代头部的余额与底部的月度条，避免信息重复。 -->
      {#if balanceLabel}
        <div class="card__money-row">
          <span class="card__money-label">账户余额</span>
          <span class="card__money-value" title="账户余额">{balanceLabel}</span>
        </div>
      {/if}
      {#if w.monthly}
        <div class="card__money-row">
          <span class="card__money-label">本月消费</span>
          <span class="card__money-value">{formatUsage(w.monthly.used, w.monthly.unit)}</span>
        </div>
      {/if}
    {/if}
    {#if !isPayAsYouGo(snapshot) && w.five_hour}
      <UsageBar usage={w.five_hour} label="5 小时" />
      {#if w.five_hour.reset_at}
        <ResetCountdown resetAt={w.five_hour.reset_at} label="5h" />
      {/if}
    {/if}
    {#if !isPayAsYouGo(snapshot) && w.weekly}
      <UsageBar usage={w.weekly} label="周用量" />
      {#if w.weekly.reset_at}
        <ResetCountdown resetAt={w.weekly.reset_at} label="周" />
      {/if}
    {/if}
    {#if !isPayAsYouGo(snapshot) && w.monthly}
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

  .card__status {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 36px;
    height: 36px;
  }

  .card__logo {
    position: relative;
    z-index: 1;
    display: inline-flex;
  }

  /* 活跃辉光：仅在 active（本轮轮询检测到用量增长）时点亮，主题色 =
     账户强调色（走 --acct-accent-glow，未设则回退全局 accent）。动效与
     ProgressRing 的 ring-breathe 同一套语言：2600ms 呼吸。闲置时完全
     隐藏，不占视觉预算；prefers-reduced-motion 由 tokens.css 全局降级。 */
  .card__halo {
    position: absolute;
    inset: -4px;
    border-radius: 50%;
    background: radial-gradient(
      circle,
      var(--acct-accent-glow, var(--tum-accent-glow)) 0%,
      transparent 70%
    );
    opacity: 0;
    z-index: 0;
    pointer-events: none;
  }

  .card__halo--on {
    animation: logo-breathe 2600ms ease-in-out infinite;
  }

  @keyframes logo-breathe {
    0%,
    100% {
      opacity: 0.3;
      transform: scale(0.9);
    }
    50% {
      opacity: 0.95;
      transform: scale(1.08);
    }
  }

  /* 状态角标：大 Logo 接管左侧主视觉后，ok/warn/crit 信号缩为右上角小点；
     1px 同色底衬保证在 tile/剪影两种 logo 形态上都可读。 */
  .card__status-dot {
    position: absolute;
    right: -3px;
    top: -3px;
    z-index: 2;
    display: inline-flex;
    border-radius: 50%;
    background: var(--tum-bg-solid);
    padding: 1px;
  }

  .card__exp {
    flex: none;
    white-space: nowrap;
    font-size: var(--tum-font-size-xs);
    font-weight: 600;
    /* 品牌色 22% 底由内联 style 注入（色源 = 账户强调色 ?? 品牌色，与 .ps__exp
       同口径）；hex 解析失败时退回 --tum-surface-hover。圆角 6（spec 决策 12）。 */
    color: var(--acct-accent, var(--tum-accent));
    background: var(--tum-surface-hover);
    border: none;
    padding: 1px 5px;
    border-radius: 6px;
    letter-spacing: 0.5px;
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

  /* 按量付费统一对齐金额块：两行 label/value 右对齐，等宽数字 */
  .card__money-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--tum-space-2);
  }

  .card__money-label {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    letter-spacing: 0.3px;
    text-transform: uppercase;
  }

  .card__money-value {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-primary);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.3px;
  }

  /* 数据新鲜度角标：只在离开 fresh 档时出现，是提示而非装饰，因此压低
     对比度，色阶只用来区分“有点旧”到“可能已失效”。 */
  .card__fresh {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .card__fresh[data-tone="recent"] {
    color: var(--tum-text-secondary);
  }

  .card__fresh[data-tone="stale"] {
    color: var(--tum-text-muted);
  }

  .card__fresh[data-tone="expired"] {
    color: var(--tum-text-muted);
    opacity: 0.75;
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
