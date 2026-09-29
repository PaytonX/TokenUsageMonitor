<script lang="ts">
  // 设备页（B5 设备视图 / B8 多端同步 hub）。从本地 hub 拉取设备列表：本机恒在首位，
  // 其余为已上报到 hub 的其它实例。hub 模式由设置控制（hub/agent/off）。
  import { getHubDevices, onToolsUpdated } from "../api";
  import type { HubDevice } from "../types";
  import TrendLineChart from "./TrendLineChart.svelte";

  interface Props {
    /** 兼容旧调用：本地 Provider 数（已被每设备 provider_count 取代，保留无副作用）。 */
    providerCount?: number;
    accent?: string;
  }
  let { accent = "#4cc2ff" }: Props = $props();

  let devices = $state<HubDevice[]>([]);
  let warning = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    warning = null;
    try {
      const r = await getHubDevices();
      devices = r.devices;
      warning = r.warning;
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

  function fmtTokens(v: number): string {
    if (v >= 1_000_000) return `${(v / 1e6).toFixed(1)}M`;
    if (v >= 1_000) return `${(v / 1e3).toFixed(1)}K`;
    return `${Math.round(v)}`;
  }

  function relTime(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    if (Number.isNaN(ms)) return "";
    const min = Math.floor(ms / 60_000);
    if (min < 1) return "刚刚";
    if (min < 60) return `${min} 分钟前`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr} 小时前`;
    return `${Math.floor(hr / 24)} 天前`;
  }

  // 每台设备的逐日序列转 TrendLineChart 输入（单序列折线/面积）。
  function deviceDays(d: HubDevice) {
    const out: {
      date: string;
      label: string;
      parts: { id: string; value: number }[];
      total: number;
    }[] = [];
    for (const day of d.daily) {
      const [, m, dd] = day.date.split("-");
      out.push({
        date: day.date,
        label: `${m}/${dd}`,
        parts: day.total > 0 ? [{ id: d.device_id, value: day.total }] : [],
        total: day.total,
      });
    }
    return out;
  }
  function deviceMax(d: HubDevice): number {
    return Math.max(1, ...d.daily.map((x) => x.total));
  }

  // 依据序列长度给一个稀疏基数（图内部还会按像素自适应加稀）。
  function deviceTick(n: number): number {
    if (n > 45) return 7;
    if (n > 20) return 3;
    return 1;
  }

  // —— 全端汇总（按设备去重聚合）——
  //
  // 去重口径（两层）：
  // 1. 设备维度：同一 device_id 只计一次。get_hub_devices 虽已按 device_id
  //    去重（本机 → 远端 hub → 本地存储三路合并会撞 id），这里再防御一次；
  //    每台机器只上报自己的本地扫描数据，因此设备之间天然不重叠。
  // 2. 日期维度：单设备序列内同一日期出现多条时取末条（上游是 Map 构建，
  //    正常不会重复，防上游口径变化）。
  // 跨设备同日用量是不同机器的真实消耗，按加法合并——不是重复。
  // 注意：date 是各上报方的本地日期，跨时区组网时同一天可能有 ±1 偏差。
  const DEV_COLORS = ["#4cc2ff", "#f2b35b", "#5fd4a2", "#f27b9b", "#b58cf5", "#6fd1d1"];
  function colorForDevice(id: string): string {
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
    return DEV_COLORS[Math.abs(h) % DEV_COLORS.length];
  }

  interface AggDay {
    date: string;
    label: string;
    parts: { id: string; value: number }[];
    total: number;
  }

  let aggDays = $derived.by(() => {
    const perDate = new Map<string, number>();
    const partsByDate = new Map<string, { id: string; value: number }[]>();
    const seenDevices = new Set<string>();
    for (const d of devices) {
      if (seenDevices.has(d.device_id)) continue;
      seenDevices.add(d.device_id);
      const days = new Map<string, number>();
      for (const day of d.daily) days.set(day.date, day.total);
      for (const [date, total] of days) {
        if (total <= 0) continue;
        perDate.set(date, (perDate.get(date) ?? 0) + total);
        const parts = partsByDate.get(date) ?? [];
        parts.push({ id: d.device_id, value: total });
        partsByDate.set(date, parts);
      }
    }
    return [...perDate.keys()].sort().map<AggDay>((date) => {
      const [, m, dd] = date.split("-");
      const parts = partsByDate.get(date) ?? [];
      return {
        date,
        label: `${m}/${dd}`,
        parts,
        total: parts.reduce((s, p) => s + p.value, 0),
      };
    });
  });
  let aggTotal = $derived(aggDays.reduce((s, d) => s + d.total, 0));
  let aggMax = $derived(Math.max(1, ...aggDays.map((d) => d.total)));
  // 今日全端用量（本地时区的今天；各端序列里没有则为 0）。
  let aggToday = $derived.by(() => {
    const t = new Date();
    const key = `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, "0")}-${String(t.getDate()).padStart(2, "0")}`;
    return aggDays.find((d) => d.date === key)?.total ?? 0;
  });
  // 图例：每台设备近 90 天合计（降序），配色与堆叠图一致。
  let aggLegend = $derived.by(() => {
    const seen = new Set<string>();
    const out: { id: string; name: string; total: number; color: string }[] = [];
    for (const d of devices) {
      if (seen.has(d.device_id)) continue;
      seen.add(d.device_id);
      const days = new Map<string, number>();
      for (const day of d.daily) days.set(day.date, day.total);
      out.push({
        id: d.device_id,
        name: d.hostname,
        total: [...days.values()].reduce((s, v) => s + v, 0),
        color: colorForDevice(d.device_id),
      });
    }
    return out.sort((a, b) => b.total - a.total);
  });
</script>

<div class="dev" data-tauri-drag-region={false}>
  <div class="dev__head">
    <span class="dev__title">设备 · 多端同步</span>
    <button type="button" class="dev__refresh" title="刷新" onclick={() => void load()}>↻</button>
  </div>

  {#if loading && devices.length === 0}
    <div class="dev__empty">正在加载设备…</div>
  {:else if error && devices.length === 0}
    <div class="dev__empty dev__empty--err">加载失败：{error}</div>
  {:else if warning}
    <div class="dev__warn" title={warning}>{warning}</div>
  {/if}

  {#if devices.length === 0}
    <div class="dev__empty">暂无设备</div>
  {:else}
    <div class="dev__agg">
      <div class="dev__agg-head">
        <span class="dev__agg-title">全端汇总</span>
        <span class="dev__agg-sub">{devices.length} 台设备 · 近 90 天 · 按设备去重</span>
      </div>
      <div class="dev__stats dev__agg-stats">
        <span class="dev__stat"><b>{fmtTokens(aggTotal)}</b><span>全端合计</span></span>
        <span class="dev__stat"><b>{fmtTokens(aggToday)}</b><span>今日全端</span></span>
        <span class="dev__stat"><b>{devices.length}</b><span>设备</span></span>
      </div>
      {#if aggDays.length > 0}
        <div class="dev__chart dev__agg-chart">
          <TrendLineChart
            days={aggDays}
            colors={Object.fromEntries(aggLegend.map((l) => [l.id, l.color]))}
            maxY={aggMax}
            tickEvery={deviceTick(aggDays.length)}
            {accent}
            stacked={true}
          />
        </div>
        <div class="dev__agg-legend">
          {#each aggLegend as l (l.id)}
            <span class="dev__agg-item" title="{l.name} · {fmtTokens(l.total)} tokens">
              <span class="dev__agg-dot" style="background: {l.color};"></span>
              <span class="dev__agg-name">{l.name}</span>
              <span class="dev__agg-total tum-numeric">{fmtTokens(l.total)}</span>
            </span>
          {/each}
        </div>
      {/if}
    </div>

    <div class="dev__list">
      {#each devices as d, i (d.device_id)}
        <div class="dev__card" style="--dev-accent: {accent}">
          <div class="dev__card-top">
            <span class="dev__name">{d.hostname}</span>
            {#if i === 0}
              <span class="dev__badge">本机</span>
            {:else}
              <span class="dev__badge dev__badge--remote">{relTime(d.reported_at)}</span>
            {/if}
          </div>
          <div class="dev__meta">
            <span>{d.os} · {d.arch}</span>
            <span>v{d.version}</span>
            {#if i > 0}<span>PPID {d.device_id}</span>{/if}
          </div>
          <div class="dev__stats">
            <span class="dev__stat"><b>{fmtTokens(d.tool_tokens)}</b><span>工具 tokens</span></span>
            <span class="dev__stat"><b>{d.provider_count}</b><span>Provider</span></span>
            <span class="dev__stat"><b>{d.tool_count}</b><span>本地工具</span></span>
          </div>
          {#if d.daily.length > 0}
            <div class="dev__chart">
              <TrendLineChart
                days={deviceDays(d)}
                colors={{ [d.device_id]: accent }}
                maxY={deviceMax(d)}
                tickEvery={deviceTick(d.daily.length)}
                {accent}
                stacked={false}
              />
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <div class="dev__sync">
      <div class="dev__sync-title">多端同步（B8）</div>
      <p class="dev__sync-desc">
        在其它设备运行的 TokenUsageMonitor 开启「agent」并指向本机的 hub 地址后，会在此列出。
        本机作为 hub 时，其它设备通过 <code>POST /ingest</code> 上报用量摘要。
      </p>
    </div>
  {/if}
</div>

<style>
  /* 面板根容器可滚动：多台设备的卡片 + 汇总区会超出 680px 视口，
     此前 overflow 默认被 .shell 裁掉且无滚动条，用户看不到视口外的设备。
     min-height:0 让 flex 子项允许收缩出滚动区。 */
  .dev {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--tum-border) transparent;
    padding-right: 2px;
  }
  .dev::-webkit-scrollbar {
    width: 4px;
  }
  .dev::-webkit-scrollbar-thumb {
    background: var(--tum-border-strong, rgba(255, 255, 255, 0.12));
    border-radius: 2px;
  }

  .dev__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  .dev__title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.2px;
  }
  .dev__refresh {
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid var(--tum-border-strong, rgba(255, 255, 255, 0.12));
    color: inherit;
    border-radius: 8px;
    width: 26px;
    height: 26px;
    font-size: 14px;
    cursor: pointer;
  }
  .dev__refresh:hover {
    background: rgba(255, 255, 255, 0.12);
  }
  .dev__list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  /* —— 全端汇总 —— */
  .dev__agg {
    border: 1px solid var(--tum-border-strong, rgba(255, 255, 255, 0.12));
    background: linear-gradient(135deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.02));
    border-radius: 12px;
    padding: 12px 14px;
    margin-bottom: 10px;
  }
  .dev__agg-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .dev__agg-title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.2px;
  }
  .dev__agg-sub {
    font-size: 10px;
    font-family: var(--tum-font-mono);
    color: rgba(232, 234, 240, 0.55);
  }
  .dev__agg-stats {
    margin-top: 10px;
  }
  .dev__agg-chart {
    height: 110px;
  }
  .dev__agg-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin-top: 8px;
  }
  .dev__agg-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: rgba(232, 234, 240, 0.75);
  }
  .dev__agg-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .dev__agg-name {
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dev__agg-total {
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--tum-text-primary, #e6e8ea);
  }
  .dev__card {
    border: 1px solid var(--tum-border-strong, rgba(255, 255, 255, 0.12));
    background: linear-gradient(135deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.02));
    border-radius: 12px;
    padding: 12px 14px;
    position: relative;
    overflow: hidden;
  }
  .dev__card::before {
    content: "";
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: linear-gradient(90deg, var(--dev-accent), transparent);
    opacity: 0.7;
  }
  .dev__card-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dev__name {
    font-size: 15px;
    font-weight: 700;
  }
  .dev__badge {
    font-size: 11px;
    font-weight: 700;
    color: var(--dev-accent);
    border: 1px solid var(--dev-accent);
    border-radius: 999px;
    padding: 1px 8px;
    opacity: 0.9;
  }
  .dev__badge--remote {
    color: rgba(232, 234, 240, 0.7);
    border-color: rgba(232, 234, 240, 0.28);
    font-weight: 600;
  }
  .dev__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 12px;
    margin-top: 8px;
    font-size: 12px;
    color: rgba(232, 234, 240, 0.72);
  }
  .dev__stats {
    display: flex;
    gap: 10px;
    margin-top: 10px;
  }
  .dev__stat {
    flex: 1 1 0;
    min-width: 0;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--tum-border-strong, rgba(255, 255, 255, 0.1));
    border-radius: 10px;
    padding: 10px;
    text-align: center;
  }
  .dev__stat b {
    display: block;
    font-size: 15px;
    font-weight: 700;
  }
  .dev__stat span {
    font-size: 11px;
    color: rgba(232, 234, 240, 0.62);
  }
  .dev__chart {
    margin-top: 10px;
    height: 92px;
    display: flex;
  }
  .dev__chart :global(svg) {
    width: 100%;
    height: 100%;
  }
  .dev__sync {
    margin-top: 14px;
    border: 1px dashed var(--tum-border-strong, rgba(255, 255, 255, 0.14));
    border-radius: 12px;
    padding: 12px 14px;
  }
  .dev__sync-title {
    font-size: 13px;
    font-weight: 600;
  }
  .dev__sync-desc {
    margin: 6px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: rgba(232, 234, 240, 0.6);
  }
  .dev__sync-desc code {
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    padding: 0 4px;
  }
  .dev__empty {
    padding: 20px;
    text-align: center;
    font-size: 13px;
    color: rgba(232, 234, 240, 0.6);
  }
  .dev__empty--err {
    color: #ff7a6e;
  }
  .dev__warn {
    margin-bottom: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    background: rgba(255, 170, 60, 0.12);
    border: 1px solid rgba(255, 170, 60, 0.35);
    color: #ffc77a;
    font-size: 12px;
    line-height: 1.5;
    word-break: break-all;
  }
</style>