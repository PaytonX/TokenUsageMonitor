<script lang="ts">
  // 嵌入趋势线图（C7）：以 SVG 折线绘制每日总量 + 各 provider 用量，横轴按
  // 天数均匀分布，x 轴刻度按区间稀疏（tickEvery）显示，长区间不截断、不重叠。
  // 悬停显示十字辅助线 + 当日总量与各 provider 明细。
  import type { TrendDay } from "../trend-data";
  import ColorSwatch from "./atoms/ColorSwatch.svelte";
  import { t } from "../i18n/store";

  interface Props {
    days: TrendDay[];
    /** 序列 id -> 颜色。 */
    colors: Record<string, string>;
    /** 序列 id -> 展示名（tooltip/图例用；缺省显示 id）。 */
    names?: Record<string, string>;
    /** y 轴最大值（各 provider 总值）。 */
    maxY: number;
    /** x 轴刻度稀疏倍数（minimum；会根据像素宽度自适应加稀）。 */
    tickEvery: number;
    /** 总量面积填充色。 */
    accent: string;
    /** true = 堆积面积图（各 provider 依次叠加）；false = 多折线 + 总量面积。 */
    stacked?: boolean;
  }

  let { days, colors, names = {}, maxY, tickEvery, accent, stacked = true }: Props = $props();

  // 监听容器尺寸，SVG 随面板缩放重算几何。
  let width = $state(0);
  let height = $state(0);
  let hoverIndex: number | null = $state(null);

  const PAD_L = 8;
  const PAD_R = 8;
  const PAD_T = 10;
  const PAD_B = 18;

  let plotW = $derived(Math.max(0, width - PAD_L - PAD_R));
  let plotH = $derived(Math.max(0, height - PAD_T - PAD_B));
  let maxCap = $derived(maxY * 1.12 + 1e-9);

  let n = $derived(days.length);
  function x(i: number): number {
    if (n <= 1) return PAD_L + plotW / 2;
    return PAD_L + (i / (n - 1)) * plotW;
  }
  function y(v: number): number {
    return PAD_T + (1 - clamp(v, 0, maxCap) / maxCap) * plotH;
  }
  function clamp(v: number, lo: number, hi: number): number {
    return Math.max(lo, Math.min(hi, v));
  }

  // 每个 provider 的逐日值序列（缺日补 0），用于折线。
  let providerSeries = $derived.by(() => {
    const ids: string[] = [];
    for (const d of days) for (const p of d.parts) if (!ids.includes(p.id)) ids.push(p.id);
    return ids.map((id) => ({
      id,
      values: days.map((d) => d.parts.find((p) => p.id === id)?.value ?? 0),
      color: colors[id] ?? "rgb(138,143,152)",
    }));
  });

  let linePoints = (values: number[]): string =>
    values.map((v, i) => `${x(i)},${y(v)}`).join(" ");

  // 堆积面积：按 provider 顺序累计，每个 provider 画一块介于其上下界之间的
  // 填充多边形（底边=之前 provider 累积值，顶边=含本 provider 的累积值）。
  let stackedAreas = $derived.by(() => {
    if (n === 0 || plotW <= 0) return [] as { id: string; color: string; d: string; label: string }[];
    const acc = new Array<number>(n).fill(0);
    return providerSeries.map((s) => {
      const top: string[] = [];
      const bottom: string[] = [];
      const values = s.values;
      for (let i = 0; i < n; i++) {
        const lo = acc[i];
        const hi = lo + values[i];
        top.push(`${x(i)},${y(hi)}`);
        bottom.push(`${x(i)},${y(lo)}`);
        acc[i] = hi;
      }
      bottom.reverse();
      return { id: s.id, color: s.color, d: `M ${top.join(" ")} L ${bottom.join(" ")} Z`, label: s.id };
    });
  });

  // 总量面积 + 上沿折线。
  let areaPath = $derived.by(() => {
    if (n === 0 || plotW <= 0) return "";
    const top = days.map((d, i) => `${x(i)},${y(d.total)}`).join(" ");
    const bottom = `${x(n - 1)},${PAD_T + plotH} ${x(0)},${PAD_T + plotH}`;
    return `M ${top} L ${bottom} Z`;
  });
  let totalLine = $derived(days.map((d, i) => `${x(i)},${y(d.total)}`).join(" "));

  // x 轴刻度：自适应步长。按可选像素宽度计算相邻标签所需的最小索引间隔，
  // 保证标签间距 ≥ 标签自身宽度，避免 30/90 天等密集区间两端交叠。
  const LABEL_GAP_PX = 34;
  let tickStep = $derived(
    Math.max(
      tickEvery,
      n <= 1 || plotW <= 0
        ? 1
        : Math.ceil(((n - 1) * LABEL_GAP_PX) / plotW),
    ),
  );
  let ticks = $derived(
    days.map((d, i) => ({ i, label: d.label })).filter((_, i) => tickVisible(i)),
  );
  function tickVisible(i: number): boolean {
    if (i === 0 || i === n - 1) return true;
    if (i % tickStep !== 0) return false;
    // 中间刻度若离末尾强制显示的标签不足一个步长，则跳过，避免尾部交叠。
    return n - 1 - i >= tickStep;
  }
  let tickAnchor = (i: number): string =>
    i === 0 ? "start" : i === n - 1 ? "end" : "middle";

  // 网格参考线（含总数值提示）。
  let grid = $derived([0.25, 0.5, 0.75, 1].map((f) => PAD_T + plotH * f));

  // 悬停命中：由 SVG 内相对坐标换算索引。
  function onMove(e: PointerEvent) {
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const px = e.clientX - rect.left;
    if (n <= 1) {
      hoverIndex = 0;
      return;
    }
    // 容量边到 plot 左侧/右侧时按最接近的索引。
    const ratio = (px - PAD_L) / plotW;
    const idx = Math.round(ratio * (n - 1));
    hoverIndex = clamp(idx, 0, n - 1);
  }

  let hover = $derived(
    hoverIndex !== null ? days[hoverIndex] ?? null : null,
  );
  let hoverX = $derived(hoverIndex !== null ? x(hoverIndex) : 0);
