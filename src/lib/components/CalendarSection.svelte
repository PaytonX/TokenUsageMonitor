<script lang="ts">
  // 趋势页的日历区：provider 高亮药丸条 + 月历热力图。
  //
  // 2026-10 从总览页迁来。理由是维度对齐：总览回答"账户额度还剩多少"
  // （provider 口径），趋势回答"用量烧了多少、哪天烧的"（时间序列口径），
  // 原来把工具口径的日历挂在额度卡片下面是错配。
  //
  // 迁来后卡片↔日历在 400px 窗口里无法同屏，联动载体顺势换成这条药丸条——
  // 它与趋势页已有的"7天/30天/90天"区间按钮同构，且不占垂直空间。
  // 配色/拆解/统计全部复用 calendar-linkage.ts 的纯函数，逻辑未变。
  //
  // 2026-10 追加：删掉「口径下拉」（全部工具 / 火山账户日账…）。实测两套口径
  // 几乎零重叠（服务端 20 天 vs 本机 GLM 4 天，交集仅 2 天且服务端量微不足道），
  // 并排切换只会让人以为能对齐，实际上是两个问题的答案。现恒为合并口径，
  // 逐 provider「有服务端日账就以官方值为准，否则用本机归因」（服务端**替换**
  // 而非相加，故不会双计）。代价：OpenAI/xAI 的 USD、Kimi 的 CNY 行单位非
  // token，不进日历总量与色阶（见 buildUnifiedBreakdown）。
  import HeatmapGrid from "./HeatmapGrid.svelte";
  import { providerColor, providerLabel } from "../model-provider";
  import { hexToRgbTriplet } from "../calendar-linkage";

  interface Props {
    /** 可高亮的 provider key 序列（来自趋势序列，有数据才能高亮）。 */
    series: string[];
    /** provider key → 品牌色（与堆叠层同色，保证两处识别一致）。 */
    colors: Record<string, string>;
    /** 当前高亮焦点；null = 不高亮。父组件可双向绑定，实现跨页联动。 */
    highlightKey?: string | null;
    /** 日历区最大宽度。独立窗口很宽时不限会摊出 26px 的大格子、吃掉纵向预算，
     *  反而把上方的趋势图挤扁；限宽后密度与总览页一致。 */
    maxWidth?: string;
  }

  let {
    series,
    colors,
    highlightKey = $bindable(null),
    maxWidth,
  }: Props = $props();

  // Provider 过多时改用下拉。400px 面板一行约放得下 5 个药丸，超出就换行，
  // 换行会吃掉日历的垂直预算（趋势页要在 680px 里同时装堆叠柱 + 日历）。
  // 阈值取 6：与 PillsOrSelect 的 >3 不同，这里要保住"一眼看到全部 provider
  // 构成"的价值，只在真的要溢出时才降级为下拉。
  const PILL_LIMIT = 6;
  let useSelect = $derived(series.length > PILL_LIMIT);

  function pick(key: string) {
    highlightKey = highlightKey === key ? null : key;
  }
  function hexTriplet(hex: string): [number, number, number] {
    return hexToRgbTriplet(colors[hex] ?? providerColor(hex));
  }
  function stroke(key: string): string {
    const [r, g, b] = hexTriplet(key);
    return `rgba(${r},${g},${b},0.45)`;
  }
  function fill(key: string): string {
    const [r, g, b] = hexTriplet(key);
    return `rgba(${r},${g},${b},0.12)`;
  }
</script>

<section
  class="cal-sec"
  style={maxWidth ? `max-width:${maxWidth}` : ""}
  onclick={(e) => e.stopPropagation()}
  onpointerdown={(e) => e.stopPropagation()}
  role="presentation"
>
  <div class="cal-sec__head">
    <span class="cal-sec__title">日历热力图</span>
    <span class="cal-sec__range">近 6 个月</span>
  </div>

  <!-- 高亮药丸条：日历恒为合并口径，高亮在**任何**口径下都成立
       （服务端日账也是画在同一张日历里的），故不再有第二行"口径"药丸。
       并排两行药丸在真机验收时就被问到"这两个下拉/药丸谁是谁"。 -->
  {#if series.length > 0}
    {#if useSelect}
      <div class="cal-sec__pills cal-sec__pills--select">
        <span class="cal-sec__pill-label">高亮</span>
        <select
          class="cal-sec__sel cal-sec__sel--focus"
          value={highlightKey ?? ""}
          aria-label="高亮 Provider"
          onchange={(e) => {
            const v = e.currentTarget.value;
            highlightKey = v === "" ? null : v;
          }}
        >
          <option value="">全部</option>
          {#each series as key (key)}
            <option value={key}>{providerLabel(key)}</option>
          {/each}
        </select>
      </div>
    {:else}
      <div class="cal-sec__pills" role="group" aria-label="高亮 Provider">
        <button
          type="button"
          class="cal-sec__pill"
          class:is-on={highlightKey === null}
          onclick={(e) => {
            e.stopPropagation();
            highlightKey = null;
          }}
        >全部</button>
        {#each series as key (key)}
          <button
            type="button"
            class="cal-sec__pill"
            class:is-on={highlightKey === key}
            style={highlightKey === key
              ? `color:${colors[key] ?? providerColor(key)};border-color:${stroke(key)};background:${fill(key)}`
              : ""}
            onclick={(e) => {
              e.stopPropagation();
              pick(key);
            }}
          >
            <i style={`background:${colors[key] ?? providerColor(key)}`}></i>
            {providerLabel(key)}
          </button>
        {/each}
      </div>
    {/if}
  {/if}

  <div class="cal-sec__grid" onclick={(e) => e.stopPropagation()} role="presentation">
    <HeatmapGrid
      {highlightKey}
      emptyHint="暂无用量记录——使用 Claude Code / ZCode 等工具后会自动写入账本"
    />
  </div>
</section>

<style>
  .cal-sec {
    flex: none;
    border-top: 1px solid var(--tum-border);
    padding-top: 8px;
  }

  .cal-sec__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 6px;
  }

  .cal-sec__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .cal-sec__sel {
    font-family: var(--tum-font);
    font-size: 10px;
    color: var(--tum-text-primary);
    background: rgba(45, 50, 60, 0.96);
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    padding: 2px 6px;
    outline: none;
    cursor: pointer;
    max-width: 190px;
  }

  .cal-sec__sel:focus-visible {
    border-color: var(--tum-accent);
  }

  .cal-sec__sel option {
    background-color: var(--tum-bg-solid);
    color: var(--tum-text-primary);
  }

  .cal-sec__range {
    font-size: 10px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .cal-sec__pills {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    margin-bottom: 6px;
  }

  .cal-sec__pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 21px;
    padding: 0 9px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-pill);
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .cal-sec__pill:hover {
    background: var(--tum-surface-hover);
    color: var(--tum-text-primary);
  }

  .cal-sec__pill.is-on {
    font-weight: 600;
  }

  .cal-sec__pill i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    display: inline-block;
  }

  /* Provider 过多时的降级形态：标签 + 下拉，单行不换行。 */
  .cal-sec__pills--select {
    flex-wrap: nowrap;
    align-items: center;
  }

  .cal-sec__pill-label {
    font-size: 10px;
    color: var(--tum-text-muted);
    flex: none;
  }

  .cal-sec__sel--focus {
    flex: 1;
    min-width: 0;
    max-width: none;
  }
</style>
