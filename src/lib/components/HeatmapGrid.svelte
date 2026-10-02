<script lang="ts">
  import { getHeatmap, getUsageHistory } from "../api";
  import { formatUsage, type HeatmapCell, type UsageUnit } from "../types";

  interface Props {
    /** provider 日账模式的数据键（volcengine/openai/xai 等真实服务端日账）。 */
    providerId?: string;
    /** 账本模式：'all' = 合并全部本机工具；工具 id = 仅该工具。
     *  设置了 ledger 时优先于 providerId（统一账本口径）。 */
    ledger?: "all" | string | null;
    /** Shown instead of the default empty hint (e.g. DeepSeek ramp-up note). */
    emptyHint?: string;
    /** Unit fallback before the first fetch resolves; fetched cells carry
     *  their own unit and take precedence. */
    unit?: UsageUnit;
    /** "compact" = 近 31 天 5×7 网格；"calendar" = GitHub 式近 6 个月日历。 */
    view?: "compact" | "calendar";
  }

  let {
    providerId = "",
    ledger = null,
    emptyHint,
    unit = "tokens" as UsageUnit,
    view = "compact",
  }: Props = $props();

  const WEEKS = 5; // compact: 5 Monday-aligned columns cover the last 31 days
  const CAL_WEEKS = 26; // calendar: ~6 months of Monday-aligned weeks

  type Day =
    | {
        date: string;
        value: number;
        level: 0 | 1 | 2 | 3 | 4;
        isToday: boolean;
      }
    | null;

  let cells = $state<HeatmapCell[]>([]);
  let loaded = $state(false);
  let fetchSeq = 0;

  $effect(() => {
    const days = view === "calendar" ? 200 : 31;
    const seq = ++fetchSeq;
    loaded = false;
    cells = [];
    const fetcher = ledger
      ? getUsageHistory(days).then(({ rows }) =>
          rows
            .filter(
              (r) =>
                r.kind === "tool" &&
                r.model === "" &&
                (ledger === "all" || r.source === ledger),
            )
            .map((r) => ({
              date: r.date,
              value: r.total,
              unit: "tokens" as UsageUnit,
            })),
        )
      : getHeatmap(providerId, days);
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

  // Visible Monday..today window. The IPC snapshot branch ignores `days` and
  // may embed up to 200 days, so levels and the empty-state check normalize on
  // this window only (mirrors DetailCard's 7-day projection). One derived
  // keeps the clock and payload in lockstep — it recomputes whenever cells
  // changes (e.g. when a tab-switch fetch resolves on the long-lived instance).
  let grid = $derived.by(() => {
    const today = new Date();
    const todayKey = localDateKey(today);
    const todayDow = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const weeks = view === "calendar" ? CAL_WEEKS : WEEKS;
    const start = new Date(today);
    start.setDate(today.getDate() - todayDow - (weeks - 1) * 7);
    const startKey = localDateKey(start);
    const windowCells = cells.filter(
      (c) => c.date >= startKey && c.date <= todayKey,
    );
    // Column = week, row = Mon..Sun (ISO). Past days missing from the payload
    // render as transparent level-0 cells; future days render null (clear).
    const cols: Day[][] = [];
    const monthLabels: Array<string | null> = [];
    let prevMonth = -1;
    for (let wi = 0; wi < weeks; wi++) {
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
      // 月份标注：每周一的月份相对上一周变化时，在该周上方标 “N月”
      // （GitHub 日历风格；首列不标，避免与左侧星期标签挤在一起）。
      const monday = new Date(start);
      monday.setDate(start.getDate() + wi * 7);
      const m = monday.getMonth();
      monthLabels.push(wi > 0 && m !== prevMonth ? `${m + 1}月` : null);
      prevMonth = m;
    }
    // 统计行（截图规格）：累计 / 单日峰值 / 活跃天数。
    const total = windowCells.reduce((s, c) => s + c.value, 0);
    const peak = Math.max(0, ...windowCells.map((c) => c.value));
    const activeDays = windowCells.filter((c) => c.value > 0).length;
    return { cols, windowCells, monthLabels, total, peak, activeDays };
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

  // Accent-blue ramp (legacy amber survives only on the "today" stroke).
  // Level 0 = no usage: a muted light gray that stays visible against the dark
  // panel (so empty cells are still discernible as a grid), instead of black.
  function levelColor(level: 0 | 1 | 2 | 3 | 4): string {
    switch (level) {
      case 0:
        return "rgba(148, 163, 184, 0.16)";
      case 1:
        return "rgba(76,194,255,0.22)";
      case 2:
        return "rgba(76,194,255,0.42)";
      case 3:
        return "rgba(76,194,255,0.66)";
      case 4:
        return "rgba(76,194,255,0.94)";
    }
  }

  function cellShadow(day: NonNullable<Day>): string {
    const parts: string[] = [];
    if (day.level === 4) parts.push("0 0 4px rgba(76,194,255,0.45)");
    if (day.isToday) parts.push("inset 0 0 0 1.5px var(--tum-amber)");
    return parts.length > 0 ? parts.join(",") : "none";
  }

  const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];
  const legendLevels: Array<1 | 2 | 3 | 4> = [1, 2, 3, 4];
  let legendColors = $derived(legendLevels.map((l) => levelColor(l)));
  let maxValue = $derived(Math.max(0, ...grid.windowCells.map((c) => c.value)));
  let displayUnit = $derived(grid.windowCells[0]?.unit ?? unit);
</script>

<div class="heatmap" class:heatmap--cal={view === "calendar"}>
  {#if loaded && maxValue <= 0}
    <div class="heatmap__empty">{emptyHint ?? "该来源暂无热力图数据"}</div>
  {:else if view === "calendar"}
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
                  <div
                    class="cal__cell"
                    style={`background:${levelColor(day.level)};box-shadow:${cellShadow(day)}`}
                    title={`${day.date} · ${formatUsage(day.value, displayUnit)}`}
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
          {#each legendColors as color}
            <span class="heatmap__legend-cell" style={`background:${color}`}></span>
          {/each}
          <span class="heatmap__legend-text">多</span>
        </div>
      </div>
    </div>
  {:else}
    <div class="heatmap__cols">
      {#each grid.cols as col}
        <div class="heatmap__col">
          {#each col as day}
            {#if day}
              <div
                class="heatmap__cell"
                style={`background:${levelColor(day.level)};box-shadow:${cellShadow(day)}`}
                title={`${day.date} · ${formatUsage(day.value, displayUnit)}`}
              ></div>
            {:else}
              <div class="heatmap__cell heatmap__cell--future"></div>
            {/if}
          {/each}
        </div>
      {/each}
    </div>
    <div class="heatmap__legend">
      <span class="heatmap__legend-text">少</span>
      {#each legendColors as color}
        <span class="heatmap__legend-cell" style={`background:${color}`}></span>
      {/each}
      <span class="heatmap__legend-text">多</span>
    </div>
  {/if}
</div>

<style>
  .heatmap {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .heatmap__cols {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 4px;
    justify-items: center;
  }

  .heatmap__col {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .heatmap__cell {
    width: 14px;
    height: 14px;
    border-radius: var(--tum-radius-xs);
    /* 空档由 level-0 的浅灰填充体现，不再用虚线描边 */
    cursor: default;
    transition: transform 0.15s ease;
  }

  .heatmap__cell:hover {
    transform: scale(1.4);
  }

  .heatmap__cell--future {
    background: transparent;
    border: none;
    pointer-events: none;
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
</style>
