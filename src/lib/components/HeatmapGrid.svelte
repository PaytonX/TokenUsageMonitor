<script lang="ts">
  import type { HeatmapCell, UsageUnit } from "../types";
  import { formatUsage } from "../types";

  interface Props {
    cells: HeatmapCell[];
    unit?: UsageUnit;
    weeks?: number;
  }

  let { cells, unit = "tokens" as UsageUnit, weeks = 14 }: Props = $props();

  // Build a grid: column = week, row = day of week (Mon..Sun, ISO).
  // Each cell: { date, value, level }.
  type Day = { date: string; value: number; level: 0 | 1 | 2 | 3 | 4 } | null;

  let byDate = $derived.by(() => {
    const map = new Map<string, HeatmapCell>();
    for (const c of cells) map.set(c.date, c);
    return map;
  });

  // Start = (today - weeks*7 days), aligned to Monday.
  let grid = $derived.by(() => {
    const today = new Date();
    const todayDay = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const start = new Date(today);
    start.setDate(today.getDate() - todayDay - (weeks - 1) * 7);
    const cols: Day[][] = [];
    for (let w = 0; w < weeks; w++) {
      const col: Day[] = [];
      for (let d = 0; d < 7; d++) {
        const d2 = new Date(start);
        d2.setDate(start.getDate() + w * 7 + d);
        if (d2 > today) {
          col.push(null);
          continue;
        }
        const dateStr = d2.toISOString().slice(0, 10);
        const cell = byDate.get(dateStr);
        const value = cell?.value ?? 0;
        col.push({ date: dateStr, value, level: levelFor(value, cells) });
      }
      cols.push(col);
    }
    return cols;
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

  function levelColor(level: 0 | 1 | 2 | 3 | 4): string {
    // Amber scale - matches the primary accent for a cohesive instrument feel.
    switch (level) {
      case 0:
        return "rgba(148,163,184,0.10)";
      case 1:
        return "rgba(251,191,36,0.20)";
      case 2:
        return "rgba(251,191,36,0.42)";
      case 3:
        return "rgba(251,191,36,0.68)";
      case 4:
        return "rgba(251,191,36,0.95)";
    }
  }

  function levelGlow(level: 0 | 1 | 2 | 3 | 4): string {
    // Subtle glow only on the brightest level, like a phosphor pixel.
    return level === 4 ? "0 0 4px rgba(251,191,36,0.5)" : "none";
  }

  // Pre-compute legend colors so the template doesn't need TS casts inline.
  const legendLevels: Array<0 | 1 | 2 | 3 | 4> = [0, 1, 2, 3, 4];
  let legendColors = $derived(legendLevels.map((l) => levelColor(l)));
</script>

<div class="heatmap">
  <div class="heatmap__cols">
    {#each grid as col}
      <div class="heatmap__col">
        {#each col as day}
          <div
            class="heatmap__cell"
            style={`background:${day ? levelColor(day.level) : "transparent"}; box-shadow:${day ? levelGlow(day.level) : "none"}`}
            title={day ? `${day.date} · ${formatUsage(day.value, unit)}` : ""}
          ></div>
        {/each}
      </div>
    {/each}
  </div>
  <div class="heatmap__legend">
    <span class="heatmap__legend-text">少</span>
    {#each legendColors as color}
      <span class="heatmap__cell" style={`background:${color}`}></span>
    {/each}
    <span class="heatmap__legend-text">多</span>
  </div>
</div>

<style>
  .heatmap {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .heatmap__cols {
    display: flex;
    gap: 3px;
  }

  .heatmap__col {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .heatmap__cell {
    width: 10px;
    height: 10px;
    border-radius: var(--tum-radius-xs);
    cursor: default;
    transition: transform 0.15s ease;
  }

  .heatmap__cell:hover {
    transform: scale(1.4);
  }

  .heatmap__legend {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    justify-content: flex-end;
  }

  .heatmap__legend-text {
    font-size: 9px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
  }
</style>
