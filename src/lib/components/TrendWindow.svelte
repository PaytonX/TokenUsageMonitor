<script lang="ts">
  // 独立趋势窗口（C7）：可拖拽缩放、最大化。展示逐日堆叠柱状图，柱宽固定
  // （≥ 窗口放不下时横向滚动，放大窗口即可一次看更多 / 更粗），x 轴刻度按
  // 区间稀疏，保证任何区间都清晰完整。数据源与趋势页同源：统一账本的
  // 分工具 token 日序列。
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    buildTrendDays,
    dimOthers,
    fetchProviderCrossToolSeries,
    formatCompact,
    RANGES,
    type RangeKey,
  } from "../trend-data";
  import { hexToRgb } from "../types";
  import CalendarSection from "./CalendarSection.svelte";
  import { RangePills, ColorSwatch } from "./atoms";

  let status: "loading" | "ready" | "empty" = $state("loading");
  let seriesIds: string[] = $state([]);
  // 序列按 Provider 跨工具聚合（与嵌入版 TrendPanel 同口径）。原先这里用的是
  // fetchToolSeries（按工具），是 723e360 重构时的遗漏：两个趋势视图对同一份
  // 数据给出两种切分，且日历的药丸条只能按 provider 高亮，键空间对不上。
  let seriesNames: Record<string, string> = $state({});
  let seriesColors: Record<string, string> = $state({});
  let highlightKey: string | null = $state(null);

  let range: RangeKey = $state("30d");
  let rangeDef = $derived(RANGES.find((r) => r.key === range)!);

  let seriesById: Record<string, Record<string, number>> = $state({});
  let days = $derived(
    buildTrendDays(seriesById, seriesIds, rangeDef.days),
  );
  let maxTotal = $derived(Math.max(1, ...days.map((d) => d.total)));
  let rangeTotal = $derived(days.reduce((s, d) => s + d.total, 0));
  let streak = $derived.by(() => {
    let num = 0;
    for (let i = days.length - 1; i >= 0; i--) {
      if (days[i].total > 0) num++;
      else break;
    }
    return num;
  });

  onMount(async () => {
    try {
      const built = await fetchProviderCrossToolSeries(rangeDef.days);
      if (built.ids.length === 0) {
        status = "empty";
        return;
      }
      seriesIds = built.ids;
      seriesById = built.series;
      seriesNames = built.names;
      seriesColors = built.colors;
      status = "ready";
    } catch {
      status = "empty";
    }
  });

  let refreshTrendTick = $state(0);

  $effect(() => {
    void refreshTrendTick; // 依赖刷新 tick：周期刷新后重建顶层对象触发重渲染
    if (seriesIds.length === 0 || status !== "ready") return;
    void fetchProviderCrossToolSeries(rangeDef.days).then((r) => {
      seriesById = r.series;
      seriesNames = r.names;
      seriesColors = r.colors;
    });
  });

  // 周期刷新（每 60s），仅当窗口有线图数据时启用。
  $effect(() => {
    if (seriesIds.length === 0 || status !== "ready") return;
    const id = setInterval(() => refreshTrendTick++, 60_000);
    return () => clearInterval(id);
  });

  function colorOf(id: string): string {
    return dimOthers(seriesColors, highlightKey)[id] ?? "rgb(138,143,152)";
  }
  function nameOf(id: string): string {
    return seriesNames[id] ?? id;
  }
  function segStyle(id: string): string {
    const c = colorOf(id);
    // 高亮时 colorOf 返回的已是 rgba(...)（压暗过的），不能再包一层——否则
    // hexToRgb 解析失败会全部回落到同一个系统蓝，压暗效果被抹掉。
    if (c.startsWith("rgba(")) return `background: ${c}`;
    const rgb = hexToRgb(c) ?? "76,194,255";
    return `background: rgba(${rgb}, 0.85)`;
  }

  // 稀疏刻度：按可视宽度自适应步长，保证相邻标签间距 ≥ LABEL_GAP_PX，
  // 从构造上杜绝 30/90 天拥挤/交叠。
  const LABEL_GAP_PX = 44;
  let bodyW = $state(0);
  let tickEvery = $derived.by(() => {
    let base = rangeDef.days <= 7 ? 1 : rangeDef.days <= 30 ? 3 : 7;
    const usable = Math.max(1, bodyW - 8);
    const n = Math.max(1, days.length);
    return Math.max(base, Math.ceil(((n - 1) * LABEL_GAP_PX) / usable));
  });
  function tickVisible(i: number): boolean {
    if (i === 0 || i === days.length - 1) return true;
    if (i % tickEvery !== 0) return false;
    // 中间刻度若离末尾强制显示的标签不足一个步长，则跳过，避免尾部交叠。
    return days.length - 1 - i >= tickEvery;
  }

  function close() {
    void getCurrentWindow().close();
  }
  function minimize() {
    void getCurrentWindow().minimize();
  }
  function toggleMaximize() {
    void getCurrentWindow().toggleMaximize();
  }
  function onDrag(e: PointerEvent) {
    if ((e.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }
</script>

<div class="tw">
  <div class="tw__bar" onpointerdown={onDrag} role="toolbar" aria-label="窗口控制" tabindex="-1">
    <span class="tw__bar-title">用量趋势</span>
    <div class="tw__bar-controls">
      <button type="button" class="tw__bar-btn" aria-label="最小化" onclick={minimize} onpointerdown={(e) => e.stopPropagation()}>—</button>
      <button type="button" class="tw__bar-btn" aria-label="最大化/还原" onclick={toggleMaximize} onpointerdown={(e) => e.stopPropagation()}>▢</button>
      <button type="button" class="tw__bar-btn tw__bar-btn--close" aria-label="关闭" onclick={close} onpointerdown={(e) => e.stopPropagation()}>✕</button>
    </div>
  </div>
  <header class="tw__head">
    <span class="tw__title">用量趋势</span>
    <RangePills
      options={RANGES}
      value={range}
      onChange={(k) => (range = k)}
      label="时间区间"
    />
    <span class="tw__stats">
      累计 <b>{formatCompact(rangeTotal)}</b> · 连续 <b>{streak}</b> 天
    </span>
  </header>

  {#if status === "empty"}
    <div class="tw__empty">暂无用量数据，请先在主面板配置 Provider</div>
  {:else if status === "loading"}
    <div class="tw__empty">正在加载用量数据…</div>
  {:else}
    <div class="tw__body" bind:clientWidth={bodyW}>
      <div class="tw__chart">
        {#each days as d, i (d.date)}
          <div
            class="tw__day"
            title={`${d.date} · ${d.total.toLocaleString()}`}
          >
            <div class="tw__col">
              {#each d.parts as p (p.id)}
                <div class="tw__seg" style={`height:${((p.value / maxTotal) * 100).toFixed(1)}%;${segStyle(p.id)}`}></div>
              {:else}
                <div class="tw__seg tw__seg--zero"></div>
              {/each}
            </div>
            <span class="tw__tick" class:tw__tick--hidden={!tickVisible(i)}>{tickVisible(i) ? d.label : ""}</span>
          </div>
        {/each}
      </div>
    </div>

    <div class="tw__legend">
      {#each seriesIds as id (id)}
        <span class="tw__key">
          <ColorSwatch color={colorOf(id)} size={9} />
          <span>{nameOf(id)}</span>
        </span>
      {/each}
      <span class="tw__legend-hint">放大本窗口可获得更粗的柱与更多细节</span>
    </div>

    <!-- 不限宽：窗口已从 760×480 抬到 780×720，日历铺满 740px 时约占 260px，
         剩下的都给趋势图（.tw__body 是 flex:1）。早先限宽 440px 是为了压住
         480 矮窗的日历，但那样右侧会空出 340px，比原问题更难看。 -->
    <CalendarSection series={seriesIds} colors={seriesColors} bind:highlightKey />
  {/if}
</div>

<style>
  .tw {
    height: 100vh;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 16px;
    box-sizing: border-box;
    /* 与主界面一致的分层玻璃：强调色径向光晕 + 半透明暗底 + 背景模糊。 */
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
      rgba(20, 22, 26, 0.82);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-lg);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    overflow: hidden;
  }

  .tw__head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
    flex-wrap: wrap;
  }

  /* 自绘主题标题栏（无边框窗口）：拖动区 + 最小化/最大化/关闭，暗色玻璃与令牌 */
  .tw__bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: none;
    height: 34px;
    margin: -14px -16px 0;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--tum-border);
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
  }

  .tw__bar-title {
    font-family: var(--tum-font-mono);
    font-size: 11px;
    color: var(--tum-text-muted);
    letter-spacing: 0.6px;
    text-transform: uppercase;
  }

  .tw__bar-controls {
    display: flex;
    gap: 2px;
  }

  .tw__bar-btn {
    width: 30px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 11px;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .tw__bar-btn:hover {
    background: var(--tum-surface-hover);
    color: var(--tum-text-primary);
  }

  .tw__bar-btn--close:hover {
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
  }

  .tw__title {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
  }

  .tw__stats {
    flex: 1;
    font-family: var(--tum-font-mono);
    font-size: 11px;
    color: var(--tum-text-muted);
  }

  .tw__stats b {
    color: var(--tum-text-primary);
    font-variant-numeric: tabular-nums;
  }

  .tw__body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  /* 弹性柱宽：所有柱一次性铺满整行，任何区间都不截断（柱随窗口宽度伸缩，
     放大窗口柱子变粗）。刻度保持固定高度槽位对齐柱底。 */
  .tw__chart {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 100%;
    width: 100%;
    padding: 0 4px 6px 4px;
  }

  .tw__day {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    height: 100%;
  }

  .tw__col {
    width: 100%;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 1px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.03);
    overflow: hidden;
  }

  .tw__seg {
    width: 100%;
    min-height: 2px;
    border-radius: 2px;
  }

  .tw__seg--zero {
    background: rgba(255, 255, 255, 0.05);
    height: 3px;
  }

  .tw__tick {
    flex: none;
    height: 14px;
    line-height: 14px;
    font-family: var(--tum-font-mono);
    font-size: 10px;
    color: var(--tum-text-muted);
    white-space: nowrap;
  }

  /* 留出固定高度的刻度槽，使有/无标签的柱子底边对齐同一基线。 */
  .tw__tick--hidden {
    visibility: hidden;
  }

  .tw__legend {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 16px;
    flex: none;
  }

  .tw__key {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-family: var(--tum-font-mono);
    font-size: 11px;
    color: var(--tum-text-secondary);
  }

  .tw__legend-hint {
    margin-left: auto;
    font-family: var(--tum-font-mono);
    font-size: 10px;
    color: var(--tum-text-muted);
  }

  .tw__empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    text-align: center;
  }
</style>