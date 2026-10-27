<script lang="ts">
  // 胶囊展开态（方案 A）：当前聚焦账户的 5h / 日 / 周 / 月窗口明细。
  //
  // 为什么单独成组件而不是复用 DetailCard：DetailCard 是 400px 全窗态的详情
  // 面板，带图表、成本、诊断等一整列内容；胶囊展开区只有 168px 宽、两百多
  // 像素高，需要的是同一套数据的最紧凑表达。两者共用的是**口径**（used/quota、
  // countdown 方向、状态色），不是布局。
  import { brandColorFor } from "../brand-glyphs";
  import { t } from "../i18n/store";
  import { formatResetIn } from "../reset-time";
  import {
    formatUsage,
    percent,
    type UsageSnapshot,
    type WindowKey,
    type WindowUsage,
  } from "../types";

  interface Props {
    snapshot: UsageSnapshot;
    /** true = 显示剩余百分比/剩余填充；false = 显示已用（与圆环同向）。 */
    countdown?: boolean;
    /** Wall-clock ms，由 App 的统一 1s tick 传入，保证三个重置倒计时同拍。 */
    now?: number;
    /** 账户强调色；ok 状态的进度条用它，其余状态用 warn/crit 色。 */
    accent?: string;
  }

  let { snapshot, countdown = true, now = Date.now(), accent }: Props = $props();

  const WINDOW_ORDER: WindowKey[] = ["five_hour", "daily", "weekly", "monthly"];
  const WINDOW_LABELS: Record<WindowKey, string> = {
    five_hour: "card.fiveHour",
    daily: "detail.poolNames.daily",
    weekly: "card.weeklyUsage",
    monthly: "detail.poolNames.monthly",
  };

  let rows = $derived.by(() =>
    WINDOW_ORDER.map((key) => ({ key, win: snapshot.windows[key] })).filter(
      (row): row is { key: WindowKey; win: WindowUsage } =>
        Boolean(row.win && row.win.quota > 0),
    ),
  );
  let plan = $derived(snapshot.plan_tier?.trim() ?? "");
  let brand = $derived(accent ?? brandColorFor(snapshot.provider_id.split("-")[0]));

  function usedFraction(win: WindowUsage): number {
    return percent(win);
  }

  /** 显示方向与胶囊圆环一致：countdown 时展示剩余，否则展示已用。 */
  function shownFraction(win: WindowUsage): number {
    return countdown ? 1 - usedFraction(win) : usedFraction(win);
  }

  function tone(win: WindowUsage): "ok" | "warn" | "crit" {
    const used = usedFraction(win);
    return used >= 0.95 ? "crit" : used >= 0.8 ? "warn" : "ok";
  }

  function barColor(win: WindowUsage): string {
    const tn = tone(win);
    return tn === "ok" ? brand : `var(--tum-${tn})`;
  }
</script>

<div class="pwd" role="group" aria-label={$t("app.pill.windowsTitle")}>
  <div class="pwd__head">
    <span class="pwd__name" title={snapshot.provider_display_name}>
      {snapshot.provider_display_name}
    </span>
    {#if plan}
      <span class="pwd__plan">{plan}</span>
    {/if}
  </div>

  {#if rows.length === 0}
    <p class="pwd__note">{$t("app.pill.noWindows")}</p>
  {:else}
    {#each rows as row (row.key)}
      {@const shown = shownFraction(row.win)}
      {@const toneName = tone(row.win)}
      <div class="pwd__row">
        <div class="pwd__top">
          <span class="pwd__label">{$t(WINDOW_LABELS[row.key])}</span>
          <span class="pwd__amount">
            {formatUsage(row.win.used, row.win.unit)} / {formatUsage(row.win.quota, row.win.unit)}
          </span>
          <span class="pwd__pct pwd__pct--{toneName}">{Math.round(shown * 100)}%</span>
        </div>
        <span class="pwd__track">
          <span
            class="pwd__fill"
            style={`width:${(shown * 100).toFixed(1)}%;background:${barColor(row.win)}`}
          ></span>
        </span>
        <span class="pwd__reset">
          {$t("app.pill.resetIn", { time: formatResetIn(row.win.reset_at, now) })}
        </span>
      </div>
    {/each}
    <p class="pwd__note">{$t("app.pill.windowsNote")}</p>
  {/if}
</div>

<style>
  .pwd {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 8px 2px 4px;
  }

  .pwd__head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 6px;
    min-width: 0;
  }

  .pwd__name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    color: var(--tum-text-primary);
  }

  .pwd__plan {
    flex: none;
    font-family: var(--tum-font-mono);
    font-size: 9px;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    color: var(--tum-text-muted);
  }

  .pwd__row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .pwd__top {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }

  .pwd__label {
    flex: none;
    font-family: var(--tum-font-mono);
    font-size: 9px;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    color: var(--tum-text-muted);
  }

  .pwd__amount {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    font-family: var(--tum-font-mono);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-secondary);
  }

  .pwd__pct {
    flex: none;
    min-width: 30px;
    text-align: right;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-primary);
  }

  .pwd__pct--ok {
    color: var(--tum-ok);
  }

  .pwd__pct--warn {
    color: var(--tum-warn);
  }

  .pwd__pct--crit {
    color: var(--tum-crit);
  }

  .pwd__track {
    display: block;
    height: 3px;
    border-radius: 2px;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.09);
  }

  .pwd__fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    transition: width var(--tum-dur-ring, 420ms) var(--tum-ease-ring);
  }

  .pwd__reset {
    font-family: var(--tum-font-mono);
    font-size: 8px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.2px;
    color: var(--tum-text-muted);
  }

  .pwd__note {
    font-family: var(--tum-font-mono);
    font-size: 9px;
    line-height: 1.45;
    color: var(--tum-text-muted);
  }
</style>
