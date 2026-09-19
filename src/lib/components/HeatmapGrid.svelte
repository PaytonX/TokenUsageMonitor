<script lang="ts">
  import { getHeatmap } from "../api";
  import { formatUsage, type HeatmapCell, type UsageUnit } from "../types";

  interface Props {
    providerId: string;
    /** Shown instead of the default empty hint (e.g. DeepSeek ramp-up note). */
    emptyHint?: string;
    /** Unit fallback before the first fetch resolves; fetched cells carry
     *  their own unit and take precedence. */
    unit?: UsageUnit;
  }

  let { providerId, emptyHint, unit = "tokens" as UsageUnit }: Props = $props();

  const WEEKS = 5; // 5 Monday-aligned columns cover the last 31 days

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
    const id = providerId;
    const seq = ++fetchSeq;
    loaded = false;
    cells = [];
    getHeatmap(id, 31)
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
  // may embed up to 90 days, so levels and the empty-state check normalize on
  // this window only (mirrors DetailCard's 7-day projection). One derived
  // keeps the clock and payload in lockstep — it recomputes whenever cells
  // changes (e.g. when a tab-switch fetch resolves on the long-lived instance).
  let view = $derived.by(() => {
    const today = new Date();
    const todayKey = localDateKey(today);
    const todayDow = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const start = new Date(today);
    start.setDate(today.getDate() - todayDow - (WEEKS - 1) * 7);
    const startKey = localDateKey(start);
    const windowCells = cells.filter(
      (c) => c.date >= startKey && c.date <= todayKey,
    );
    // Column = week, row = Mon..Sun (ISO). Past days missing from the payload
    // render as transparent level-0 cells; future days render null (clear).
    const cols: Day[][] = [];
    for (let wi = 0; wi < WEEKS; wi++) {
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
    }
    return { cols, windowCells };
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
  function levelColor(level: 0 | 1 | 2 | 3 | 4): string {
    switch (level) {
      case 0:
        return "transparent";
      case 1:
        return "rgba(76,194,255,0.18)";
      case 2:
        return "rgba(76,194,255,0.38)";
      case 3:
        return "rgba(76,194,255,0.62)";
      case 4:
        return "rgba(76,194,255,0.92)";
    }
  }

  function cellShadow(day: NonNullable<Day>): string {
    const parts: string[] = [];
    if (day.level === 4) parts.push("0 0 4px rgba(76,194,255,0.45)");
    if (day.isToday) parts.push("inset 0 0 0 1.5px var(--tum-amber)");
    return parts.length > 0 ? parts.join(",") : "none";
  }

  const legendLevels: Array<1 | 2 | 3 | 4> = [1, 2, 3, 4];
  let legendColors = $derived(legendLevels.map((l) => levelColor(l)));
  let maxValue = $derived(Math.max(0, ...view.windowCells.map((c) => c.value)));
  let displayUnit = $derived(view.windowCells[0]?.unit ?? unit);
</script>

<div class="heatmap">
  {#if loaded && maxValue <= 0}
    <div class="heatmap__empty">{emptyHint ?? "该来源暂无热力图数据"}</div>
  {:else}
    <div class="heatmap__cols">
      {#each view.cols as col}
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
    grid-template-columns: repeat(5, 14px);
    gap: 3px;
    justify-content: center;
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
    cursor: default;
    transition: transform 0.15s ease;
  }

  .heatmap__cell:hover {
    transform: scale(1.4);
  }

  .heatmap__cell--future {
    background: transparent;
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
    justify-content: center;
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
</style>
