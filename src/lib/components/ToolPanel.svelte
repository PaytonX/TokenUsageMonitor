<script lang="ts">
  // 本机工具用量面板（C8 / B7）：读取本地 AI 工具会话日志聚合的逐日 token 用量。
  // 首版支持 Claude Code（后端 local::claude 扫描 ~/.claude/projects）。展示
  // 累计/会话/项目统计 + 跨日期的 input/cache/output 堆叠柱状。
  import { getHubDevices, getLocalTools, getUsageHistory, onTabsChanged, onToolsUpdated, openToolWindow } from "../api";
  import { readAggMode, readPref, writePref } from "../prefs";
  import { buildAggregateToolsPayload, stackColors, staleDetailDevices } from "../device-agg";
  import { dimOthers } from "../trend-data";
  import type { LocalDay, LocalToolReport, LocalToolsPayload } from "../types";
  import PillsOrSelect from "./PillsOrSelect.svelte";
  import TrendLineChart from "./TrendLineChart.svelte";
  import { PanelHeader, RangePills, ZoomButton, Stat, ColorSwatch } from "./atoms";

  /** 「全部工具」聚合视图的虚拟 id（不对应任何真实工具）。 */
  const ALL_TOOLS_ID = "__all__";

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
  // 全端汇总模式：数据源从本机扫描切到多端合并（设置页开关，tabs-changed 广播）。
  let aggMode = $state(readAggMode());
  let staleDevs = $state<string[]>([]);
  // 全工具堆叠面积图（账本/设备序列，按工具堆叠；随 range 刷新）。
  let stackIds = $state<string[]>([]);
  let stackSeries = $state<Record<string, Record<string, number>>>({});
  let stackNames = $state<Record<string, string>>({});
  let stackRefresh = $state(0);

  async function loadStack() {
    try {
      if (aggMode) {
        const r = await getHubDevices();
        // 全端模式：堆叠按设备（与设备页汇总同源）。
        const byDev = new Map<string, Map<string, number>>();
        for (const d of r.devices) {
          const m = new Map<string, number>();
          for (const day of d.daily ?? []) m.set(day.date, day.total);
          byDev.set(d.hostname || d.device_id, m);
        }
        stackIds = [...byDev.keys()];
        stackSeries = Object.fromEntries([...byDev.entries()].map(([k, m]) => [k, Object.fromEntries(m)]));
        stackNames = {};
        return;
      }
      const { rows } = await getUsageHistory(rangeDays);
      const byTool = new Map<string, Map<string, number>>();
      for (const r of rows) {
        if (r.kind !== "tool" || r.model !== "") continue;
        let m = byTool.get(r.source);
        if (!m) { m = new Map(); byTool.set(r.source, m); }
        m.set(r.date, (m.get(r.date) ?? 0) + r.total);
      }
      stackIds = [...byTool.keys()];
      stackSeries = Object.fromEntries([...byTool.entries()].map(([k, m]) => [k, Object.fromEntries(m)]));
      stackNames = {};
    } catch {
      /* 保留旧值 */
    }
  }

  async function load(force = false) {
    loading = true;
    error = null;
    try {
      if (aggMode) {
        const r = await getHubDevices();
        payload = buildAggregateToolsPayload(r.devices);
        staleDevs = staleDetailDevices(r.devices);
      } else {
        staleDevs = [];
        payload = await getLocalTools(force);
      }
      if (activeId && !payload.tools.some((t) => t.id === activeId)) {
        activeId = null;
      }
      writePref("tum.tool.tab", activeId ?? "");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
    void loadStack();
  }

  $effect(() => {
    void rangeDays;
    void stackRefresh;
    void aggMode;
    void loadStack();
  });

  $effect(() => {
    void load();
    let disposed = false;
    const unlistens: (() => void)[] = [];
    void onToolsUpdated(() => {
      if (!disposed) void load();
    }).then((fn) => {
      if (disposed) fn();
      else unlistens.push(fn);
    });
    void onTabsChanged(() => {
      if (disposed) return;
      const next = readAggMode();
      if (next !== aggMode) {
        aggMode = next;
        void load();
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlistens.push(fn);
    });
    return () => {
      disposed = true;
      for (const fn of unlistens) fn();
    };
  });

  // 当前视图：选中单个工具时用该工具；「全部工具」（activeId 为 null）时用
  // 各工具逐日求和的聚合视图。与趋势页的 provider 高亮同构——焦点只有一个含义，
  // 上方堆叠图和下方折线图都跟着它走。
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
      id: ALL_TOOLS_ID,
      name: "全部工具",
      daily,
      total_tokens: daily.reduce((s, d) => s + d.total, 0),
      session_count: tools.reduce((s, t) => s + t.session_count, 0),
      project_count: tools.reduce((s, t) => s + t.project_count, 0),
      scanned_at: tools[0]?.scanned_at ?? "",
      models: [],
    };
  });
  // 选中单个工具时，上方堆叠图把其余工具压暗（activeId 为 null = 全部，不压暗）。
  let stackColorsDim = $derived(
    activeId ? dimOthers(stackColors(stackIds), activeId) : stackColors(stackIds),
  );
  // 当前选中的工具窗口：取最近 N 天的逐日数据（旧→新）。
  let win: LocalDay[] = $derived.by(() => {
    if (!view) return [];
    const all = view.daily;
    return all.length > rangeDays ? all.slice(all.length - rangeDays) : all;
  });

  let rangeTotal = $derived(win.reduce((s, d) => s + d.total, 0));
  let rangeInput = $derived(win.reduce((s, d) => s + d.input, 0));
  let rangeCache = $derived(win.reduce((s, d) => s + d.cache_read, 0));
  let rangeOutput = $derived(win.reduce((s, d) => s + d.output, 0));

  // 线图数据：生成区间内的连续日期序列（无用量日补 0），让 x 轴等距、刻度统一，
  // 避免 30/90 天出现柱状图的"显不全 / 刻度参差"问题。
  let dailyByDate = $derived(
    view ? new Map(view.daily.map((d) => [d.date, d])) : new Map<string, LocalDay>(),
  );
  let lineDays = $derived.by(() => {
    if (!view) return [] as { date: string; label: string; parts: { id: string; value: number }[]; total: number }[];
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
        parts: total > 0 ? [{ id: view.id, value: total }] : [],
        total,
      });
    }
    return out;
  });
  let lineColors = $derived(view ? { [view.id]: accent } : {});
  let maxLine = $derived(Math.max(1, ...lineDays.map((d) => d.total)));

  let stackDays = $derived.by(() => {
    const today = new Date();
    const out: { date: string; label: string; parts: { id: string; value: number }[]; total: number }[] = [];
    for (let i = rangeDays - 1; i >= 0; i--) {
      const dt = new Date(today);
      dt.setDate(today.getDate() - i);
      const m = String(dt.getMonth() + 1).padStart(2, "0");
      const dd = String(dt.getDate()).padStart(2, "0");
      const key = `${dt.getFullYear()}-${m}-${dd}`;
      const parts = stackIds
        .map((id) => ({ id, value: stackSeries[id]?.[key] ?? 0 }))
        .filter((p) => p.value > 0);
      out.push({
        date: key,
        label: `${m}/${dd}`,
        parts,
        total: parts.reduce((s, p) => s + p.value, 0),
      });
    }
    return out;
  });
  let stackMax = $derived(Math.max(1, ...stackDays.map((d) => d.total)));

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
  <PanelHeader title={aggMode ? "全端工具" : "本机工具"}>
    <RangePills
      options={RANGES}
      value={range}
      accent={accent}
      onChange={(k) => { range = k; writePref("tum.tool.range", k); }}
    />
    <button
      type="button"
      class="tool__refresh"
      title={aggMode ? "刷新多端数据" : "重新扫描本地日志"}
      onclick={() => void load(true)}
    >↻</button>
    {#if !aggMode}
      <ZoomButton title="放大为独立窗口" onclick={() => void openToolWindow()} />
    {/if}
  </PanelHeader>

  {#if aggMode && staleDevs.length > 0 && (payload?.tools.length ?? 0) === 0}
    <div class="tool__empty tool__empty--err">
      参与设备中 {staleDevs.join("、")} 未上报分工具明细（版本过旧），无法合成全端视图
    </div>
  {/if}

  {#if loading && !payload}
    <div class="tool__empty">正在扫描本地工具日志…</div>
  {:else if error && !payload}
    <div class="tool__empty tool__empty--err">扫描失败：{error}</div>
  {:else}
    {#if stackIds.length > 0}
    <div class="tool__stack">
      <div class="tool__stack-head">
        <span class="tool__stack-title">{aggMode ? "全端各设备" : "全部工具"}</span>
        <span class="tool__stack-sub">按日堆叠 · tokens</span>
      </div>
      <TrendLineChart
        days={stackDays}
        colors={stackColorsDim}
        names={stackNames}
        maxY={stackMax}
        tickEvery={1}
        {accent}
        stacked={true}
      />
    </div>
  {/if}

  {#if !view}
    <div class="tool__empty">未发现本地工具日志（{payload?.sessions_parsed ?? 0} 会话）</div>
  {:else}
    {#if (payload?.tools.length ?? 0) > 0}
      <PillsOrSelect
        items={[
          { id: "", label: "全部工具" },
          ...(payload!.tools ?? []).map((t) => ({ id: t.id, label: t.name })),
        ]}
        value={activeId ?? ""}
        onPick={(id) => {
            activeId = id === "" ? null : id;
            writePref("tum.tool.tab", id);
          }}
      />
    {/if}
    <div class="tool__stats">
      <Stat value={fmtTokens(view.total_tokens)} label="累计 tokens" />
      <Stat value={view.session_count} label="会话" />
      <Stat value={view.project_count} label="项目" />
    </div>

    <div class="tool__breakdown">
      <span class="tool__bd"><ColorSwatch color={BANDS[0].color} size={7} />输入 {fmtTokens(rangeInput)}</span>
      <span class="tool__bd"><ColorSwatch color={BANDS[1].color} size={7} />缓存 {fmtTokens(rangeCache)}</span>
      <span class="tool__bd"><ColorSwatch color={BANDS[2].color} size={7} />输出 {fmtTokens(rangeOutput)}</span>
    </div>

    <div class="tool__chart">
      <TrendLineChart days={lineDays} colors={lineColors} maxY={maxLine} tickEvery={1} {accent} />
    </div>
  {/if}
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

  .tool__stack {
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-sm);
    padding: 8px 10px;
    background: rgba(255, 255, 255, 0.03);
  }
  .tool__stack-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 6px;
  }
  .tool__stack-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--tum-text-primary);
  }
  .tool__stack-sub {
    font-size: 9px;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-muted);
  }
  /* TrendLineChart 的根容器是 .tl（flex:1），在非 flex 的 .tool__stack 里
     高度会塌陷为 0 导致图表不渲染——必须给容器本身高度，而不是里面的 svg。
     130px（原 110）：上方是「各工具横向构成」，需要看得清谁多谁少；
     下方是「单工具时间走势」，需要时间分辨率。原 110px 下 7 条堆叠带挤在
     一起分不开，130px 后上下约 1:1.4。 */
  .tool__stack :global(.tl) {
    height: 130px;
    width: 100%;
  }

  .tool__stats {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    flex: none;
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