<script lang="ts">
  import { getUsageHistory, forceRefresh, loadInstanceKinds } from "../api";
  import { t, locale } from "../i18n/store";
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
  import { buildUnifiedBreakdown, ledgerSeriesForKind } from "../calendar-linkage";
  import { isPercentUnit } from "../quota-usage";
  import { fetchQuotaGroups } from "../trend-data";
  import ProviderLogo from "./ProviderLogo.svelte";
  import Icon from "./atoms/Icon.svelte";

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

  // 窗口名 → i18n 键。five_hour / daily / monthly 的中文与 settings.ring.*
  // 完全一致，直接复用；weekly 这里是「本周窗口」（settings.ring.weekly 是
  // 「周用量窗口」），单独给 detail.windowNames.weekly。模板经 $t 渲染。
  const WINDOW_LABELS: Record<WindowKey, string> = {
    five_hour: "settings.ring.fiveHour",
    daily: "settings.ring.daily",
    weekly: "detail.windowNames.weekly",
    monthly: "settings.ring.monthly",
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
    return new Date(at).toLocaleString($locale === "en" ? "en-US" : "zh-CN", {
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
    if (secs < 60) return $t("detail.agoSeconds", { n: secs });
    const mins = Math.floor(secs / 60);
    if (mins < 60) return $t("detail.agoMinutes", { n: mins });
    return $t("detail.agoHours", { n: Math.floor(mins / 60) });
  });

  // --- 近 7 日迷你柱状：统一账本口径 ---
  // 数据源选择交给 calendar-linkage（唯一数据源），本组件不再维护映射表。
  // 旧实现有两处问题：① 自带一份 LEDGER_TOOL_FOR_KIND，与 App 侧不一致
  // （App 有 deepseek，DetailCard 没有）—— 正是 calendar-linkage 要消灭的漂移；
  // ② 按 source === 工具 id 过滤，漏掉跨工具归因的行，与日历高亮的口径不同。
  // 现直接复用 buildUnifiedBreakdown，与日历/趋势页完全同一口径。
  //
  // 「无按日数据」的判据是**账本里近 90 天是否真的出现过这个 provider**，
  // 而不是注册表里有没有它的 key：注册表回答的是"能不能归因到它"，可一个
  // 配了账户却从不经被扫描工具使用的 Provider（如 Kimi 账号）映射非空、
  // 序列却恒为空——只判 null 会漏掉这类最常见的"点了没反应"。
  const PROBE_DAYS = 90; // 存在性探测窗口；小图仍只画近 7 天
  let weekCells = $state<HeatmapCell[]>([]);
  let weekSeq = 0;
  let chartNote = $state<string | null>(null);

  /**
   * 百分比额度制（OpenCode Go / MiniMax Token Plan）的额度摘要。
   *
   * 这是全应用里**唯一**呈现 percent 用量的地方（docs/adr/0001 的 D3）：
   * 趋势页是纯 token 页面，额度不占趋势页版面，只在「这个账户现在用了多少」
   * 的语境下出现——也就是这张卡片详情。
   *
   * 多 Key 的统一口径也在这里落地：`groupQuotaSeries` 用订阅指纹把同一份
   * 订阅下的多个 Key 去重（同订阅的 Key 在服务端看到同一批 reset_at，也看到
   * 同一个已用百分比，相加就是三倍），不同订阅按容量取均值。所以 `keys` 与
   * `subscriptions` 分开记：3 个 Key / 1 份订阅 = 一份用量，不是三份。
   */
  interface QuotaSummary {
    /** 本自然月已用（容量归一后的百分点）。 */
    monthTotal: number;
    /** 今日增量（本账户）。 */
    today: number;
    /** 有非零记录的天数（用来判断"这个数字有几天样本撑腰"）。 */
    sampleDays: number;
    /** 参与合并的 Key 数。 */
    keys: number;
    /** 去重后的订阅份数 = 容量倍数。 */
    subscriptions: number;
  }
  let quota = $state<QuotaSummary | null>(null);

  $effect(() => {
    const providerId = snapshot.provider_id;
    const kind = providerId.split("-")[0];
    const series = ledgerSeriesForKind(kind, providerId);
    const seq = ++weekSeq;
    quota = null; // 换卡片时先清掉上一张的额度摘要，避免看到别人的数字
    if (!series) {
      // 没有 token 日序列的 Provider 分两种：
      // ① 账本里从来没有它（配了账户却没被扫描工具用过）；
      // ② 百分比额度制（OpenCode Go / MiniMax Token Plan）——服务端只报已用
      //    额度百分比，日值以 unit='percent' 落在账本里（见 quota-usage 文件头）。
      // ②以前会落到「无按日数据」，但其实有数据，只是单位不是 token。所以先按
      // 单位探一次：有 percent 行就画「%/日」柱（标题里写明 %，不会被读成 token），
      // 一条都没有才退回原来的说明文案。
      void Promise.all([getUsageHistory(PROBE_DAYS), fetchQuotaGroups(PROBE_DAYS)])
        .then(([{ rows }, groups]) => {
          if (seq !== weekSeq) return;
          const picked = rows.filter(
            (r) =>
              r.kind === "provider" &&
              r.source === providerId &&
              isPercentUnit(r.unit),
          );
          if (picked.length === 0) {
            chartNote = $t("detail.noDailyLedger");
            weekCells = [];
            return;
          }
          chartNote = $t("detail.percentOnlyNote");
          // 补齐 7 天（缺数据的日期画 0 柱），与跨工具分支同样连续。
          const byDate = new Map(picked.map((r) => [r.date, r.total]));
          const today = localDateKey(new Date());
          weekCells = lastNDays(7).map((date) => ({
            date,
            value: byDate.get(date) ?? 0,
            unit: "percent" as UsageUnit,
          }));
          // 多 Key 统一由 quota-usage 负责：同订阅的 Key 共享 reset_at，去重后
          // 只算一次，多份订阅按容量取均值（详见该文件头）。这里只取本 kind。
          const group = groups.find((g) => g.kind === kind) ?? null;
          quota = {
            monthTotal: group?.monthTotal ?? 0,
            today: byDate.get(today) ?? 0,
            sampleDays: picked.filter((r) => (Number(r.total) || 0) > 0).length,
            keys: group?.keys ?? 1,
            subscriptions: group?.subscriptions ?? 1,
          };
        })
        .catch(() => {
          if (seq === weekSeq) weekCells = [];
        });
      return;
    }
    chartNote = null;
    getUsageHistory(PROBE_DAYS)
      .then(({ rows }) => loadInstanceKinds().then((kindByInstance) => ({ rows, kindByInstance })))
      .then(({ rows, kindByInstance }) => {
        if (seq !== weekSeq) return;
        if (series.mode === "account-daily") {
          // 服务端日账：单位可能是 tokens / usd / cny / afp，保留行上单位。
          const picked = rows.filter(
            (r) => r.kind === "provider" && r.source === series.providerId,
          );
          if (picked.length === 0) {
            chartNote = $t("detail.noServerDailyLedger");
            weekCells = [];
            return;
          }
          weekCells = picked
            .filter((r) => r.date >= lastNDays(7)[0])
            .map((r) => ({
              date: r.date,
              value: r.total,
              unit: (r.unit === "usd"
                ? "usd"
                : r.unit === "cny"
                  ? "cny"
                  : "tokens") as UsageUnit,
            }));
          return;
        }
        // 跨工具：与日历高亮同一拆解（合并口径，服务端优先），口径一致。
        // 账户映射让经路由的行按计费账户而非模型名归因。
        const map = buildUnifiedBreakdown(rows, kindByInstance);
        const key = series.providerKey;
        const appears = [...map.values()].some((d) => (d.byProvider[key] ?? 0) > 0);
        if (!appears) {
          chartNote = $t("detail.noUsageIn90d");
          weekCells = [];
          return;
        }
        weekCells = lastNDays(7).map((date) => ({
          date,
          value: map.get(date)?.byProvider[key] ?? 0,
          unit: "tokens" as UsageUnit,
        }));
      })
      .catch(() => {
        if (seq === weekSeq) weekCells = [];
      });
  });

  /** 最近 n 天的日期键（升序，含今天），本地时区。 */
  function lastNDays(n: number): string[] {
    const out: string[] = [];
    const today = new Date();
    for (let i = n - 1; i >= 0; i--) {
      const d = new Date(today);
      d.setDate(today.getDate() - i);
      out.push(localDateKey(d));
    }
    return out;
  }

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

  /** 柱图 tooltip 的数值：百分比保留一位小数——四舍五入到整数会让 0.4% 变成
   *  「0%」，也容易被读成 token 计数；其余单位沿用 formatUsage。 */
  function barValueLabel(value: number): string {
    return barUnit === "percent"
      ? `${value.toFixed(1)}%`
      : formatUsage(value, barUnit);
  }

  // Quota pool (额度池): every quota'd window with its used/quota amounts +
  // the used/remaining fraction depending on `countdown`.
  // label 存的是 i18n 键（5 小时/周用量复用 card.*，其余用 detail.poolNames.*），
  // 模板里经 $t 渲染。
  let quotaRows = $derived.by(() => {
    const wins: { label: string; win: WindowUsage }[] = [];
    const push = (key: WindowKey, label: string) => {
      const w = snapshot.windows[key];
      if (w && w.quota > 0) wins.push({ label, win: w });
    };
    push("five_hour", "card.fiveHour");
    push("daily", "detail.poolNames.daily");
    push("weekly", "card.weeklyUsage");
    push("monthly", "detail.poolNames.monthly");
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
      {critical ? $t(WINDOW_LABELS[critical.key]) : $t("detail.noWindow")}
    </span>
  </div>

  {#if quotaRows.length > 0}
    <div class="detail__pool">
      <span class="detail__pool-label">{$t("detail.poolLabel")}</span>
      {#each quotaRows as row (row.label)}
        <div class="detail__pool-row">
          <span class="detail__pool-name">{$t(row.label)}</span>
          <span class="detail__pool-amount">{row.amount}</span>
          <span class="detail__pool-pct">{row.pctLabel}</span>
        </div>
      {/each}
      {#if critical?.window.reset_at}
        <div class="detail__pool-row">
          <span class="detail__pool-name">{$t("countdown.reset")}</span>
          <span class="detail__pool-amount">{$t("detail.nextWindow")}</span>
          <span class="detail__pool-pct">{resetLabel}</span>
        </div>
      {/if}
    </div>
  {/if}

  <dl class="detail__rows">
    {#if costLabel}
      <dt>{$t("detail.estimatedCost")}</dt>
      <dd>
        <span class="detail__cost">{costLabel}</span>
        <span class="detail__tag">{$t("detail.estimatedTag")}</span>
      </dd>
    {/if}
    <dt>{$t("detail.burnRate")}</dt>
    <dd>{burnLabel}</dd>
    <dt>{$t("detail.eta")}</dt>
    <dd>{etaLabel}</dd>
    <dt>{$t("detail.currentReset")}</dt>
    <dd>{resetLabel}</dd>
    <dt>{$t("detail.lastRefresh")}</dt>
    <dd>{refreshedLabel}</dd>
    {#if error}
      <dt class="detail__err-label">{$t("detail.diagnostics")}</dt>
      <dd class="detail__err">{error}</dd>
    {/if}
  </dl>

  {#if quota}
    <!-- 额度摘要：百分比制订阅的「本期已用」出口（唯一入口，见文件头注释）。
         三个数字各有明确口径：本月已用 = 自然月内每日增量的和（月度限额制
         订阅的当期用量）；今日增量 = 与柱图最后一根同源；样本天数用来提示
         这个数字背后有多少天观测，样本少时不要当成稳定趋势读。 -->
    <div class="detail__quota">
      <span class="detail__quota-label">{$t("quota.monthToDate")}</span>
      <span class="detail__quota-value">{quota.monthTotal.toFixed(1)}%</span>
      <span class="detail__quota-label">{$t("quota.today")}</span>
      <span class="detail__quota-value">{quota.today.toFixed(1)}%</span>
      <span class="detail__quota-meta">
        {$t("quota.sampleDaysLabel")}
        {quota.sampleDays}{#if quota.keys > 1 || quota.subscriptions > 1}
          ·{$t("quota.keySubNote", {
            keys: quota.keys,
            subs: quota.subscriptions,
          })}{/if}
      </span>
    </div>
  {/if}

  <div class="detail__chart">
    <!-- 百分比额度制的 Provider 没有 token 日序列，柱图的单位随之换成 %
         （标题直接写「%/日」），保证不会被当成本日 token 用量读。 -->
    <span class="detail__chart-label">
      {$t(
        barUnit === "percent" ? "detail.last7DaysPercent" : "detail.last7Days",
      )}
    </span>
    {#if chartNote}
      <p class="detail__chart-note">{chartNote}</p>
    {/if}
    <div class="detail__bars">
      {#each bars as b (b.date)}
        <div class="detail__bar-wrap" title={`${b.date} · ${barValueLabel(b.value)}`}>
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
      title={$t("detail.refreshThisTitle")}
    ><Icon name="refresh" size={12} /> {$t("detail.refreshThis")}</button>
  </div>
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-radius: var(--tum-radius-md);
    border: 1px solid var(--tum-border-strong);
    /* 玻璃外壳（2026-10-06 原型拍板）：径向强调色微光 + 半透明深底 + 背景模糊，
       对齐 tum-glass 语言；backdrop-filter 只采样窗内内容（透明窗对窗下桌面无效，
       属预期）。参数：blur 走 --tum-blur-card，底色 α0.78。 */
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
      rgba(24, 26, 30, 0.78);
    backdrop-filter: blur(var(--tum-blur-card));
    -webkit-backdrop-filter: blur(var(--tum-blur-card));
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
    /* 内联 flex 纵向居中：SVG 无内联基线，默认 baseline 对齐会相对文字上浮 */
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font);
    color: var(--tum-text-secondary);
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s ease, transform var(--tum-dur-fast, 150ms) var(--tum-ease-spring);
  }

  .detail__btn:active {
    transform: scale(0.94);
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

  /* 额度摘要（percent 制）：两枚数字并排，样本/多 Key 说明折行在下面。
     刻意不做进度条——详情卡里已经有窗口环与柱图，再加一条进度会被当成
     "还能用多少"的第二种说法。 */
  .detail__quota {
    display: grid;
    grid-template-columns: auto auto auto auto;
    align-items: baseline;
    gap: 2px 8px;
    padding: 5px 8px;
    background: var(--tum-surface);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-sm, 4px);
    font-family: var(--tum-font-mono);
  }

  .detail__quota-label {
    font-size: 9px;
    color: var(--tum-text-muted);
    letter-spacing: 0.8px;
    text-transform: uppercase;
  }

  .detail__quota-value {
    font-size: var(--tum-font-size-sm);
    font-variant-numeric: tabular-nums;
    color: var(--tum-text);
  }

  .detail__quota-meta {
    grid-column: 1 / -1;
    font-size: 9px;
    color: var(--tum-text-muted);
    line-height: 1.45;
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

  /* 差分类 provider 无按日数据：说明占柱图位置，不用空白让用户猜。 */
  .detail__chart-note {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    line-height: 1.5;
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
    background: var(--tum-surface-hover);
  }
</style>
