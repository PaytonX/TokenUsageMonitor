<script lang="ts">
  // 趋势看板（C7 / B5）——嵌入面板：跨 provider 聚合逐日用量，以 SVG 折线展示
  // 长区间趋势（30/90 天也不截断、刻度不重叠）。点击图表或 ⤢ 打开独立可缩放的
  // 趋势窗口（TrendWindow，堆叠柱 + 滚动），获得可放大的明细视图。
  import { getHubDevices, onTabsChanged, openTrendWindow } from "../api";
  import { readAggMode, readPref, writePref } from "../prefs";
  import { buildDeviceSeries, stackColors } from "../device-agg";
  import {
    buildTrendDays,
    dimOthers,
    fetchProviderCrossToolSeries,
    formatCompact,
    RANGES,
    tickEveryFor,
    type RangeKey,
  } from "../trend-data";
  import TrendLineChart from "./TrendLineChart.svelte";
  import CalendarSection from "./CalendarSection.svelte";
  import {
    PanelHeader,
    RangePills,
    ZoomButton,
    Stat,
    ColorSwatch,
  } from "./atoms";

  interface Props {
    /** 总量线 / 面积颜色（系统强调色）。 */
    accent?: string;
    /** 日历高亮焦点。父级（App）可绑定，从而让总览页卡片点击预置趋势页高亮。 */
    highlightKey?: string | null;
  }

  let {
    accent = "#4cc2ff",
    highlightKey = $bindable(null),
  }: Props = $props();

  let range: RangeKey = $state(
    RANGES.some((r) => r.key === (readPref("tum.trend.range", "30d") as RangeKey))
      ? (readPref("tum.trend.range", "30d") as RangeKey)
      : "30d",
  );
  let rangeDef = $derived(RANGES.find((r) => r.key === range)!);

  // 全端汇总模式：序列来自各设备上报（按设备堆叠）；本机模式：账本中各
  // 工具的 token 日序列（按工具堆叠）。两者同为 tokens 口径。
  let aggMode = $state(readAggMode());
  let srcIds = $state<string[]>([]);
  let srcSeries = $state<Record<string, Record<string, number>>>({});
  let srcNames = $state<Record<string, string>>({});

  let seq = 0;
  let refreshTick = $state(0);

  // 拉取：aggMode → 设备序列；本机 → 账本工具序列。依赖 aggMode / range /
  // refreshTick，变化即重拉；两个分支只写 src* 状态、不读它们，避免 effect
  // 自依赖循环。
  $effect(() => {
    void rangeDef.days;
    void refreshTick;
    const mySeq = ++seq;
    if (aggMode) {
      void getHubDevices().then((r) => {
        if (mySeq !== seq) return;
        const built = buildDeviceSeries(r.devices);
        srcIds = built.ids;
        srcSeries = built.series;
        srcNames = built.names;
      });
    } else {
      // 本机模式：按 Provider 跨工具聚合（MiniMax 线 = 全部工具对其模型的
      // 使用合计，含 MiniMax Code 未标记行的兜底归因）。
      void fetchProviderCrossToolSeries(rangeDef.days).then((built) => {
        if (mySeq !== seq) return;
        srcIds = built.ids;
        srcSeries = built.series;
        srcNames = built.names;
        providerColorsFromLedger = built.colors;
      });
    }
  });

  // 周期刷新（账本/设备序列都是整段重取，成本低）。
  $effect(() => {
    const id = setInterval(() => refreshTick++, 60_000);
    return () => clearInterval(id);
  });

  $effect(() => {
    void onTabsChanged(() => {
      const next = readAggMode();
      if (next !== aggMode) {
        aggMode = next;
        refreshTick++;
      }
    });
  });

  let activeIds = $derived(srcIds);
  let activeSeries = $derived(srcSeries);
  // 堆叠顺序配色：相邻层不同色相（哈希配色会让相近色叠在一起）。
  // 本机模式：provider 品牌色；全端模式：按堆叠顺序的高对比配色。
  let providerColorsFromLedger = $state<Record<string, string>>({});
  let activeColors = $derived.by(() => {
    if (aggMode) return stackColors(srcIds);
    return Object.keys(providerColorsFromLedger).length > 0
      ? providerColorsFromLedger
      : stackColors(srcIds);
  });

  let days = $derived(buildTrendDays(activeSeries, activeIds, rangeDef.days));
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
  // 图例文案：聚合态显示主机名，本机态显示 provider id。
  function labelOf(id: string): string {
    return srcNames[id] ?? id;
  }
</script>

<div class="trend" data-tauri-drag-region={false}>
  <PanelHeader
    title={aggMode ? "全端趋势看板" : "趋势看板 · 本机工具"}
  >
    <RangePills
      options={RANGES}
      value={range}
      onChange={(k) => {
        range = k;
        writePref("tum.trend.range", k);
      }}
    />
    {#if !aggMode}
      <ZoomButton onclick={() => void openTrendWindow()} />
    {/if}
  </PanelHeader>

  <div class="trend__stats">
    <Stat value={formatCompact(rangeTotal)} label={aggMode ? "全端累计" : "本机累计"} />
    <Stat value={streak} label="连续活跃" suffix="天" />
  </div>

  <div class="trend__chart">
    <TrendLineChart
      {days}
      colors={dimOthers(activeColors, highlightKey)}
      maxY={maxTotal}
      {tickEvery}
      {accent}
    />
  </div>

  <div class="trend__legend">
    {#each activeIds as id (id)}
      <span class="trend__key">
        <ColorSwatch color={activeColors[id] ?? "rgb(138,143,152)"} size={8} />
        <span>{labelOf(id)}</span>
      </span>
    {/each}
  </div>

  <!-- 日历区（2026-10 从总览页迁入）。药丸条承载 provider 高亮：
       日历离开总览后卡片↔日历无法同屏，联动载体换成这条与区间按钮同构的条。 -->
  <CalendarSection
    series={aggMode ? [] : activeIds}
    colors={activeColors}
    bind:highlightKey
  />
</div>

<style>
  /* 打开独立窗口只走右上角的 ⤢ 按钮，不再整块可点。
     整块 onclick 会与日历区的下拉/药丸冲突（点下拉冒泡到根 div 就弹窗），
     而且 cursor: zoom-in 会让整块看起来都可点，实际只有一处能点。 */
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
    position: relative;
  }

  .trend__stats {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    flex: none;
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
</style>