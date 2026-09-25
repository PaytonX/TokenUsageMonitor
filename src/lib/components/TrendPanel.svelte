<script lang="ts">
  // 趋势看板（C7 / B5）——嵌入面板：跨 provider 聚合逐日用量，以 SVG 折线展示
  // 长区间趋势（30/90 天也不截断、刻度不重叠）。点击图表或 ⤢ 打开独立可缩放的
  // 趋势窗口（TrendWindow，堆叠柱 + 滚动），获得可放大的明细视图。
  import { openTrendWindow } from "../api";
  import { readPref, writePref } from "../prefs";
  import {
    buildTrendDays,
    fetchProviderSeries,
    formatCompact,
    RANGES,
    refreshProviderSeries,
    tickEveryFor,
    type RangeKey,
  } from "../trend-data";
  import TrendLineChart from "./TrendLineChart.svelte";

  interface Props {
    /** Active snapshots; defines provider order + colors. */
    providerIds: string[];
    colors: Record<string, string>;
    /** 总量线 / 面积颜色（系统强调色）。 */
    accent?: string;
  }

  let { providerIds, colors, accent = "#4cc2ff" }: Props = $props();

  let range: RangeKey = $state(
    RANGES.some((r) => r.key === (readPref("tum.trend.range", "30d") as RangeKey))
      ? (readPref("tum.trend.range", "30d") as RangeKey)
      : "30d",
  );
  let rangeDef = $derived(RANGES.find((r) => r.key === range)!);

  let seriesByProvider: Record<string, Record<string, number>> = $state({});
  let seq = 0;
  // 周期刷新"当天"用量（只在拉取过系列后生效），刷新后重建顶层对象触发重渲染。
  let refreshTick = $state(0);
  $effect(() => {
    const id = setInterval(() => {
      void refreshProviderSeries(providerIds, 2).then(() => refreshTick++);
    }, 60_000);
    return () => clearInterval(id);
  });

  $effect(() => {
    void refreshTick; // 依赖刷新 tick：周期刷新后重新切片重绘
    const ids = providerIds;
    const mySeq = ++seq;
    void fetchProviderSeries(ids, rangeDef.days).then((result) => {
      if (mySeq === seq) seriesByProvider = result;
    });
  });

  let days = $derived(
    buildTrendDays(seriesByProvider, providerIds, rangeDef.days),
  );
  let maxTotal = $derived(
    Math.max(1, ...days.map((d) => d.total)),
  );
  let rangeTotal = $derived(days.reduce((s, d) => s + d.total, 0));
  let streak = $derived.by(() => {
    let num = 0;
    for (let i = days.length - 1; i >= 0; i--) {
      if (days[i].total > 0) num++;
      else break;
    }
    return num;
  });
  let tickEvery = $derived(tickEveryFor(rangeDef.days));
</script>

<div
    class="trend"
    data-tauri-drag-region={false}
    role="button"
    tabindex="0"
    aria-label="打开趋势窗口"
    onclick={() => void openTrendWindow()}
    onkeydown={(e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        void openTrendWindow();
      }
    }}
  >
  <div class="trend__head">
    <span class="trend__title">趋势看板</span>
    <div class="trend__head-right" role="group" aria-label="时间区间">
      {#each RANGES as r (r.key)}
        <button
          type="button"
          class="trend__range"
          class:is-active={range === r.key}
          onclick={(e) => {
            e.stopPropagation();
            range = r.key;
            writePref("tum.trend.range", r.key);
          }}
        >{r.label}</button>
      {/each}
      <button
        type="button"
        class="trend__zoom"
        title="放大为独立窗口"
        onclick={(e) => {
          e.stopPropagation();
          void openTrendWindow();
        }}
      >⤢</button>
    </div>
  </div>

  <div class="trend__stats">
    <span class="trend__stat"><b>{formatCompact(rangeTotal)}</b><span>本区间累计</span></span>
    <span class="trend__stat"><b>{streak} 天</b><span>连续活跃</span></span>
  </div>

  <div class="trend__chart">
    <TrendLineChart {days} {colors} maxY={maxTotal} {tickEvery} {accent} />
  </div>

  <div class="trend__legend">
    {#each providerIds as id (id)}
      <span class="trend__key">
        <i class="trend__swatch" style={`background:${colors[id] ?? "#8a8f98"}`}></i>
        <span>{id}</span>
      </span>
    {/each}
  </div>
</div>

<style>
  .trend {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    background: var(--tum-surface);
    padding: 10px 12px;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    cursor: zoom-in;
    position: relative;
  }

  .trend__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    flex: none;
  }

  .trend__head-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .trend__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .trend__range {
    border: none;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    padding: 2px 8px;
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
  }

  .trend__range:hover {
    color: var(--tum-text-primary);
  }

  .trend__range.is-active {
    background: rgba(76, 194, 255, 0.18);
    color: var(--tum-text-primary);
  }

  .trend__zoom {
    flex: none;
    width: 18px;
    height: 18px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    background: var(--tum-surface);
    color: var(--tum-text-muted);
    font-size: 11px;
    line-height: 1;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .trend__zoom:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .trend__stats {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    flex: none;
  }

  .trend__stat {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
  }

  .trend__stat b {
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
  }

  .trend__stat span {
    color: var(--tum-text-muted);
    font-size: 10px;
  }

  .trend__chart {
    flex: 1;
    min-height: 0;
    display: flex;
    margin: 0 -2px;
  }

  .trend__legend {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    flex: none;
  }

  .trend__key {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--tum-font-mono);
    font-size: 9px;
    color: var(--tum-text-secondary);
  }

  .trend__swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    display: inline-block;
  }
</style>