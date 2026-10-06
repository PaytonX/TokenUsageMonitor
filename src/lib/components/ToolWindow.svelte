<script lang="ts">
  // 独立工具用量窗口（C8 / B7）：可拖拽缩放、最大化。展示所选本地工具逐日
  // input/cache/output 堆叠柱（柱宽固定，窗口放不下时横向滚动；放大窗口可看
  // 更多更粗），附工具切换、区间切换、统计与图例。窗口自读 get_local_tools。
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getLocalTools } from "../api";
  import type { LocalDay, LocalToolReport, LocalToolsPayload } from "../types";
  import { hexToRgb } from "../types";
  import PillsOrSelect from "./PillsOrSelect.svelte";
  import { PanelHeader, RangePills, ColorSwatch } from "./atoms";

  type RangeKey = "7d" | "30d" | "90d";
  const RANGES: { key: RangeKey; label: string; days: number }[] = [
    { key: "7d", label: "近 7 天", days: 7 },
    { key: "30d", label: "近 30 天", days: 30 },
    { key: "90d", label: "近 90 天", days: 90 },
  ];

  const BANDS = [
    { key: "input", label: "输入", color: "#76a9ff" },
    { key: "cache_read", label: "缓存", color: "#ffcc66" },
    { key: "output", label: "输出", color: "#4cc2ff" },
  ] as const;

  let status: "loading" | "ready" | "empty" = $state("loading");
  let payload: LocalToolsPayload | null = $state(null as LocalToolsPayload | null);
  let activeId: string | null = $state(null);
  let range: RangeKey = $state("30d");
  let rangeDef = $derived(RANGES.find((r) => r.key === range)!);

  // 与嵌入版 ToolPanel 同构：activeId 为 null = 全部工具（逐日求和的聚合视图）。
  // 焦点只有一个含义——下方柱图画它，上方堆叠图也压暗其余工具。
  let view = $derived.by<LocalToolReport | null>(() => {
    const tools = payload?.tools ?? [];
    if (activeId) {
      const t = tools.find((x) => x.id === activeId);
      if (t) return t;
    }
    if (tools.length === 0) return null;
    const byDate = new Map<string, LocalDay>();
    for (const t of tools) {
      for (const d of t.daily) {
        const cur = byDate.get(d.date) ?? { date: d.date, input: 0, cache_read: 0, output: 0, total: 0 };
        cur.input += d.input;
        cur.cache_read += d.cache_read;
        cur.output += d.output;
        cur.total += d.total;
        byDate.set(d.date, cur);
      }
    }
    const daily = [...byDate.values()].sort((a, b) => a.date.localeCompare(b.date));
    return {
      id: "__all__",
      name: "全部工具",
      daily,
      total_tokens: daily.reduce((s, d) => s + d.total, 0),
      session_count: tools.reduce((s, t) => s + t.session_count, 0),
      project_count: tools.reduce((s, t) => s + t.project_count, 0),
      scanned_at: tools[0]?.scanned_at ?? "",
      models: [],
    };
  });
  let win: LocalDay[] = $derived.by(() => {
    if (!view) return [];
    // 生成区间内连续日期（无用量补 0），保证 x 轴等距、柱底对齐。
    const byDate = new Map(view.daily.map((d) => [d.date, d]));
    const out: LocalDay[] = [];
    const today = new Date();
    for (let i = rangeDef.days - 1; i >= 0; i--) {
      const dt = new Date(today);
      dt.setDate(today.getDate() - i);
      const m = String(dt.getMonth() + 1).padStart(2, "0");
      const dd = String(dt.getDate()).padStart(2, "0");
      const key = `${dt.getFullYear()}-${m}-${dd}`;
      const rec = byDate.get(key);
      out.push({
        date: key,
        input: rec?.input ?? 0,
        cache_read: rec?.cache_read ?? 0,
        output: rec?.output ?? 0,
        total: rec?.total ?? 0,
      });
    }
    return out;
  });

  let maxTotal = $derived(Math.max(1, ...win.map((d) => d.total)));
  let rangeTotal = $derived(win.reduce((s, d) => s + d.total, 0));

  // 刻度：按可视宽度自适应步长，杜绝两端交叠。
  const LABEL_GAP_PX = 44;
  let bodyW = $state(0);
  let tickEvery = $derived.by(() => {
    let base = rangeDef.days <= 7 ? 1 : rangeDef.days <= 30 ? 3 : 7;
    const usable = Math.max(1, bodyW - 8);
    const n = Math.max(1, win.length);
    return Math.max(base, Math.ceil(((n - 1) * LABEL_GAP_PX) / usable));
  });
  function tickVisible(i: number): boolean {
    if (i === 0 || i === win.length - 1) return true;
    if (i % tickEvery !== 0) return false;
    return win.length - 1 - i >= tickEvery;
  }
  // 始终预留刻度槽（固定高度），无标签时隐藏文本占住空间，保证柱底对齐。
  function bandStyle(color: string): string {
    const rgb = hexToRgb(color) ?? "118,169,255";
    return `background: rgba(${rgb}, 0.85)`;
  }

  onMount(async () => {
    try {
      payload = await getLocalTools(false);
      activeId = null; // 默认「全部工具」，与嵌入版一致
      status = payload && payload.tools.length > 0 ? "ready" : "empty";
    } catch {
      status = "empty";
    }
  });

  function fmtTokens(v: number): string {
    if (v >= 1_000_000) return `${(v / 1e6).toFixed(1)}M`;
    if (v >= 1_000) return `${(v / 1e3).toFixed(1)}K`;
    return `${v}`;
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
    <span class="tw__bar-title">工具用量</span>
    <div class="tw__bar-controls">
      <button type="button" class="tw__bar-btn" aria-label="最小化" onclick={minimize} onpointerdown={(e) => e.stopPropagation()}>—</button>
      <button type="button" class="tw__bar-btn" aria-label="最大化/还原" onclick={toggleMaximize} onpointerdown={(e) => e.stopPropagation()}>▢</button>
      <button type="button" class="tw__bar-btn tw__bar-btn--close" aria-label="关闭" onclick={close} onpointerdown={(e) => e.stopPropagation()}>✕</button>
    </div>
  </div>
  <PanelHeader title="工具用量" label="工具用量窗口">
    <PillsOrSelect
      label="选择工具"
      items={[
        { id: "", label: "全部工具" },
        ...(payload?.tools ?? []).map((t) => ({ id: t.id, label: t.name })),
      ]}
      value={activeId ?? ""}
      onPick={(id) => (activeId = id === "" ? null : id)}
    />
    <RangePills
      options={RANGES}
      value={range}
      onChange={(k) => (range = k)}
      label="时间区间"
    />
    <span class="tw__stats">本区间 <b>{fmtTokens(rangeTotal)}</b></span>
  </PanelHeader>

  {#if status === "empty"}
    <div class="tw__empty">暂无本地工具用量数据</div>
  {:else if status === "loading"}
    <div class="tw__empty">正在扫描本地工具日志…</div>
  {:else}
    <div class="tw__body" bind:clientWidth={bodyW}>
      <div class="tw__chart">
        {#each win as d, i (d.date)}
          <div class="tw__day" title={`${d.date} · 输入 ${fmtTokens(d.input)} / 缓存 ${fmtTokens(d.cache_read)} / 输出 ${fmtTokens(d.output)}`}>
            <div class="tw__col">
              {#if d.total > 0}
                <div class="tw__seg" style={`height:${(d.input / maxTotal) * 100}%;${bandStyle(BANDS[0].color)}`}></div>
                <div class="tw__seg" style={`height:${(d.cache_read / maxTotal) * 100}%;${bandStyle(BANDS[1].color)}`}></div>
                <div class="tw__seg" style={`height:${(d.output / maxTotal) * 100}%;${bandStyle(BANDS[2].color)}`}></div>
              {:else}
                <div class="tw__seg tw__seg--zero"></div>
              {/if}
            </div>
            <span class="tw__tick" class:tw__tick--hidden={!tickVisible(i)}>{tickVisible(i) ? d.date.slice(5) : ""}</span>
          </div>
        {/each}
      </div>
    </div>

    <div class="tw__legend">
      {#each BANDS as b (b.key)}
        <span class="tw__key"><ColorSwatch color={b.color} size={9} />{b.label}</span>
      {/each}
      <span class="tw__legend-hint">放大本窗口可获得更粗的柱与更多细节</span>
    </div>
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
    font-size: var(--tum-font-size-sm);
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
    font-size: var(--tum-font-size-sm);
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--tum-radius-xs);
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

  .tw__stats {
    flex: 1;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
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
    background: var(--tum-surface);
    height: 3px;
  }

  .tw__tick {
    flex: none;
    height: 14px;
    line-height: 14px;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    white-space: nowrap;
  }

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
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-secondary);
  }

  .tw__legend-hint {
    margin-left: auto;
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
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