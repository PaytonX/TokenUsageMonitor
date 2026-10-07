<script module lang="ts">
  // 供 CalendarSection 右侧统计栏渲染的聚合快照。随高亮焦点切换口径：
  // 总口径 = 全部来源；聚焦时 total/peak/activeDays 为该来源的窗口值。
  export interface HeatStats {
    focused: string | null;
    /** tokens 总量。 */
    total: number;
    peak: number;
    /** 峰值所在日本地日期（YYYY-MM-DD）；无数据时空串。 */
    peakDate: string;
    activeDays: number;
    /** 窗口自然日跨度（含零值日）。 */
    spanDays: number;
    /** 有用量的来源数。 */
    sourceCount: number;
    /** 今天（YYYY-MM-DD）。 */
    todayKey: string;
    /** 窗口范围标签（YYYY-MM ~ YYYY-MM）。 */
    rangeLabel: string;
    /** 聚焦时该来源占总量百分比；总口径为 null。 */
    focusSharePct: number | null;
    /** 全部来源占比（按 tokens，降序）。 */
    shares: { key: string; pct: number }[];
    unit: UsageUnit;
  }
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { getUsageHistory } from "../api";
  import { formatUsage, type HeatmapCell, type UsageUnit } from "../types";
  import {
    buildUnifiedBreakdown,
    focusStats,
    hexToRgbTriplet,
    paintCell,
    type DayBreakdown,
    type PaintMode,
  } from "../calendar-linkage";
  import { providerColor, providerLabel } from "../model-provider";
  import { t, locale } from "../i18n/store";

  interface Props {
    /** P2 分层高亮：要高亮的 provider key；null = 不高亮（基线行为）。 */
    highlightKey?: string | null;
    /** Shown instead of the default empty hint. */
    emptyHint?: string;
    /** Unit fallback before the first fetch resolves. */
    unit?: UsageUnit;
    /** 聚合快照出口：日历数据/高亮变化时回调一次（统计栏渲染用）。 */
    onStats?: (s: HeatStats) => void;
  }

  let {
    highlightKey = null,
    emptyHint,
    unit = "tokens" as UsageUnit,
    onStats,
  }: Props = $props();

  // 日历恒为「本机工具 + 服务端日账」的合并口径（buildUnifiedBreakdown，
  // 服务端优先）。不再有「全部工具 / 某账户日账」的模式开关——两套口径并排
  // 切换既让人分不清，当前者实测又几乎零重叠（服务端 20 天 vs 本机 GLM 4 天，
  // 交集仅 2 天且服务端量微不足道），合并不会双计。
  //
  // 取数天数与展示周数解耦：CAL_DAYS 覆盖取数窗口（含色阶分位所需的样本），
  // CAL_WEEKS 决定画几列。该值还写死在样式块的 grid-template-columns
  // （纯 CSS 读不到 JS 常量）；2026-10 起日历是单张网格，同步点只剩一处
  // repeat(26, …)。
  //
  // ⚠️ 本文件的 script 注释里**不得出现 Svelte 的块级标签**（style 块、script
  // 块的尖括号形式），哪怕裹在反引号里也不行。Svelte 解析器不认 JS 注释，
  // 会把它当成样式块起点，于是整份 script 被判为未闭合：症状是 vite build 能过，
  // 只有 svelte-check 报 "script was left open"，且定位落在文件末尾那个真正的
  // 闭合标签上，完全指不到出错的那一行。要提到标签就写 "style 块" 这种纯文字。
  const CAL_DAYS = 200; // 近 6 个月的取数天数
  const CAL_WEEKS = 26;

  type Day = {
    date: string;
    value: number;
    level: 0 | 1 | 2 | 3 | 4;
    isToday: boolean;
  } | null;

  let cells = $state<HeatmapCell[]>([]);
  let breakdown = $state<Map<string, DayBreakdown>>(new Map());
  let loaded = $state(false);
  let fetchSeq = 0;

  // 恒定口径 → 没有任何 props 依赖，effect 只在挂载时跑一次。
  // 存量守卫仍在：unified 换源时 in-flight 的旧请求不得覆盖新结果。
  $effect(() => {
    const seq = ++fetchSeq;
    loaded = false;
    cells = [];
    breakdown = new Map();
    getUsageHistory(CAL_DAYS)
      .then(({ rows }) => {
        // 一次性拆解：合并口径（服务端日账优先，逐 provider 判定），
        // 总量与 provider 归因来自同一次遍历，不再二选一。
        const map = buildUnifiedBreakdown(rows);
        if (seq !== fetchSeq) return;
        breakdown = map;
        return [...map.entries()]
          .sort((a, b) => a[0].localeCompare(b[0]))
          .map(([date, d]) => ({ date, value: d.total, unit: "tokens" as UsageUnit }));
      })
      .then((rows) => {
        if (rows && seq === fetchSeq) cells = rows;
      })
      .catch(() => {
        if (seq === fetchSeq) cells = [];
      })
      .finally(() => {
        if (seq === fetchSeq) loaded = true;
      });
  });

  function localDateKey(d: Date): string {
    const y = d.getFullYear();
    const m = `${d.getMonth() + 1}`.padStart(2, "0");
    const day = `${d.getDate()}`.padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  let byDate = $derived.by(() => {
    const map = new Map<string, HeatmapCell>();
    for (const c of cells) map.set(c.date, c);
    return map;
  });

  let paintMode = $derived<PaintMode>(highlightKey ? "highlight" : "total");
  let brandRgb = $derived(hexToRgbTriplet(providerColor(highlightKey ?? "minimax")));

  let grid = $derived.by(() => {
    // 月标注随界面语言：zh-CN → "3月"，en → "Mar"（Intl 对两种语言都给最
    // 自然的短格式，故月名不走词典键）。$locale 变化会触发本派生重算。
    const monthFmt = new Intl.DateTimeFormat($locale, { month: "short" });
    const today = new Date();
    const todayKey = localDateKey(today);
    const todayDow = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const start = new Date(today);
    start.setDate(today.getDate() - todayDow - (CAL_WEEKS - 1) * 7);
    const startKey = localDateKey(start);
    const windowCells = cells.filter((c) => c.date >= startKey && c.date <= todayKey);
    const windowDates = windowCells.map((c) => c.date);
    // 分位数分档的基准：窗口内所有非零值升序排列（GitHub contribution graph
    // 同款）。见 levelFor 的注释——线性分档在真实数据下会把几乎所有日子
    // 压进最暗的一档，热力图退化成一块死板色。
    const sortedNonZero = windowCells
      .map((c) => c.value)
      .filter((v) => v > 0)
      .sort((a, b) => a - b);
    const cols: Day[][] = [];
    const monthLabels: Array<string | null> = [];
    let prevMonth = -1;
    for (let wi = 0; wi < CAL_WEEKS; wi++) {
      const col: Day[] = [];
      for (let di = 0; di < 7; di++) {
        const d2 = new Date(start);
        d2.setDate(start.getDate() + wi * 7 + di);
        if (d2 > today) {
          col.push(null);
          continue;
        }
        const dateStr = localDateKey(d2);
        const value = byDate.get(dateStr)?.value ?? 0;
        col.push({
          date: dateStr,
          value,
          level: levelFor(value, sortedNonZero),
          isToday: dateStr === todayKey,
        });
      }
      cols.push(col);
      const monday = new Date(start);
      monday.setDate(start.getDate() + wi * 7);
      const m = monday.getMonth();
      monthLabels.push(wi > 0 && m !== prevMonth ? monthFmt.format(monday) : null);
      prevMonth = m;
    }
    const total = windowCells.reduce((s, c) => s + c.value, 0);
    const peak = Math.max(0, ...windowCells.map((c) => c.value));
    const activeDays = windowCells.filter((c) => c.value > 0).length;
    // share 的分母必须是**同一 merge 口径**下的窗口总量（cells 来自
    // d.total，而 breakdown 来自 buildUnifiedBreakdown，含服务端替换后的值）。
    // 两者同源，所以 share 是纯 token 比值，不会跨单位失真。
    const stats = highlightKey
      ? focusStats(breakdown, windowDates, highlightKey, total)
      : null;
    return { cols, windowCells, windowDates, monthLabels, total, peak, activeDays, stats, todayKey };
  });

  // 统计栏快照：来源占比按窗口逐日 breakdown 聚合（与格阵同源，token 纯比值）。
  // 峰值日：总口径取格阵 argmax，聚焦口径取该来源逐日 argmax。
  let heatStats = $derived.by(() => {
    const perKind = new Map<string, number>();
    for (const date of grid.windowDates) {
      const d = breakdown.get(date);
      if (!d) continue;
      for (const [k, v] of Object.entries(d.byProvider)) {
        if (v > 0) perKind.set(k, (perKind.get(k) ?? 0) + v);
      }
    }
    const grand = [...perKind.values()].reduce((a, b) => a + b, 0);
    const shares = [...perKind.entries()]
      .map(([key, total]) => ({ key, pct: grand > 0 ? (total / grand) * 100 : 0 }))
      .sort((a, b) => b.pct - a.pct);
    let peakDate = "";
    let best = 0;
    if (highlightKey) {
      for (const date of grid.windowDates) {
        const v = breakdown.get(date)?.byProvider[highlightKey] ?? 0;
        if (v > best) { best = v; peakDate = date; }
      }
    } else {
      for (const c of grid.windowCells) {
        if (c.value > best) { best = c.value; peakDate = c.date; }
      }
    }
    const last = grid.windowDates[grid.windowDates.length - 1] ?? "";
    return {
      focused: highlightKey,
      total: grid.stats ? grid.stats.sum : grid.total,
      peak: grid.stats ? grid.stats.peak : grid.peak,
      activeDays: grid.stats ? grid.stats.days : grid.activeDays,
      focusSharePct: grid.stats ? grid.stats.share : null,
      peakDate,
      spanDays: grid.windowDates.length,
      sourceCount: perKind.size,
      todayKey: grid.todayKey,
      rangeLabel: grid.windowDates.length
        ? `${grid.windowDates[0].slice(0, 7)} ~ ${last.slice(0, 7)}`
        : "",
      shares,
      unit: displayUnit,
    };
  });

  // 只依赖 heatStats：回调经 untrack 读取，父组件回调身份变化不会重触发，
  // 父组件 setState 也不会反过来再进本 effect（无回环）。
  $effect(() => {
    const snapshot = heatStats;
    untrack(() => onStats)?.(snapshot);
  });

  /**
   * 色阶分档：**分位数**（GitHub contribution graph 同款），不是线性。
   *
   * 线性分档（value / max）在真实数据下会失效：实测本机账本近 200 天里最大
   * 967M、中位仅 4M，跨度 240 倍，于是 95 个有效日中有 90 个落进最暗的 L1，
   * 整张热力图退化成一块看不出规律的色块。分位数分档保证每档大致各占 1/4，
   * 无论数据分布多偏。
   *
   * @param sortedNonZero 窗口内所有**非零**值，升序。
   */
  function levelFor(value: number, sortedNonZero: number[]): 0 | 1 | 2 | 3 | 4 {
    if (value <= 0) return 0;
    const n = sortedNonZero.length;
    if (n === 0) return 1;
    // 低于 25% 分位 → L1，50% → L2，75% → L3，其余 L4。
    // 用「小于该分位值的比例」判定，保证同值同档、且与样本量无关。
    const rank = sortedNonZero.filter((v) => v < value).length;
    const q = (p: number) => Math.max(1, Math.ceil(n * p));
    if (rank < q(0.25)) return 1;
    if (rank < q(0.5)) return 2;
    if (rank < q(0.75)) return 3;
    return 4;
  }

  function paint(day: NonNullable<Day>) {
    return paintCell({
      mode: paintMode,
      level: day.level,
      isToday: day.isToday,
      focusValue: highlightKey
        ? (breakdown.get(day.date)?.byProvider[highlightKey] ?? 0)
        : 0,
      brandRgb,
    });
  }

  /** tooltip：当日总量 + 按 provider 构成明细。provider 名是 i18n 键或品牌名，
   *  经 $t 取词（非键原样返回）；预构建进 $derived，语言切换后随派生重算，
   *  模板经 titleFor 读取保持响应式。 */
  let tooltips = $derived.by(() => {
    const map = new Map<string, string>();
    for (const [date, d] of breakdown) {
      const parts = Object.entries(d.byProvider)
        .filter(([, v]) => v > 0)
        .sort((a, b) => b[1] - a[1])
        .map(([k, v]) => `  ${$t(providerLabel(k))} ${formatUsage(v, displayUnit)}`);
      map.set(
        date,
        [`${date} · ${formatUsage(d.total, displayUnit)}`, ...parts].join("\n"),
      );
    }
    return map;
  });

  function titleFor(day: NonNullable<Day>): string {
    // breakdown 未覆盖的零值日：只有日期 + 0，无文案，无需取词。
    return tooltips.get(day.date) ?? `${day.date} · ${formatUsage(day.value, displayUnit)}`;
  }

  const WEEKDAY_KEYS = [
    "heat.mon",
    "heat.tue",
    "heat.wed",
    "heat.thu",
    "heat.fri",
    "heat.sat",
    "heat.sun",
  ];
  let legendColors = $derived(
    [1, 2, 3, 4].map((l) =>
      paintCell({ mode: "total", level: l as 1 | 2 | 3 | 4, isToday: false, focusValue: 0, brandRgb }),
    ),
  );
  let maxValue = $derived(Math.max(0, ...grid.windowCells.map((c) => c.value)));
  let displayUnit = $derived(grid.windowCells[0]?.unit ?? unit);
</script>

<div class="heatmap">
  {#if loaded && maxValue <= 0}
    <div class="heatmap__empty">{emptyHint ?? $t("heat.emptyNoData")}</div>
  {:else}
    <div class="cal">
      <!-- 单张网格：第 1 行放月份、第 1 列放星期，格与标签共用同一套行轨道，
           行高随格宽（aspect-ratio）浮动时对位恒齐（旧版标签定高 10px 会漂移）。 -->
      <div class="cal__grid">
        <span class="cal__corner"></span>
        {#each grid.monthLabels as label, ci}
          <span
            class="cal__month"
            class:cal__month--has={!!label}
            style={`grid-column:${ci + 2};grid-row:1`}
          >{label ?? ""}</span>
        {/each}
        {#each WEEKDAY_KEYS as wd, di}
          <span class="cal__wd" style={`grid-column:1;grid-row:${di + 2}`}>{$t(wd)}</span>
        {/each}
        {#each grid.cols as col, ci}
          {#each col as day, di}
            {#if day}
              {@const p = paint(day)}
              <div
                class="cal__cell"
                style={`grid-column:${ci + 2};grid-row:${di + 2};background:${p.bg};box-shadow:${p.shadow}`}
                title={titleFor(day)}
              ></div>
            {:else}
              <div
                class="cal__cell cal__cell--future"
                style={`grid-column:${ci + 2};grid-row:${di + 2}`}
              ></div>
            {/if}
          {/each}
        {/each}
      </div>
      <div class="cal__foot">
        <span class="cal__foot-meta">
          {$t("heat.footMeta", { n: heatStats.sourceCount, date: grid.todayKey.slice(5) })}
        </span>
        <div class="heatmap__legend">
          <span class="heatmap__legend-text">{$t("heat.less")}</span>
          {#each legendColors as c}
            <span class="heatmap__legend-cell" style={`background:${c.bg}`}></span>
          {/each}
          <span class="heatmap__legend-text">{$t("heat.more")}</span>
          {#if highlightKey}
            <span class="heatmap__legend-sep"></span>
            <span
              class="heatmap__legend-cell"
              style={`background:rgba(${brandRgb.join(",")},0.66);box-shadow:inset 0 0 0 1px rgba(${brandRgb.join(",")},0.95)`}
            ></span>
            <span class="heatmap__legend-text">{$t(providerLabel(highlightKey))}</span>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .heatmap {
    width: 100%;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .heatmap__empty {
    padding: var(--tum-space-4) 0;
    text-align: center;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
  }

  .heatmap__legend {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    /* 靠左排列，配合上方铺满的热力图网格 */
    justify-content: flex-start;
  }

  .heatmap__legend-cell {
    width: 10px;
    height: 10px;
    border-radius: var(--tum-radius-xs);
  }

  .heatmap__legend-text {
    font-size: 9px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
  }

  .heatmap__legend-sep {
    width: 1px;
    height: 12px;
    background: var(--tum-border-strong);
    margin: 0 4px;
  }

  /* ===== GitHub 式日历视图（近 6 个月）=====
     360px 面板：内容 328px。星期标签列 12px + 3px 间距，余下 26 周 ×
     ~10px 格（1fr + 2px gap），与截图的周列 + 月标注 + 统计布局一致。 */
  .cal {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  /* 单网格：12px 星期列 + 26 周列；行高由格子的 aspect-ratio 决定，
     星期标签 align-self:stretch + flex 居中，与所在行恒对齐。
     改 CAL_WEEKS 时只需同步这里一处 repeat(26, …)。 */
  .cal__grid {
    display: grid;
    grid-template-columns: 12px repeat(26, 1fr);
    grid-auto-rows: min-content;
    gap: 2px;
    align-items: start;
  }

  .cal__corner {
    grid-column: 1;
    grid-row: 1;
    width: 12px;
  }

  .cal__month {
    font-size: 9px;
    line-height: 12px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    white-space: nowrap;
    visibility: hidden;
  }

  .cal__month--has {
    visibility: visible;
  }

  .cal__wd {
    align-self: stretch;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 8px;
    line-height: 1;
    color: var(--tum-text-muted);
    font-family: var(--tum-font);
  }

  .cal__cell {
    width: 100%;
    aspect-ratio: 1;
    border-radius: 2px;
    cursor: default;
    transition: transform 0.15s ease;
  }

  .cal__cell:hover {
    transform: scale(1.5);
  }

  .cal__cell--future {
    background: transparent;
    border: none;
    pointer-events: none;
  }

  .cal__foot {
    margin-top: auto;
    padding-top: 8px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }

  .cal__foot-meta {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
  }

  .cal__foot .heatmap__legend {
    margin-top: 0;
  }

</style>
