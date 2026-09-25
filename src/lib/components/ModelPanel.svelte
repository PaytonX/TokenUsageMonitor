<script lang="ts">
  // 模型用量面板（B5 模型视图 / 本地工具按模型聚合）：跨工具把相同模型归并，
  // 展示各模型的 token 总量与成本；点选模型查看其逐日走势（线图）。
  // 数据复用 get_local_tools 的 models 字段（claude/cherry/minimax 已按模型聚合）。
  import { getLocalTools, onToolsUpdated } from "../api";
  import { readPref, writePref } from "../prefs";
  import type { LocalModelUsage, LocalToolsPayload } from "../types";
  import { displayCurrency, formatCost, normalizeCurrency, toUsd } from "../currency";
  import type { Currency } from "../currency";
  import PillsOrSelect from "./PillsOrSelect.svelte";
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

  // 跨工具归并同一模型：汇总 total/cost，并合并逐日序列。
  // 成本口径：各工具上报币种不同（cherry=CNY，claude/codex/minimax/hermes=USD），
  // 必须先按静态汇率折算成 USD 再累加——直接把不同币种的原始数值相加是错的。
  // 折算只发生在"合并"这一步；展示仍按用户所选展示币种（见 fmtCost）。
  let modelList = $derived.by(() => {
    const byName = new Map<string, { id: string; name: string; total: number; costUsd: number; currencies: Set<Currency>; cost_estimated: boolean; daily: Map<string, LocalModelUsage["daily"][number]> }>();
    for (const tool of payload?.tools ?? []) {
      for (const mu of tool.models) {
        const key = shortName(mu.model);
        let agg = byName.get(key);
        if (!agg) {
          agg = { id: key, name: key, total: 0, costUsd: 0, currencies: new Set(), cost_estimated: false, daily: new Map() };
          byName.set(key, agg);
        }
        agg.total += mu.total_tokens;
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

  let active = $derived(modelList.find((m) => m.id === activeId) ?? null);

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
  let lineColors = $derived(active ? { [active.id]: accent } : {});
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
    <PillsOrSelect
      items={modelList.map((m) => ({ id: m.id, label: `${m.name} · ${fmtTokens(m.total)}` }))}
      value={activeId}
      onPick={(id) => { activeId = id; writePref("tum.model.tab", id); }}
    />

    {#if active}
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
    flex: 1;
    min-height: 0;
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