</script>

<div
    class="tl"
    bind:clientWidth={width}
    bind:clientHeight={height}
    role="img"
    aria-label={$t("chart.ariaLabel")}
    onpointermove={onMove}
    onpointerleave={() => (hoverIndex = null)}
  >
  {#if width > 0 && height > 0 && n > 0}
    <svg class="tl__svg" width={width} height={height} aria-label={$t("chart.ariaLabel")}>
      {#each grid as gy (gy)}
        <line x1={PAD_L} x2={width - PAD_R} y1={gy} y2={gy} class="tl__grid"></line>
      {/each}

      {#if stacked}
        {#each stackedAreas as a (a.id)}
          <path d={a.d} class="tl__stack" style={`fill:${a.color}`}></path>
        {/each}
        <polyline points={totalLine} class="tl__total" style={`stroke:${accent}`} fill="none"></polyline>
      {:else}
        {#if areaPath}
          <path d={areaPath} class="tl__area" style={`fill:${accent}`}></path>
        {/if}
        <polyline points={totalLine} class="tl__total" style={`stroke:${accent}`} fill="none"></polyline>
        {#each providerSeries as s (s.id)}
          <polyline points={linePoints(s.values)} class="tl__line" style={`stroke:${s.color}`} fill="none"></polyline>
        {/each}
      {/if}

      {#each ticks as t (t.i)}
        <text x={x(t.i)} y={height - 5} class="tl__tick" text-anchor={tickAnchor(t.i)}>{t.label}</text>
      {/each}

      {#if hover}
        <line x1={hoverX} x2={hoverX} y1={PAD_T} y2={PAD_T + plotH} class="tl__guide"></line>
        <circle cx={hoverX} cy={y(hover.total)} r="2.5" class="tl__dot" style={`stroke:${accent}`}></circle>
      {/if}

      <rect x={PAD_L} y={PAD_T} width={plotW} height={plotH} fill="transparent"></rect>
    </svg>

    {#if hover}
      <div
        class="tl__tip"
        style={hoverX > width / 2
          ? `right:${Math.max(4, width - hoverX + 10)}px`
          : `left:${Math.max(4, hoverX + 10)}px`}
      >
        <div class="tl__tip-date">{hover.date}</div>
        <div class="tl__tip-row"><span class="tl__tip-k">{$t("chart.total")}</span><b>{hover.total.toLocaleString()}</b></div>
        {#each hover.parts as p (p.id)}
          <div class="tl__tip-row">
            <span class="tl__tip-k">
              <ColorSwatch color={colors[p.id] ?? "rgb(138,143,152)"} size={8} />
              {$t(names[p.id] ?? p.id)}
            </span>
            <b>{p.value.toLocaleString()}</b>
          </div>
        {/each}
      </div>
    {/if}
  {:else if width > 0}
    <div class="tl__empty">{$t("common.noData")}</div>
  {/if}
</div>

<style>
  .tl {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .tl__svg {
    display: block;
  }

  .tl__grid {
    stroke: rgba(255, 255, 255, 0.07);
    stroke-width: 1;
  }

  .tl__area {
    opacity: 0.16;
  }

  .tl__stack {
    opacity: 0.5;
  }

  .tl__total {
    stroke-width: 1.6;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .tl__line {
    stroke-width: 1;
    opacity: 0.9;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .tl__tick {
    font-family: var(--tum-font-mono);
    font-size: 8px;
    fill: var(--tum-text-muted);
  }

  .tl__guide {
    stroke: rgba(255, 255, 255, 0.22);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }

  .tl__dot {
    fill: var(--tum-surface);
    stroke-width: 1.5;
  }

  .tl__tip {
    position: absolute;
    top: 2px;
    pointer-events: none;
    min-width: 110px;
    padding: 5px 7px;
    border-radius: var(--tum-radius-sm);
    border: 1px solid var(--tum-border-strong);
    background: rgba(28, 30, 34, 0.95);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4);
    font-family: var(--tum-font-mono);
    font-size: 9px;
    line-height: 1.5;
    color: var(--tum-text-secondary);
    z-index: 5;
  }

  .tl__tip-date {
    color: var(--tum-text-primary);
    margin-bottom: 2px;
    font-weight: 600;
  }

  .tl__tip-row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }

  .tl__tip-k {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--tum-text-muted);
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tl__empty {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
  }
</style>