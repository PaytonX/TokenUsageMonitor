<script lang="ts">
  // 设备页（B5 设备视图 / B8 多端同步 hub）。从本地 hub 拉取设备列表：本机恒在首位，
  // 其余为已上报到 hub 的其它实例。hub 模式由设置控制（hub/agent/off）。
  import { addRemoteDevice, getHubDevices, onToolsUpdated, removeHubDevice } from "../api";
  import { deviceColor } from "../device-agg";
  import type { HubDevice } from "../types";
  import TrendLineChart from "./TrendLineChart.svelte";
  import { PanelHeader, Stat, ColorSwatch } from "./atoms";
  import { t } from "../i18n/store";

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
  // 移除确认：第二次点击同一条目才真正删除（与账户删除同口径）。
  let removeConfirmId = $state<string>("");
  // 手动添加远端设备（一次性拉取其 hub 列表并入本地存储）。
  let adding = $state(false);
  let addBase = $state("");
  let addBusy = $state(false);
  let addError = $state<string | null>(null);
  let addOk = $state<number | null>(null);

  async function removeDevice(id: string) {
    if (removeConfirmId !== id) {
      removeConfirmId = id;
      return;
    }
    removeConfirmId = "";
    try {
      await removeHubDevice(id);
      await load();
    } catch (e) {
      error = String(e);
    }
  }

  async function submitAdd() {
    addBusy = true;
    addError = null;
    addOk = null;
    try {
      addOk = await addRemoteDevice(addBase);
      addBase = "";
      adding = false;
      await load();
    } catch (e) {
      addError = String(e);
    } finally {
      addBusy = false;
    }
  }

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

  // 相对时间：返回 freshness.*（词典预置键）的取词键与插值参数，
  // 模板里经 $t 渲染，语言切换后随模板重算。
  function relTimeParts(iso: string): { key: string; params?: Record<string, number> } | null {
    const ms = Date.now() - new Date(iso).getTime();
    if (Number.isNaN(ms)) return null;
    const min = Math.floor(ms / 60_000);
    if (min < 1) return { key: "freshness.justNow" };
    if (min < 60) return { key: "freshness.minutesAgo", params: { n: min } };
    const hr = Math.floor(min / 60);
    if (hr < 24) return { key: "freshness.hoursAgo", params: { n: hr } };
    return { key: "freshness.daysAgo", params: { n: Math.floor(hr / 24) } };
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
        color: deviceColor(d.device_id),
      });
    }
    return out.sort((a, b) => b.total - a.total);
  });
</script>

