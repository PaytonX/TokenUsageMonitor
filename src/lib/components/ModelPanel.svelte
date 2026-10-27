<script lang="ts">
  // 模型用量面板（B5 模型视图 / 本地工具按模型聚合）：跨工具把相同模型归并，
  // 展示各模型的 token 总量与成本；点选模型查看其逐日走势（线图）。
  // 数据复用 get_local_tools 的 models 字段（claude/cherry/minimax 已按模型聚合）。
  import { getHubDevices, getUsageHistory, onTabsChanged, onToolsUpdated } from "../api";
  import { readAggMode, readPref, writePref } from "../prefs";
  import { buildAggregateModels, staleDetailDevices } from "../device-agg";
  import { collectRouterRowsByDate, routedAttribution, ROUTED_MODEL, ROUTED_MODEL_DISPLAY } from "../router-attribution";
  import { canonicalModelKey, dominantDisplayName, isPlaceholderModelName, stripModelPrefix } from "../model-name";
  import type { LocalModelUsage, LocalToolsPayload } from "../types";
  import { displayCurrency, formatCost, normalizeCurrency, toUsd } from "../currency";
  import type { Currency } from "../currency";
  import { t, locale } from "../i18n/store";
  import TrendLineChart from "./TrendLineChart.svelte";
  import {
    PanelHeader,
    RangePills,
    ColorSwatch,
    Stat,
    Icon,
  } from "./atoms";

  interface Props {
    accent?: string;
  }
  let { accent = "#4cc2ff" }: Props = $props();

  type RangeKey = "7d" | "30d" | "90d";
  // label 存 i18n 键（与 trend-data.ts RANGES 同约定），渲染处经 $t 取词。
  const RANGES: { key: RangeKey; label: string; days: number }[] = [
    { key: "7d", label: "model.range7d", days: 7 },
    { key: "30d", label: "model.range30d", days: 30 },
    { key: "90d", label: "model.range90d", days: 90 },
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
  // 全端汇总模式：合并各设备上报的分模型序列（成本不同步，汇总态隐藏成本）。
  let aggMode = $state(readAggMode());
  let staleDevs = $state<string[]>([]);

  async function load(force = false) {
    loading = true;
    error = null;
    try {
      if (aggMode) {
        const r = await getHubDevices();
        const models = buildAggregateModels(r.devices);
        staleDevs = staleDetailDevices(r.devices);
        // 合成单一"全端"工具承载合并模型列表，复用既有渲染路径。
        const dayMap = new Map<string, { date: string; input: number; cache_read: number; output: number; total: number }>();
        for (const m of models) {
          for (const d of m.daily) {
            const cur = dayMap.get(d.date) ?? { date: d.date, input: 0, cache_read: 0, output: 0, total: 0 };
            cur.total += d.total;
            dayMap.set(d.date, cur);
          }
        }
        const daily = [...dayMap.values()].sort((a, b) => a.date.localeCompare(b.date));
        payload = {
          tools: [{
            id: "__aggregate__",
            name: "全端",
            daily,
            total_tokens: models.reduce((s, m) => s + m.total_tokens, 0),
            session_count: 0,
            project_count: 0,
            scanned_at: new Date().toISOString(),
            models,
          }],
          sessions_parsed: 0,
        };
      } else {
        // 本机模式：数据源 = 统一账本（与日历/趋势严格同源）。把账本的分模型
        // 行合成单一"全部工具"载体，复用既有渲染路径。
        staleDevs = [];
        const { rows } = await getUsageHistory(90);
        const byModel = new Map<string, {
          total: number;
          cost: number;
          currencies: Set<string>;
          cost_estimated: boolean;
          days: Map<string, { date: string; input: number; cache_read: number; output: number; total: number }>;
        }>();
        const dayMap = new Map<string, { date: string; input: number; cache_read: number; output: number; total: number }>();
        // 经路由的哨兵行先攒着（含分列），遍历完按路由台账替换成真实模型名。
        const routed = new Map<string, { input: number; cache_read: number; output: number; total: number }>();
        const routerRows = collectRouterRowsByDate(rows);
        const bumpRouted = (r: (typeof rows)[number]) => {
          const cur = routed.get(r.date) ?? { input: 0, cache_read: 0, output: 0, total: 0 };
          cur.input += r.input; cur.cache_read += r.cache_read;
          cur.output += r.output; cur.total += r.total;
          routed.set(r.date, cur);
        };
        for (const r of rows) {
          if (r.kind !== "tool") continue;
          if (r.model === "") {
            const cur = dayMap.get(r.date) ?? { date: r.date, input: 0, cache_read: 0, output: 0, total: 0 };
            cur.input += r.input; cur.cache_read += r.cache_read;
            cur.output += r.output; cur.total += r.total;
            dayMap.set(r.date, cur);
            continue;
          }
          if (r.model === ROUTED_MODEL) {
            bumpRouted(r);
            continue;
          }
          let m = byModel.get(r.model);
          if (!m) {
            m = { total: 0, cost: 0, currencies: new Set(), cost_estimated: false, days: new Map() };
            byModel.set(r.model, m);
          }
          m.total += r.total;
          if ((r.cost ?? 0) > 0) {
            m.cost += r.cost!;
            if (r.currency) m.currencies.add(r.currency);
          }
          if (r.cost_estimated) m.cost_estimated = true;
          const d = m.days.get(r.date) ?? { date: r.date, input: 0, cache_read: 0, output: 0, total: 0 };
          d.input += r.input; d.cache_read += r.cache_read;
          d.output += r.output; d.total += r.total;
          m.days.set(r.date, d);
        }
        // 替换归因：路由台账只给**总量**，而模型页按 input/cache/output 分列，
        // 故按各切片占比把哨兵的分列拆回去。台账覆盖不足时，剩余部分保留在
        // 哨兵名下——把无法确认的量塞给某个真实模型比诚实单列更糟。
        const ensure = (name: string) => {
          let m = byModel.get(name);
          if (!m) {
            m = { total: 0, cost: 0, currencies: new Set(), cost_estimated: false, days: new Map() };
            byModel.set(name, m);
          }
          return m;
        };
        for (const [date, agg] of routed) {
          const { slices, fallback } = routedAttribution(agg.total, routerRows.get(date) ?? []);
          for (const s of slices) {
            const m = ensure(s.model);
            const ratio = agg.total > 0 ? s.total / agg.total : 0;
            const d = m.days.get(date) ?? { date, input: 0, cache_read: 0, output: 0, total: 0 };
            d.input += agg.input * ratio; d.cache_read += agg.cache_read * ratio;
            d.output += agg.output * ratio; d.total += s.total;
            m.days.set(date, d);
            m.total += s.total;
          }
          if (fallback > 0) {
            const ratio = agg.total > 0 ? fallback / agg.total : 0;
            const m = ensure(ROUTED_MODEL);
            const d = m.days.get(date) ?? { date, input: 0, cache_read: 0, output: 0, total: 0 };
            d.input += agg.input * ratio; d.cache_read += agg.cache_read * ratio;
            d.output += agg.output * ratio; d.total += fallback;
            m.days.set(date, d);
            m.total += fallback;
          }
        }
        const models: LocalModelUsage[] = [...byModel.entries()].map(([model, m]) => ({
          model,
          total_tokens: m.total,
          cost: m.cost,
          currency: m.currencies.size === 1 ? [...m.currencies][0] : m.currencies.size > 1 ? "USD" : "",
          cost_estimated: m.cost_estimated,
          // 模型页按模型名合桶，账户已在这一层丢失；空串表示未记。
          provider: "",
          daily: [...m.days.values()].sort((a, b) => a.date.localeCompare(b.date)),
        })).sort((a, b) => b.total_tokens - a.total_tokens);
        const daily = [...dayMap.values()].sort((a, b) => a.date.localeCompare(b.date));
        payload = {
          tools: [{
            id: "__ledger__",
            name: "全部工具",
            daily,
            total_tokens: daily.reduce((s, d) => s + d.total, 0),
            session_count: 0,
            project_count: 0,
            scanned_at: new Date().toISOString(),
            models,
          }],
          sessions_parsed: 0,
        };
      }
      if (activeId === null || !modelList.some((m) => m.id === activeId)) {
        // 默认选中排名第一的真模型；只有全是占位桶时才退回占位桶。
        activeId = (rankedModels[0] ?? modelList[0])?.id ?? null;
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

  // 归一化模型名：合并 claude("MiniMax-M3") / cherry("MiniMax-M3") /
  // minimax("minimax/MiniMax-M3") 等写法为短名。
  function shortName(raw: string): string {
    // 哨兵是内部标识，展示前换成可读名（仅台账覆盖不足时会走到这里）。
    if (raw === ROUTED_MODEL) return ROUTED_MODEL_DISPLAY;
    return stripModelPrefix(raw);
  }

  // 跨工具归并同一模型：汇总 total/cost，并合并逐日序列。
  // 成本口径：各工具上报币种不同（cherry=CNY，claude/codex/minimax/hermes=USD），
  // 必须先按静态汇率折算成 USD 再累加——直接把不同币种的原始数值相加是错的。
  // 折算只发生在"合并"这一步；展示仍按用户所选展示币种（见 fmtCost）。
  let modelList = $derived.by(() => {
    const byName = new Map<string, { id: string; name: string; total: number; costUsd: number; currencies: Set<Currency>; cost_estimated: boolean; placeholder: boolean; daily: Map<string, LocalModelUsage["daily"][number]> }>();
    // 归并键（小写）→ 该键下各写法的累计量，用于挑展示名。同一模型被不同
    // 工具用不同大小写记录（zcode 的 GLM-5.3-Flash vs 路由台账的
    // glm-5.3-flash）时，不归并就会在模型页裂成两条。
    const variants = new Map<string, Map<string, number>>();
    const note = (key: string, display: string, total: number) => {
      let v = variants.get(key);
      if (!v) {
        v = new Map();
        variants.set(key, v);
      }
      v.set(display, (v.get(display) ?? 0) + total);
    };
    for (const tool of payload?.tools ?? []) {
      for (const mu of tool.models) {
        const short = shortName(mu.model);
        const key = canonicalModelKey(short);
        const placeholder = isPlaceholderModelName(mu.model);
        note(key, short, mu.total_tokens);
        let agg = byName.get(key);
        if (!agg) {
          agg = { id: key, name: short, total: 0, costUsd: 0, currencies: new Set(), cost_estimated: false, placeholder, daily: new Map() };
          byName.set(key, agg);
        }
        agg.placeholder = agg.placeholder || placeholder;
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
    // 展示名取该归并键下用量最大的写法，与归并键解耦。
    for (const [key, agg] of byName) {
      const v = variants.get(key);
      if (v && v.size > 0) agg.name = dominantDisplayName(v);
    }
    return [...byName.values()]
      .sort((a, b) => b.total - a.total)
      .map((m) => ({ ...m, daily: [...m.daily.values()].sort((a, b) => a.date.localeCompare(b.date)) }));
  });

  // 路由占位桶（`auto` / `__via_router__`）不是真实模型：账本里实测 `auto`
  // 近 30 天就有 1.23 亿 token，混进排名会挤掉真模型的位次并让圆环份额失真。
  // 因此排名（圆环/图例/其他成员）只认真模型，占位桶单列成一行说明——既
  // 不静默丢掉这部分量，也不让它参与排名。
  let rankedModels = $derived(modelList.filter((m) => !m.placeholder));
  let placeholderModels = $derived(modelList.filter((m) => m.placeholder));

  let active = $derived(modelList.find((m) => m.id === activeId) ?? null);

  // 圆环里拿到独立切片的模型；其余小份额模型并进「其他」，但仍要能以独立
  // 按钮点开看走势（见下方 others 与浮层列表）。
  let mergedModels = $derived(
    rankedModels.filter((m) => !ringSlices.some((s) => !s.isOther && s.id === m.id)),
  );

  // 「其他」里的成员 = 小份额真模型 + 路由占位桶。它们共用同一个出口：
  // 点圆环的灰色切片（或图例里的「其他」）弹出列表，选中即切换下方折线。
  // 这样既不给小份额模型占一行标签墙（上一版正是因此把折线图挤没了），
  // 也不让它们变成"看不见 = 没记录"。
  let others = $derived([...mergedModels, ...placeholderModels]);
  let otherPopOpen = $state(false);

  // —— 模型占比圆环（替代原 PillsOrSelect 选择行）：手写 SVG 环图。
  // 占比 <2% 的模型一律归入灰色「其他」（用户规则：用量过少即归并，与模型
  // 数量无关），其余按最多 5 个切片直显。「其他」不可选，悬停提示成员明细。
  // 防御：若模型极多导致所有切片都 <2%，退回展示前 3 名，环不留空。
  const RING_MAX_SLICES = 5;
  const RING_MIN_SHARE = 0.02;
  const RING_COLORS = ["#5fd4a2", "#f2b35b", "#f27b9b", "#7b93f2", "#4cc2ff"];
  const RING_OTHER_COLOR = "rgba(140, 148, 163, 0.8)";

  interface RingSlice {
    id: string;
    name: string;
    total: number;
    share: number;
    color: string;
    isOther: boolean;
    members: string[];
  }
  interface RingArc extends RingSlice {
    dasharray: string;
    dashoffset: number;
  }

  let hoverSliceId = $state<string | null>(null);

  let ringSlices = $derived.by(() => {
    // 份额分母只含真模型：占位桶不参与排名，也不该稀释真模型的百分比。
    const grand = rankedModels.reduce((sum, m) => sum + m.total, 0);
    if (grand <= 0) return [] as RingSlice[];
    const shareOf = (m: { total: number }) => m.total / grand;
    const head = rankedModels.filter(
      (m, i) => i < RING_MAX_SLICES && shareOf(m) >= RING_MIN_SHARE,
    );
    // 极端情况：模型多到全部 <2% → 展示前 3 名，保证环与图例不为空。
    const effective = head.length > 0 ? head : rankedModels.slice(0, 3);
    const picked: RingSlice[] = effective.map((m, i) => ({
      id: m.id,
      name: m.name,
      total: m.total,
      share: shareOf(m),
      color: RING_COLORS[i % RING_COLORS.length],
      isOther: false,
      members: [],
    }));
    const restTotal = grand - effective.reduce((sum, m) => sum + m.total, 0);
    if (restTotal > 0) {
      picked.push({
        id: "__other__",
        name: $t("mp.other"),
        total: restTotal,
        share: restTotal / grand,
        color: RING_OTHER_COLOR,
        isOther: true,
        members: rankedModels
          .filter((m) => !effective.includes(m))
          .map((m) => `${m.name} · ${fmtTokens(m.total)}`),
      });
    }
    return picked;
  });

  let ringArcs = $derived.by(() => {
    const C = 2 * Math.PI * 42;
    let cum = 0;
    return ringSlices.map((s) => {
      const arc = Math.max(1, s.share * C - 2);
      const out: RingArc = { ...s, dasharray: `${arc} ${C - arc}`, dashoffset: -cum };
      cum += s.share * C;
      return out;
    });
  });

  let centerSlice = $derived(
    ringSlices.find((s) => s.id === hoverSliceId) ?? ringSlices[0] ?? null,
  );

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
  // 成本标签：让 "≈" 有解释（估算 / 跨币种折算）。返回 i18n 键，由模板 $t 取词。
  function costLabel(m: { currencies: Set<Currency>; cost_estimated: boolean }): string {
    if (m.cost_estimated) return "model.costEstimated";
    if (m.currencies.size > 1) return "model.costConverted";
    return "common.cost";
  }
</script>

<div class="mp">
  <PanelHeader title={aggMode ? $t("model.titleAllDevices") : $t("model.title")}>
    <RangePills
      options={RANGES.map((r) => ({ key: r.key, label: $t(r.label) }))}
      value={range}
      accent={accent}
      onChange={(k) => { range = k; writePref("tum.model.range", k); }}
    />
    <button
      type="button"
      class="mp__refresh"
      title={aggMode ? $t("model.refreshAllDevices") : $t("model.rescanLocalLogs")}
      onclick={() => void load(true)}
    ><Icon name="refresh" size={13} /></button>
  </PanelHeader>

  {#if aggMode && staleDevs.length > 0 && (modelList.length === 0)}
    <div class="mp__empty mp__empty--err">
      {$t("model.staleDevices", { names: staleDevs.join($locale === "en" ? ", " : "、") })}
    </div>
  {/if}

  {#if loading && !payload}
    <div class="mp__empty">{$t("model.scanning")}</div>
  {:else if error && !payload}
    <div class="mp__empty mp__empty--err">{$t("model.scanFailed", { error })}</div>
  {:else if modelList.length === 0}
    <div class="mp__empty">{$t("model.noModelUsage")}</div>
  {:else}
    {#if rankedModels.length > 0}
      <div class="mp__ring">
        <div class="ring">
          <svg viewBox="0 0 110 110" role="img" aria-label={$t("model.ringAria")}>
            <g class="ring__dial">
              <circle class="ring__track" cx="55" cy="55" r="42" />
              {#each ringArcs as a (a.id)}
                <circle
                  class="ring__seg"
                  class:ring__seg--active={hoverSliceId === a.id || (!hoverSliceId && a.id === ringSlices[0]?.id)}
                  style="stroke: {a.color};"
                  cx="55"
                  cy="55"
                  r="42"
                  stroke-dasharray={a.dasharray}
                  stroke-dashoffset={a.dashoffset}
                  onpointerenter={() => { hoverSliceId = a.id; }}
                  onpointerleave={() => { if (hoverSliceId === a.id) hoverSliceId = null; }}
                  onpointerdown={(e) => e.stopPropagation()}
                  onclick={() => {
                    if (a.isOther) { otherPopOpen = !otherPopOpen; return; }
                    activeId = a.id;
                    writePref("tum.model.tab", a.id);
                  }}
                  onkeydown={(e) => {
                    if (e.key !== "Enter" && e.key !== " ") return;
                    e.preventDefault();
                    if (a.isOther) { otherPopOpen = !otherPopOpen; return; }
                    activeId = a.id;
                    writePref("tum.model.tab", a.id);
                  }}
                  role="button"
                  tabindex={0}
                  aria-expanded={a.isOther ? otherPopOpen : undefined}
                >
                  <title>{a.isOther
                    ? $t("model.otherTooltip", { members: a.members.join($locale === "en" ? ", " : "、") })
                    : $t("model.sliceTooltip", { name: a.name, tokens: fmtTokens(a.total), pct: (a.share * 100).toFixed(1) })}</title>
                </circle>
              {/each}
            </g>
          </svg>
          {#if centerSlice}
            <div class="ring__center">
              <span class="ring__center-num">{(centerSlice.share * 100).toFixed(centerSlice.share * 100 >= 10 ? 0 : 1)}%</span>
              <span class="ring__center-label">{centerSlice.isOther ? $t("mp.other") : centerSlice.name}</span>
            </div>
          {/if}
        </div>
        <div class="ring-legend">
          {#each ringSlices as s (s.id)}
            <button
              type="button"
              class="ring-legend__item"
              class:ring-legend__item--active={s.isOther ? otherPopOpen : activeId === s.id}
              class:ring-legend__item--other={s.isOther}
              aria-expanded={s.isOther ? otherPopOpen : undefined}
              title={s.isOther
                ? $t("model.otherTooltip", { members: s.members.join($locale === "en" ? ", " : "、") })
                : $t("model.sliceTitle", { name: s.name, tokens: fmtTokens(s.total) })}
              onpointerenter={() => { hoverSliceId = s.id; }}
              onpointerleave={() => { if (hoverSliceId === s.id) hoverSliceId = null; }}
              onclick={() => {
                if (s.isOther) { otherPopOpen = !otherPopOpen; return; }
                activeId = s.id;
                writePref("tum.model.tab", s.id);
              }}
            >
              <span class="ring-legend__dot" style="background: {s.color};"></span>
              <span class="ring-legend__name">{s.name}</span>
              <span class="ring-legend__pct">{(s.share * 100).toFixed(s.share * 100 >= 10 ? 0 : 1)}%</span>
            </button>
          {/each}
        </div>

        <!-- 「其他」浮层：小份额真模型 + 路由占位桶的唯一出口。
             上一版把这两类做成了两行常驻标签墙，flex-wrap 折成 2~3 行固定占高，
             把下方折线图压到十几像素——这次改成点击才出现的浮层，不占布局高度。 -->
        {#if otherPopOpen && others.length > 0}
          <div
            class="mp__other-pop"
            role="dialog"
            aria-label={$t("model.otherPopTitle")}
          >
            <span class="mp__other-pop-title">{$t("model.otherPopTitle")}</span>
            {#each others as m (m.id)}
              <button
                type="button"
                class="mp__other-item"
                class:mp__other-item--active={activeId === m.id}
                title={$t("model.sliceTitle", { name: m.name, tokens: fmtTokens(m.total) })}
                onclick={() => {
                  activeId = m.id;
                  writePref("tum.model.tab", m.id);
                  otherPopOpen = false;
                }}
              >
                <span class="mp__other-name">{m.name}</span>
                <span class="mp__other-tokens tum-numeric">{fmtTokens(m.total)}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if active}
      <div class="mp__stats">
        <Stat value={fmtTokens(rangeTotal)} label={$t("model.lastDays", { n: rangeDays })} />
        <Stat value={fmtTokens(active.total)} label={$t("model.totalTokens")} />
        {#if active.costUsd > 0}
          <Stat value={fmtCost(active)} label={$t(costLabel(active))} />
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
    font-size: var(--tum-font-size-sm);
    line-height: 1;
    cursor: pointer;
    transition: all 0.15s ease, transform var(--tum-dur-fast, 150ms) var(--tum-ease-spring);
  }
  .mp__refresh:active {
    transform: scale(0.94);
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

  .mp__chart {
    flex: 1;
    /* 折线图是这个页签的主内容：给一个下限，避免上面的圆环/统计把它压扁
       （上一版两行标签墙把 flex:1 挤到十几像素，就是这行 min-height 的由来）。 */
    min-height: 96px;
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

  /* —— 模型占比圆环 + 图例（替代原 PillsOrSelect 选择行）—— */
  .mp__ring {
    position: relative; /* 「其他」浮层的定位基准 */
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 8px;
  }
  .ring {
    position: relative;
    width: 118px;
    height: 118px;
    flex: none;
  }
  .ring svg {
    width: 100%;
    height: 100%;
    display: block;
  }
  .ring__dial {
    transform: rotate(-90deg);
    transform-origin: 55px 55px;
  }
  .ring__track {
    fill: none;
    stroke: rgba(255, 255, 255, 0.14);
    stroke-width: 13;
  }
  .ring__seg {
    fill: none;
    stroke-width: 13;
    transition: stroke-width 120ms var(--tum-ease-spring);
    cursor: pointer;
    outline: none;
  }
  .ring__seg--active {
    stroke-width: 15;
  }
  .ring__center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1px;
    pointer-events: none;
  }
  .ring__center-num {
    font-family: var(--tum-font-mono);
    font-size: 17px;
    font-weight: 600;
    color: var(--tum-text-primary);
  }
  .ring__center-label {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-secondary);
    max-width: 74px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ring-legend {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }
  .ring-legend__item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    text-align: left;
    cursor: pointer;
  }
  .ring-legend__item:hover {
    background: var(--tum-surface);
  }
  .ring-legend__item--active {
    background: var(--tum-border);
    color: var(--tum-text-primary);
  }
  .ring-legend__item--other {
    cursor: default;
    opacity: 0.75;
  }
  .ring-legend__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .ring-legend__name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ring-legend__pct {
    font-family: var(--tum-font-mono);
    color: var(--tum-text-primary);
  }

  /* —— 「其他」浮层：小份额模型与路由占位桶的唯一入口 —— */
  .mp__other-pop {
    position: absolute;
    top: calc(100% - 2px);
    left: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 190px;
    max-width: min(320px, 86vw);
    max-height: 168px;
    overflow-y: auto;
    padding: 5px;
    border-radius: var(--tum-radius-md);
    border: 1px solid var(--tum-border-strong);
    background: rgba(24, 26, 30, 0.97);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.5);
  }
  .mp__other-pop-title {
    padding: 1px 5px 3px;
    color: var(--tum-text-muted);
    font-size: 9px;
    letter-spacing: 0.8px;
    text-transform: uppercase;
  }
  .mp__other-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 2px 5px;
    border: 0;
    border-radius: var(--tum-radius-xs);
    background: transparent;
    color: var(--tum-text-secondary);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    line-height: 1.6;
    text-align: left;
    cursor: pointer;
  }
  .mp__other-item:hover {
    background: var(--tum-accent-fill);
    color: var(--tum-text-primary);
  }
  .mp__other-item--active {
    color: var(--tum-text-primary);
  }
  .mp__other-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp__other-tokens {
    flex: none;
    color: var(--tum-text-muted);
  }
</style>
