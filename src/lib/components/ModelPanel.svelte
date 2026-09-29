<script lang="ts">
  // 模型用量面板（B5 模型视图 / 本地工具按模型聚合）：跨工具把相同模型归并，
  // 展示各模型的 token 总量与成本；点选模型查看其逐日走势（线图）。
  // 数据复用 get_local_tools 的 models 字段（claude/cherry/minimax 已按模型聚合）。
  //
  // 布局（2026-09 重构）：早先的「圆环 + 侧边图例」在模型数 >6 时把尾部全塞进
  // 不可选的「其他」切片，且 118px 环旁的图例放不下数字。改为**可搜索的模型
  // 列表**：每行自带色点、用量、占比与内联占比条——占比信息与圆环等价，但
  // 所有模型都可直接选中，列表随模型数量增长只滚动、不挤压。详情（统计 +
  // 折线）固定在列表下方。
  import { getLocalTools, onToolsUpdated } from "../api";
  import { readPref, writePref } from "../prefs";
  import type { LocalModelUsage, LocalToolsPayload } from "../types";
  import { displayCurrency, formatCost, normalizeCurrency, toUsd } from "../currency";
  import type { Currency } from "../currency";
  import TrendLineChart from "./TrendLineChart.svelte";

  interface Props {
    accent?: string;
  }
  let { accent = "#4cc2ff" }: Props = $props();

  type RangeKey = "7d" | "30d" | "90d";
  const RANGES: { key: RangeKey; label: string; days: number }[] = [
    { key: "7d", label: "近 7 天", days: 7 },
    { key: "30d", label: "近 30 天", days: 30 },
    { key: "90d", label: "近 90 天", days: 90 },
  ];

  let range: RangeKey = $state(
    RANGES.some((r) => r.key === (readPref("tum.model.range", "30d") as RangeKey))
      ? (readPref("tum.model.range", "30d") as RangeKey)
      : "30d",
  );
  let rangeDays = $derived(RANGES.find((r) => r.key === range)!.days);

  let payload: LocalToolsPayload | null = $state(null as LocalToolsPayload | null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let activeId: string | null = $state(readPref("tum.model.tab", "") || null);
  // 搜索词只活在会话内：持久化会让"忘了清的过滤词"伪装成空列表。
  let search = $state("");

  async function load(force = false) {
    loading = true;
    error = null;
    try {
      payload = await getLocalTools(force);
      if (activeId === null || !modelList.some((m) => m.id === activeId)) {
        activeId = modelList[0]?.id ?? null;
      }
      writePref("tum.model.tab", activeId ?? "");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void load();
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void onToolsUpdated(() => {
      if (!disposed) void load();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // 归一化模型名：合并 claude("MiniMax-M3") / cherry("MiniMax-M3") /
  // minimax("minimax/MiniMax-M3") 等写法为短名。
  function shortName(raw: string): string {
    const idx = raw.lastIndexOf("/");
    return idx >= 0 ? raw.slice(idx + 1) : raw;
  }

  // 跨工具归并同一模型：汇总 total/cost，并合并逐日序列；记录来源工具数。
  // 成本口径：各工具上报币种不同（cherry=CNY，claude/codex/minimax/hermes=USD），
  // 必须先按静态汇率折算成 USD 再累加——直接把不同币种的原始数值相加是错的。
  // 折算只发生在"合并"这一步；展示仍按用户所选展示币种（见 fmtCost）。
  let modelList = $derived.by(() => {
    const byName = new Map<string, {
      id: string; name: string; total: number; costUsd: number;
      currencies: Set<Currency>; cost_estimated: boolean; sources: Set<string>;
      daily: Map<string, LocalModelUsage["daily"][number]>;
    }>();
    for (const tool of payload?.tools ?? []) {
      for (const mu of tool.models) {
        const key = shortName(mu.model);
        let agg = byName.get(key);
        if (!agg) {
          agg = { id: key, name: key, total: 0, costUsd: 0, currencies: new Set(), cost_estimated: false, sources: new Set(), daily: new Map() };
          byName.set(key, agg);
        }
        agg.total += mu.total_tokens;
        agg.sources.add(tool.name);
        if (mu.cost > 0) {
          const cur = normalizeCurrency(mu.currency);
          agg.costUsd += toUsd(mu.cost, cur);
          agg.currencies.add(cur);
          if (mu.cost_estimated) agg.cost_estimated = true;
        }
        for (const d of mu.daily) {
          const cur = agg.daily.get(d.date);
          if (cur) {
            cur.total += d.total;
            cur.input += d.input;
            cur.cache_read += d.cache_read;
            cur.output += d.output;
          } else {
            agg.daily.set(d.date, { ...d });
          }
        }
      }
    }
    return [...byName.values()]
      .sort((a, b) => b.total - a.total)
      .map((m) => ({ ...m, daily: [...m.daily.values()].sort((a, b) => a.date.localeCompare(b.date)) }));
  });

  let grandTotal = $derived(modelList.reduce((s, m) => s + m.total, 0));
  let maxModelTotal = $derived(Math.max(1, ...modelList.map((m) => m.total)));

  let filteredList = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return modelList;
    return modelList.filter((m) => m.name.toLowerCase().includes(q));
  });

  // 搜索后选中项若被过滤掉，详情仍显示原选中模型（列表只是视图过滤）。
  let active = $derived(modelList.find((m) => m.id === activeId) ?? null);

  // 稳定配色：颜色按 id 哈希分配，避免总量排名变化时同一模型换色。
  const PALETTE = ["#5fd4a2", "#f2b35b", "#f27b9b", "#7b93f2", "#4cc2ff", "#b58cf5", "#6fd1d1"];
  function colorFor(id: string): string {
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
    return PALETTE[Math.abs(h) % PALETTE.length];
  }

  let lineDays = $derived.by(() => {
    if (!active) return [] as { date: string; label: string; parts: { id: string; value: number }[]; total: number }[];
    const today = new Date();
    const out: { date: string; label: string; parts: { id: string; value: number }[]; total: number }[] = [];
    const byDate = new Map(active.daily.map((d) => [d.date, d]));
    for (let i = rangeDays - 1; i >= 0; i--) {
      const dt = new Date(today);
      dt.setDate(today.getDate() - i);
      const m = String(dt.getMonth() + 1).padStart(2, "0");
      const dd = String(dt.getDate()).padStart(2, "0");
      const key = `${dt.getFullYear()}-${m}-${dd}`;
      const total = byDate.get(key)?.total ?? 0;
      out.push({ date: key, label: `${m}/${dd}`, parts: total > 0 ? [{ id: active.id, value: total }] : [], total });
    }
    return out;
  });
  let maxLine = $derived(Math.max(1, ...lineDays.map((d) => d.total)));
  // 折线用该模型的稳定配色，与列表色点一致；accent 仅作图表 chrome 色。
  let lineColors = $derived(active ? { [active.id]: colorFor(active.id) } : {});
  // 当前区间（7/30/90 天）的累计用量。
  let rangeTotal = $derived(lineDays.reduce((s, d) => s + d.total, 0));

  function fmtTokens(v: number): string {
    if (v >= 1_000_000) return `${(v / 1e6).toFixed(1)}M`;
    if (v >= 1_000) return `${(v / 1e3).toFixed(1)}K`;
    return `${v}`;
  }
  // 成本展示：聚合成本已折算为 USD（见 modelList 合并注释），这里再按展示币种渲染。
  // 三态口径：
  // - 估算成本（后端按价格表推算）→ formatCost(approximate)，"≈" + 低精度；
  // - 混合币种聚合 → "≈" 前缀（跨币种折算用离线静态汇率，属近似值），正常精度；
  // - 单一币种 → 正常展示（展示币种与源币种相同时静态汇率往返无损）。
  function fmtCost(m: { costUsd: number; currencies: Set<Currency>; cost_estimated: boolean }): string {
    if (m.costUsd <= 0) return "";
    if (m.cost_estimated) return formatCost(m.costUsd, displayCurrency(), true);
    if (m.currencies.size > 1) return `≈ ${formatCost(m.costUsd, displayCurrency())}`;
    return formatCost(m.costUsd, displayCurrency());
  }
  // 成本标签：让 "≈" 有解释（估算 / 跨币种折算）。
  function costLabel(m: { currencies: Set<Currency>; cost_estimated: boolean }): string {
    if (m.cost_estimated) return "成本 · 估算";
    if (m.currencies.size > 1) return "成本 · 折算";
    return "成本";
  }
  function selectModel(id: string) {
    activeId = id;
    writePref("tum.model.tab", id);
  }
</script>

<div class="mp">
  <div class="mp__head">
    <span class="mp__title">模型用量</span>
    <div class="mp__head-right" role="group" aria-label="时间区间">
      {#each RANGES as r (r.key)}
        <button type="button" class="mp__range" class:is-active={range === r.key} onclick={() => { range = r.key; writePref("tum.model.range", r.key); }}>{r.label}</button>
      {/each}
      <button type="button" class="mp__refresh" title="重新扫描本地日志" onclick={() => void load(true)}>↻</button>
    </div>
  </div>

  {#if loading && !payload}
    <div class="mp__empty">正在扫描本地工具日志…</div>
  {:else if error && !payload}
    <div class="mp__empty mp__empty--err">扫描失败：{error}</div>
  {:else if modelList.length === 0}
    <div class="mp__empty">暂未上报模型级用量</div>
  {:else}
    <div class="mp__search">
      <span class="mp__search-glyph" aria-hidden="true">⌕</span>
      <input
        type="text"
        class="mp__search-input tum-numeric"
        placeholder="搜索模型…"
        aria-label="搜索模型"
        bind:value={search}
      />
      {#if search}
        <button type="button" class="mp__search-clear" title="清除搜索" aria-label="清除搜索" onclick={() => { search = ""; }}>×</button>
      {/if}
    </div>

    <div class="mp__list" role="listbox" aria-label="模型列表">
      {#each filteredList as m (m.id)}
        <button
          type="button"
          class="mp__row"
          class:mp__row--active={m.id === activeId}
          role="option"
          aria-selected={m.id === activeId}
          title="{m.name} · {fmtTokens(m.total)} tokens（{grandTotal > 0 ? ((m.total / grandTotal) * 100).toFixed(1) : '0'}%）{m.sources.size > 1 ? ` · 来自 ${[...m.sources].join('、')}` : ''}"
          onclick={() => selectModel(m.id)}
        >
          <span class="mp__row-top">
            <span class="mp__row-dot" style="background: {colorFor(m.id)};" aria-hidden="true"></span>
            <span class="mp__row-name">{m.name}</span>
            {#if fmtCost(m)}
              <span class="mp__row-cost">{fmtCost(m)}</span>
            {/if}
            <span class="mp__row-total tum-numeric">{fmtTokens(m.total)}</span>
            <span class="mp__row-share tum-numeric">{grandTotal > 0 ? ((m.total / grandTotal) * 100).toFixed(m.total / grandTotal >= 0.1 ? 0 : 1) : '0'}%</span>
          </span>
          <span class="mp__row-bar" aria-hidden="true">
            <span class="mp__row-bar-fill" style="width: {(m.total / maxModelTotal) * 100}%; background: {colorFor(m.id)};"></span>
          </span>
        </button>
      {:else}
        <div class="mp__list-none">无匹配「{search}」的模型</div>
      {/each}
    </div>

    {#if active}
      <div class="mp__detail">
        <div class="mp__detail-head">
          <span class="mp__detail-dot" style="background: {colorFor(active.id)};" aria-hidden="true"></span>
          <span class="mp__detail-name" title={active.name}>{active.name}</span>
          {#if active.sources.size > 1}
            <span class="mp__detail-src" title={[...active.sources].join('、')}>{active.sources.size} 个工具</span>
          {/if}
        </div>
        <div class="mp__stats">
          <span class="mp__stat"><b>{fmtTokens(rangeTotal)}</b><span>近 {rangeDays} 天</span></span>
          <span class="mp__stat"><b>{fmtTokens(active.total)}</b><span>累计 tokens</span></span>
          {#if active.costUsd > 0}
            <span class="mp__stat"><b>{fmtCost(active)}</b><span>{costLabel(active)}</span></span>
          {/if}
        </div>
        <div class="mp__chart">
          <TrendLineChart days={lineDays} colors={lineColors} maxY={maxLine} tickEvery={1} {accent} />
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .mp {
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
  }

  .mp__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    flex: none;
  }

  .mp__head-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .mp__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .mp__range {
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
  .mp__range:hover {
    color: var(--tum-text-primary);
  }
  .mp__range.is-active {
    background: rgba(76, 194, 255, 0.18);
    color: var(--tum-text-primary);
  }

  .mp__refresh {
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
  .mp__refresh:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  /* —— 搜索框 —— */
  .mp__search {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-pill);
    background: rgba(255, 255, 255, 0.04);
    padding: 3px 8px;
    transition: border-color 0.15s ease;
  }
  .mp__search:focus-within {
    border-color: var(--tum-accent-stroke);
  }
  .mp__search-glyph {
    color: var(--tum-text-muted);
    font-size: 12px;
    line-height: 1;
  }
  .mp__search-input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--tum-text-primary);
    font-size: 11px;
    font-family: var(--tum-font);
    padding: 0;
  }
  .mp__search-input::placeholder {
    color: var(--tum-text-muted);
  }
  .mp__search-clear {
    flex: none;
    width: 14px;
    height: 14px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
    color: var(--tum-text-secondary);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
  }
  .mp__search-clear:hover {
    background: rgba(255, 255, 255, 0.16);
    color: var(--tum-text-primary);
  }

  /* —— 模型列表（替代圆环 + 图例）—— */
  .mp__list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 -4px;
    padding: 0 4px;
  }
  .mp__list-none {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 14px 8px;
    color: var(--tum-text-secondary);
    font-size: 11px;
    font-family: var(--tum-font-mono);
  }
  .mp__row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    border: none;
    border-radius: 8px;
    background: transparent;
    padding: 5px 7px;
    text-align: left;
    cursor: pointer;
    transition: background 0.15s ease;
  }
  .mp__row:hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .mp__row--active {
    background: rgba(255, 255, 255, 0.09);
  }
  .mp__row:focus-visible {
    outline: 1px solid var(--tum-accent-stroke);
    outline-offset: -1px;
  }
  .mp__row-top {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .mp__row-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    align-self: center;
  }
  .mp__row-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--tum-text-secondary);
  }
  .mp__row--active .mp__row-name {
    color: var(--tum-text-primary);
  }
  .mp__row-cost {
    flex: none;
    font-size: 10px;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-muted);
  }
  .mp__row-total {
    flex: none;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--tum-text-primary);
  }
  .mp__row-share {
    flex: none;
    width: 34px;
    text-align: right;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 10px;
    color: var(--tum-text-muted);
  }
  .mp__row-bar {
    display: block;
    height: 2px;
    border-radius: 1px;
    background: rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }
  .mp__row-bar-fill {
    display: block;
    height: 100%;
    border-radius: 1px;
    opacity: 0.85;
    transition: width 0.3s var(--tum-ease-spring);
  }

  /* —— 选中模型详情 —— */
  .mp__detail {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-top: 1px solid var(--tum-border);
    padding-top: 8px;
  }
  .mp__detail-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .mp__detail-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .mp__detail-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--tum-text-primary);
  }
  .mp__detail-src {
    flex: none;
    font-size: 9px;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-muted);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-pill);
    padding: 1px 6px;
  }

  .mp__stats {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    flex: none;
  }
  .mp__stat {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
  }
  .mp__stat b {
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
  }
  .mp__stat span {
    color: var(--tum-text-muted);
    font-size: 10px;
  }

  .mp__chart {
    flex: none;
    height: 120px;
    display: flex;
    margin: 0 -2px;
  }

  .mp__empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tum-text-secondary);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    text-align: center;
    padding: 0 8px;
  }
  .mp__empty--err {
    color: var(--tum-danger);
  }
</style>
