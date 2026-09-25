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