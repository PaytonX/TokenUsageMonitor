<script lang="ts">
  import { getHeatmap, getUsageHistory } from "../api";
  import { formatUsage, type HeatmapCell, type UsageUnit } from "../types";
  import {
    buildLedgerBreakdown,
    focusStats,
    hexToRgbTriplet,
    paintCell,
    type DayBreakdown,
    type PaintMode,
  } from "../calendar-linkage";
  import { providerColor, providerLabel } from "../model-provider";

  interface Props {
    /** provider 日账模式的数据键（volcengine/openai/xai 等真实服务端日账）。 */
    providerId?: string;
    /** 账本模式：'all' = 合并全部本机工具。设置了 ledger 时优先于 providerId。 */
    ledger?: "all" | string | null;
    /** P2 分层高亮：要高亮的 provider key；null = 不高亮（基线行为）。 */
    highlightKey?: string | null;
    /** Shown instead of the default empty hint. */
    emptyHint?: string;
    /** Unit fallback before the first fetch resolves. */
    unit?: UsageUnit;
  }

  let {
    providerId = "",
    ledger = null,
    highlightKey = null,
    emptyHint,
    unit = "tokens" as UsageUnit,
  }: Props = $props();

  const CAL_WEEKS = 26; // 近 6 个月的周列数（周一对齐）
  const CAL_DAYS = 200; // 与 CAL_WEEKS 匹配的取数天数

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

  $effect(() => {
    const seq = ++fetchSeq;
    loaded = false;
    cells = [];
    breakdown = new Map();
    const fetcher = ledger
      ? getUsageHistory(CAL_DAYS).then(({ rows }) => {
          // 一次性拆解：总量走来源总量行，provider 归因走分模型行。
          const map = buildLedgerBreakdown(rows);
          breakdown = map;
          return [...map.entries()]
            .sort((a, b) => a[0].localeCompare(b[0]))
            .map(([date, d]) => ({ date, value: d.total, unit: "tokens" as UsageUnit }));
        })
      : getHeatmap(providerId, CAL_DAYS);
    fetcher
      .then((rows) => {
        if (seq === fetchSeq) cells = rows;
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
    const today = new Date();
    const todayKey = localDateKey(today);
    const todayDow = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const start = new Date(today);
    start.setDate(today.getDate() - todayDow - (CAL_WEEKS - 1) * 7);
    const startKey = localDateKey(start);
    const windowCells = cells.filter((c) => c.date >= startKey && c.date <= todayKey);
    const windowDates = windowCells.map((c) => c.date);
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
          level: levelFor(value, windowCells),
          isToday: dateStr === todayKey,
        });
      }
      cols.push(col);
      const monday = new Date(start);
      monday.setDate(start.getDate() + wi * 7);
      const m = monday.getMonth();
      monthLabels.push(wi > 0 && m !== prevMonth ? `${m + 1}月` : null);
      prevMonth = m;
    }
    const total = windowCells.reduce((s, c) => s + c.value, 0);
    const peak = Math.max(0, ...windowCells.map((c) => c.value));
    const activeDays = windowCells.filter((c) => c.value > 0).length;
    // windowTotal 必须是**本机工具 token 口径**的总量（cells 来自 d.total，
    // 而 buildLedgerBreakdown 只收 kind==='tool' 行），否则 share 会跨单位失真。
    const stats = highlightKey
      ? focusStats(breakdown, windowDates, highlightKey, total)
      : null;
    return { cols, windowCells, windowDates, monthLabels, total, peak, activeDays, stats };
  });

  function levelFor(value: number, all: HeatmapCell[]): 0 | 1 | 2 | 3 | 4 {
    if (value <= 0) return 0;
    const max = Math.max(1, ...all.map((c) => c.value));
    const t = value / max;
    if (t < 0.25) return 1;
    if (t < 0.5) return 2;
    if (t < 0.75) return 3;
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

  /** tooltip：当日总量 + 按 provider 构成明细。 */
  function titleFor(day: NonNullable<Day>): string {
    const parts = Object.entries(breakdown.get(day.date)?.byProvider ?? {})
      .filter(([, v]) => v > 0)
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => `  ${providerLabel(k)} ${formatUsage(v, displayUnit)}`);
    return [`${day.date} · ${formatUsage(day.value, displayUnit)}`, ...parts].join("\n");
  }

  const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];
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
    <div class="heatmap__empty">{emptyHint ?? "该来源暂无热力图数据"}</div>
  {:else}
    <div class="cal">
      <div class="cal__top">
        <span class="cal__corner"></span>
        <div class="cal__months">
          {#each grid.monthLabels as label}
            <span class="cal__month" class:cal__month--has={!!label}>{label ?? ""}</span>
          {/each}
        </div>
      </div>
      <div class="cal__body">
        <div class="cal__weekdays">
          {#each WEEKDAY_LABELS as wd}
            <span class="cal__wd">{wd}</span>
          {/each}
        </div>
        <div class="cal__cols">
          {#each grid.cols as col}
            <div class="cal__col">
              {#each col as day}
                {#if day}
                  {@const p = paint(day)}
                  <div
                    class="cal__cell"
                    style={`background:${p.bg};box-shadow:${p.shadow}`}
                    title={titleFor(day)}
                  ></div>
                {:else}
                  <div class="cal__cell cal__cell--future"></div>
                {/if}
              {/each}
            </div>
          {/each}
        </div>
      </div>
      <div class="cal__foot">
        <div class="cal__stats">
          <span class="cal__stat">累计 <b>{formatUsage(grid.total, displayUnit)}</b></span>
          <span class="cal__stat">峰值 <b>{formatUsage(grid.peak, displayUnit)}</b></span>
          <span class="cal__stat">活跃 <b>{grid.activeDays} 天</b></span>
        </div>
        <div class="heatmap__legend">
          <span class="heatmap__legend-text">少</span>
          {#each legendColors as c}
            <span class="heatmap__legend-cell" style={`background:${c.bg}`}></span>
          {/each}
          <span class="heatmap__legend-text">多</span>
          {#if highlightKey}
            <span class="heatmap__legend-sep"></span>
            <span
              class="heatmap__legend-cell"
              style={`background:rgba(${brandRgb.join(",")},0.66);box-shadow:inset 0 0 0 1px rgba(${brandRgb.join(",")},0.95)`}
            ></span>
            <span class="heatmap__legend-text">{providerLabel(highlightKey)}</span>
          {/if}
        </div>
      </div>
    </div>
    {#if highlightKey && grid.stats}
      <!-- P2 追加行：总量恒定，焦点 provider 自己的口径单列一行 -->
      <div class="cal__stats cal__stats--focus">
        <span class="cal__stat">{providerLabel(highlightKey)} 活跃 <b>{grid.stats.days} 天</b></span>
        <span class="cal__stat">累计 <b>{formatUsage(grid.stats.sum, displayUnit)}</b></span>
        <span class="cal__stat">峰值 <b>{formatUsage(grid.stats.peak, displayUnit)}</b></span>
        <span class="cal__stat">占总量 <b>{grid.stats.share}%</b></span>
      </div>
    {/if}
  {/if}
</div>

<style>
  .heatmap {
    width: 100%;
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

  .cal__top {
    display: flex;
    gap: 3px;
    align-items: flex-end;
    height: 12px;
  }

  .cal__corner {
    width: 12px;
    flex: none;
  }

  .cal__months {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(26, 1fr);
    gap: 2px;
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

  .cal__body {
    display: flex;
    gap: 3px;
  }

  .cal__weekdays {
    width: 12px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cal__wd {
    height: 10px;
    line-height: 10px;
    font-size: 8px;
    text-align: center;
    color: var(--tum-text-muted);
    font-family: var(--tum-font);
  }

  .cal__cols {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(26, 1fr);
    gap: 2px;
  }

  .cal__col {
    display: flex;
    flex-direction: column;
    gap: 2px;
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
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 3px;
    flex-wrap: wrap;
  }

  .cal__stats {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 9px;
    color: var(--tum-text-muted);
  }

  .cal__stat b {
    margin-left: 2px;
    color: var(--tum-text-primary);
    font-family: var(--tum-font-mono);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .cal__foot .heatmap__legend {
    margin-top: 0;
  }

  .cal__stats--focus {
    margin-top: 3px;
    color: var(--tum-text-secondary);
  }
  .cal__stats--focus b {
    color: var(--tum-text-primary);
  }
</style>
