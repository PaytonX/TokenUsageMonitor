<script lang="ts">
  // 本机工具用量面板（C8 / B7）：读取本地 AI 工具会话日志聚合的逐日 token 用量。
  // 首版支持 Claude Code（后端 local::claude 扫描 ~/.claude/projects）。展示
  // 累计/会话/项目统计 + 跨日期的 input/cache/output 堆叠柱状。
  import { getLocalTools, onToolsUpdated, openToolWindow } from "../api";
  import { readPref, writePref } from "../prefs";
  import type { LocalDay, LocalToolsPayload } from "../types";
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
    RANGES.some((r) => r.key === (readPref("tum.tool.range", "30d") as RangeKey))
      ? (readPref("tum.tool.range", "30d") as RangeKey)
      : "30d",
  );
  let rangeDays = $derived(RANGES.find((r) => r.key === range)!.days);

  let payload = $state<LocalToolsPayload | null>(null as LocalToolsPayload | null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let activeId: string | null = $state(readPref("tum.tool.tab", "") || null);

  async function load(force = false) {
    loading = true;
    error = null;
    try {
      payload = await getLocalTools(force);
      if (activeId === null || !payload.tools.some((t) => t.id === activeId)) {
        activeId = payload.tools[0]?.id ?? null;
      }
      writePref("tum.tool.tab", activeId ?? "");
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

  // 当前选中的工具窗口：取最近 N 天的逐日数据（旧→新）。
  let tool = $derived(payload?.tools.find((t) => t.id === activeId) ?? payload?.tools[0] ?? null);
  let win: LocalDay[] = $derived.by(() => {
    if (!tool) return [];
    const all = tool.daily;
    return all.length > rangeDays ? all.slice(all.length - rangeDays) : all;
  });

  let rangeTotal = $derived(win.reduce((s, d) => s + d.total, 0));
  let rangeInput = $derived(win.reduce((s, d) => s + d.input, 0));
  let rangeCache = $derived(win.reduce((s, d) => s + d.cache_read, 0));
  let rangeOutput = $derived(win.reduce((s, d) => s + d.output, 0));

  // 线图数据：生成区间内的连续日期序列（无用量日补 0），让 x 轴等距、刻度统一，
  // 避免 30/90 天出现柱状图的"显不全 / 刻度参差"问题。
  let dailyByDate = $derived(
    tool ? new Map(tool.daily.map((d) => [d.date, d])) : new Map<string, LocalDay>(),
  );
  let lineDays = $derived.by(() => {
    if (!tool) return [] as { date: string; label: string; parts: { id: string; value: number }[]; total: number }[];
    const out: { date: string; label: string; parts: { id: string; value: number }[]; total: number }[] = [];
    const today = new Date();
    for (let i = rangeDays - 1; i >= 0; i--) {
      const dt = new Date(today);
      dt.setDate(today.getDate() - i);
      const m = String(dt.getMonth() + 1).padStart(2, "0");
      const dd = String(dt.getDate()).padStart(2, "0");
      const key = `${dt.getFullYear()}-${m}-${dd}`;
      const total = dailyByDate.get(key)?.total ?? 0;
      out.push({
        date: key,
        label: `${m}/${dd}`,
        parts: total > 0 ? [{ id: tool.id, value: total }] : [],
        total,
      });
    }
    return out;
  });
  let lineColors = $derived(tool ? { [tool.id]: accent } : {});
  let maxLine = $derived(Math.max(1, ...lineDays.map((d) => d.total)));

  function fmtTokens(v: number): string {
    if (v >= 1_000_000) return `${(v / 1e6).toFixed(1)}M`;
    if (v >= 1_000) return `${(v / 1e3).toFixed(1)}K`;
    return `${v}`;
  }

  const BANDS = [
    { key: "input", label: "输入", color: "#76a9ff" },
    { key: "cache_read", label: "缓存", color: "#ffcc66" },
    { key: "output", label: "输出", color: "#4cc2ff" },
  ] as const;
</script>

<div class="tool" data-tauri-drag-region={false}>
  <div class="tool__head">
    <span class="tool__title">本机工具</span>
    <div class="tool__head-right" role="group" aria-label="时间区间">
      {#each RANGES as r (r.key)}
        <button
          type="button"
          class="tool__range"
          class:is-active={range === r.key}
          onclick={() => {
            range = r.key;
            writePref("tum.tool.range", r.key);
          }}
        >{r.label}</button>
      {/each}
      <button
        type="button"
        class="tool__refresh"
        title="重新扫描本地日志"
        onclick={() => void load(true)}
      >↻</button>
      <button
        type="button"
        class="tool__zoom"
        title="放大为独立窗口"
        onclick={() => void openToolWindow()}
      >⤢</button>
    </div>
  </div>

  {#if loading && !payload}
    <div class="tool__empty">正在扫描本地工具日志…</div>
  {:else if error && !payload}
    <div class="tool__empty tool__empty--err">扫描失败：{error}</div>
  {:else if !tool}
    <div class="tool__empty">未发现本地工具日志（{payload?.sessions_parsed ?? 0} 会话）</div>
  {:else}
    {#if (payload?.tools.length ?? 0) > 1}
      <PillsOrSelect
        items={(payload!.tools ?? []).map((t) => ({ id: t.id, label: t.name }))}
        value={tool?.id ?? null}
        onPick={(id) => {
            activeId = id;
            writePref("tum.tool.tab", id);
          }}
      />
    {/if}
    <div class="tool__stats">
      <span class="tool__stat"><b>{fmtTokens(tool.total_tokens)}</b><span>累计 tokens</span></span>
      <span class="tool__stat"><b>{tool.session_count}</b><span>会话</span></span>
      <span class="tool__stat"><b>{tool.project_count}</b><span>项目</span></span>
    </div>

    <div class="tool__breakdown">
      <span class="tool__bd"><i style={`background:${BANDS[0].color}`}></i>输入 {fmtTokens(rangeInput)}</span>
      <span class="tool__bd"><i style={`background:${BANDS[1].color}`}></i>缓存 {fmtTokens(rangeCache)}</span>
      <span class="tool__bd"><i style={`background:${BANDS[2].color}`}></i>输出 {fmtTokens(rangeOutput)}</span>
    </div>

    <div class="tool__chart">
      <TrendLineChart days={lineDays} colors={lineColors} maxY={maxLine} tickEvery={1} {accent} />
    </div>
  {/if}
</div>

<style>
  .tool {
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

  .tool__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    flex: none;
  }

  .tool__head-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .tool__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .tool__range {
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

  .tool__range:hover {
    color: var(--tum-text-primary);
  }

  .tool__range.is-active {
    background: rgba(76, 194, 255, 0.18);
    color: var(--tum-text-primary);
  }

  .tool__refresh {
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

  .tool__refresh:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .tool__zoom {
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

  .tool__zoom:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .tool__stats {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    flex: none;
  }

  .tool__stat {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
  }

  .tool__stat b {
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
  }

  .tool__stat span {
    color: var(--tum-text-muted);
    font-size: 10px;
  }

  .tool__breakdown {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    flex: none;
  }

  .tool__bd {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--tum-font-mono);
    font-size: 10px;
    color: var(--tum-text-secondary);
  }

  .tool__bd i {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    display: inline-block;
  }

  .tool__chart {
    flex: 1;
    min-height: 0;
    display: flex;
    margin: 0 -2px;
  }

  .tool__empty {
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

  .tool__empty--err {
    color: var(--tum-danger);
  }
</style>