<div class="dev" data-tauri-drag-region={false}>
  <PanelHeader title={$t("device.title")}>
    <button type="button" class="dev__refresh" title={$t("common.refresh")} onclick={() => void load()}>↻</button>
  </PanelHeader>

  {#if loading && devices.length === 0}
    <div class="dev__empty">{$t("device.loading")}</div>
  {:else if error && devices.length === 0}
    <div class="dev__empty dev__empty--err">{$t("device.loadFailed", { error })}</div>
  {:else if warning}
    <div class="dev__warn" title={warning}>{warning}</div>
  {/if}

  {#if devices.length === 0}
    <div class="dev__empty">{$t("device.noDevices")}</div>
  {:else}
    {#if devices.length > 1}
      <div class="dev__agg">
        <div class="dev__agg-head">
          <span class="dev__agg-title">{$t("device.summaryTitle")}</span>
          <span class="dev__agg-sub">{$t("device.summarySub", { n: devices.length })}</span>
        </div>
        <div class="dev__stats dev__agg-stats">
          <Stat value={fmtTokens(aggTotal)} label={$t("device.totalAll")} />
          <Stat value={fmtTokens(aggToday)} label={$t("device.todayAll")} />
          <Stat value={devices.length} label={$t("tab.devices")} />
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
                <ColorSwatch color={l.color} shape="round" />
                <span class="dev__agg-name">{l.name}</span>
                <span class="dev__agg-total tum-numeric">{fmtTokens(l.total)}</span>
              </span>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    <div class="dev__list">
      {#each devices as d, i (d.device_id)}
        <div class="dev__card" style="--dev-accent: {accent}">
          <div class="dev__card-top">
            <span class="dev__name">{d.hostname}</span>
            {#if i === 0}
              <span class="dev__badge">{$t("device.thisMachine")}</span>
            {:else}
              {@const ago = relTimeParts(d.reported_at)}
              <span class="dev__badge dev__badge--remote">{ago ? $t(ago.key, ago.params) : ""}</span>
              <button
                type="button"
                class="dev__remove"
                class:dev__remove--confirm={removeConfirmId === d.device_id}
                title={removeConfirmId === d.device_id
                  ? $t("device.removeConfirm")
                  : $t("device.remove")}
                aria-label={$t("device.removeNamed", { name: d.hostname })}
                onclick={() => void removeDevice(d.device_id)}
              >×</button>
            {/if}
          </div>
          <div class="dev__meta">
            <span>{d.os} · {d.arch}</span>
            <span>v{d.version}</span>
            {#if i > 0}<span>PPID {d.device_id}</span>{/if}
          </div>
          <div class="dev__stats">
            <Stat value={fmtTokens(d.tool_tokens)} label={$t("device.toolTokens")} />
            <Stat value={d.provider_count} label="Provider" />
            <Stat value={d.tool_count} label={$t("device.localTools")} />
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

    <div class="dev__add">
      {#if adding}
        <div class="dev__add-form">
          <input
            class="dev__add-input"
            type="text"
            placeholder={$t("device.peerHubPlaceholder")}
            aria-label={$t("device.peerHubAddress")}
            bind:value={addBase}
            onkeydown={(e) => {
              if (e.key === "Enter") void submitAdd();
            }}
          />
          <button type="button" class="btn-add-ok" disabled={addBusy || !addBase.trim()} onclick={() => void submitAdd()}>
            {addBusy ? $t("device.fetching") : $t("device.fetch")}
          </button>
          <button type="button" class="btn-add-cancel" onclick={() => { adding = false; addError = null; }}>{$t("common.cancel")}</button>
        </div>
        {#if addError}
          <div class="dev__add-err">{addError}</div>
        {/if}
        <p class="dev__add-hint">{$t("device.addHint")}</p>
      {:else}
        {#if addOk !== null}
          <span class="dev__add-ok">{$t("device.addedCount", { n: addOk })}</span>
        {/if}
        <button type="button" class="dev__add-btn" onclick={() => { addOk = null; adding = true; }}>{$t("device.addDevice")}</button>
      {/if}
    </div>

    <div class="dev__sync">
      <div class="dev__sync-title">{$t("device.syncTitle")}</div>
      <p class="dev__sync-desc">
        {$t("device.syncDesc1")}
        {$t("device.syncDesc2a")} <code>POST /ingest</code>{$t("device.syncDesc2b")}
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
    background: var(--tum-border-strong);
    border-radius: 2px;
  }

.dev__refresh {
    flex: none;
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    background: var(--tum-surface-hover);
    color: inherit;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition: background 0.15s ease;
  }
  .dev__refresh:hover {
    background: var(--tum-border-strong);
  }
  .dev__list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  /* —— 全端汇总 —— */
  .dev__agg {
    border: 1px solid var(--tum-border-strong);
    background: linear-gradient(135deg, var(--tum-surface), rgba(255, 255, 255, 0.02));
    border-radius: var(--tum-radius-md);
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
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font-mono);
    color: var(--tum-text-muted);
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
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-secondary);
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
    color: var(--tum-text-primary);
  }
  .dev__card {
    border: 1px solid var(--tum-border-strong);
    background: linear-gradient(135deg, var(--tum-surface), rgba(255, 255, 255, 0.02));
    border-radius: var(--tum-radius-md);
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
    font-size: var(--tum-font-size-sm);
    font-weight: 700;
    color: var(--dev-accent);
    border: 1px solid var(--dev-accent);
    border-radius: var(--tum-radius-pill);
    padding: 1px 8px;
    opacity: 0.9;
  }
  .dev__badge--remote {
    color: var(--tum-text-secondary);
    border-color: var(--tum-border-strong);
    font-weight: 600;
  }
  .dev__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 12px;
    margin-top: 8px;
    font-size: 12px;
    color: var(--tum-text-secondary);
  }
  .dev__stats {
    display: flex;
    gap: 10px;
    margin-top: 10px;
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
  .dev__remove {
    margin-left: auto;
    width: 18px;
    height: 18px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--tum-border-strong);
    border-radius: 50%;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-sm);
    line-height: 1;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .dev__remove:hover {
    color: var(--tum-crit);
    border-color: var(--tum-crit-stroke);
  }
  .dev__remove--confirm {
    color: #fff;
    background: var(--tum-danger);
    border-color: var(--tum-danger);
  }

  .dev__add {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .dev__add-btn {
    align-self: flex-start;
    border: 1px dashed var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    background: transparent;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    padding: 4px 10px;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .dev__add-btn:hover {
    color: var(--tum-text-primary);
    border-color: var(--tum-accent-stroke);
  }
  .dev__add-form {
    display: flex;
    gap: 6px;
  }
  .dev__add-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    background: var(--tum-surface);
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-sm);
    font-family: var(--tum-font-mono);
    padding: 5px 8px;
    outline: none;
  }
  .dev__add-input:focus {
    border-color: var(--tum-accent-stroke);
  }
  .dev__add-form button {
    flex: none;
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    background: transparent;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    padding: 4px 10px;
    cursor: pointer;
  }
  .dev__add-form .btn-add-ok {
    background: var(--tum-accent-fill);
    border-color: var(--tum-accent-stroke);
    color: var(--tum-text-primary);
  }
  .dev__add-form button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .dev__add-err {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-crit);
    word-break: break-all;
  }
  .dev__add-ok {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-ok);
  }
  .dev__add-hint {
    margin: 0;
    font-size: var(--tum-font-size-xs);
    line-height: 1.6;
    color: var(--tum-text-muted);
  }

  .dev__sync {
    margin-top: 14px;
    border: 1px dashed var(--tum-border-strong);
    border-radius: var(--tum-radius-md);
    padding: 12px 14px;
  }
  .dev__sync-title {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
  }
  .dev__sync-desc {
    margin: 6px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--tum-text-muted);
  }
  .dev__sync-desc code {
    background: var(--tum-surface-hover);
    border-radius: var(--tum-radius-xs);
    padding: 0 4px;
  }
  .dev__empty {
    padding: 20px;
    text-align: center;
    font-size: var(--tum-font-size-base);
    color: var(--tum-text-muted);
  }
  .dev__empty--err {
    color: var(--tum-crit);
  }
  .dev__warn {
    margin-bottom: 10px;
    padding: 8px 10px;
    border-radius: var(--tum-radius-sm);
    background: var(--tum-warn-fill);
    border: 1px solid var(--tum-warn-stroke);
    color: var(--tum-warn);
    font-size: 12px;
    line-height: 1.5;
    word-break: break-all;
  }
</style>