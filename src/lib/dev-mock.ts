// 开发期 Tauri IPC 桩 —— 让真实组件在浏览器里直接跑。
//
// 为什么需要：App.svelte / HeatmapGrid / TrendPanel 等全部通过
// `invoke()` 取数，而 invoke 依赖 `window.__TAURI_INTERNALS__`，只有
// `tauri dev` 的原生窗口才注入。纯浏览器打开时整棵树会取数失败、渲染成空壳，
// 于是「布局长什么样」这件事没法在改代码之前就看清——只能靠手抄一份简化版
// 组件，而手抄的 CSS 必然和真组件漂移（这正是本文件要消灭的东西）。
//
// 启用条件（两个都要满足，缺一不可）：
//   1. import.meta.env.DEV —— 生产构建里整段被 tree-shake 掉
//   2. 页面里没有 __TAURI_INTERNALS__ —— 真 `tauri dev` 下绝不介入
//
// 换成 Tauri 原生窗口打开时条件 2 不成立，本文件不产生任何副作用。

/** 确定性伪随机：同一台机器每次刷新得到同一份数据，便于对比布局。 */
function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const DAY = 86_400_000;
const dayKey = (t: number) => {
  const d = new Date(t);
  const p = (n: number) => `${n}`.padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
};

/** 取本机账本实测的量级，让预览里的数字不是随手编的。 */
const TOOLS = [
  { id: "minimax-code", name: "MiniMax Code", base: 4.1e7, skip: 0.45 },
  { id: "zcode", name: "ZCode", base: 3.4e7, skip: 0.5 },
  { id: "hermes", name: "Hermes", base: 1.8e7, skip: 0.52 },
  { id: "deepseek-harness", name: "DeepSeek Harness", base: 3.5e6, skip: 0.55 },
  { id: "cherry-studio", name: "Cherry Studio", base: 3.3e6, skip: 0.6 },
  { id: "codex", name: "Codex", base: 1.2e6, skip: 0.7 },
  { id: "deepseek-harness-wsl", name: "DSH (WSL)", base: 8e5, skip: 0.72 },
];

/** 工具 → 归因 provider（对齐 model-provider.ts 的 TOOL_DEFAULT_PROVIDER + 前缀规则） */
const TOOL_PROVIDER: Record<string, string> = {
  "minimax-code": "minimax",
  zcode: "glm",
  hermes: "other",
  "deepseek-harness": "deepseek",
  cherry: "other",
};

/**
 * 服务端日账：source → { 量级, 单位 }。
 *
 * 两个细节必须与真机一致，否则预览会骗人：
 * 1. `source` 是 **instance_id**（`providers/volcengine.rs:463` 写的是
 *    `self.instance_id`），不是裸 kind。前端要 split("-")[0] 还原；
 * 2. `unit` 逐 provider 不同且**决定它能否进 token 日历**：火山日明细是
 *    tokens（`unit: c.unit` 直接沿用 API 返回值），OpenAI/xAI 是 USD
 *    （`UsageUnit::Usd`），Kimi 是 CNY。非 token 的行不进日历总量与色阶。
 */
const PROVIDER_DAILY: Record<string, { base: number; unit: string }> = {
  "volcengine-9-0": { base: 5.1e7, unit: "tokens" }, // 服务端日账实测量级
  "openai-1-0": { base: 3.2, unit: "usd" },        // USD：不进 token 日历
  "xai-1-0": { base: 1.4, unit: "usd" },
};

function buildData() {
  const rnd = mulberry32(0x7a3c1d);
  const today = Date.now();
  const days: string[] = [];
  for (let i = 199; i >= 0; i--) days.push(dayKey(today - i * DAY));

  // kind='tool'：model='' 的来源总量行 + 分模型行
  const toolRows: any[] = [];
  // kind='provider'：服务端日账
  const providerRows: any[] = [];
  // 工具 → 逐日总量（喂 getLocalTools）
  const toolDaily: Record<string, Record<string, number>> = {};

  const MODEL_OF: Record<string, string> = {
    minimax: "MiniMax-M3.1-Flash-Preview",
    glm: "GLM-5.3-Flash",
    // ark- 前缀 → model-provider.ts 归到 volcengine 层，与火山服务端日账
    // 同层，合并时才会走"服务端替换本地"而不是两个独立层并存。
    volcengine: "ark-seed-1.6",
    deepseek: "deepseek-v4-flash",
    other: "未标记模型",
  };

  for (const d of days) {
    const dow = new Date(d + "T00:00:00").getDay();
    const weekend = dow === 0 || dow === 6 ? 0.28 : 1;
    const bySource: Record<string, number> = {};
    const byModel: Record<string, Record<string, number>> = {};

    for (const t of TOOLS) {
      if (rnd() < t.skip) continue;
      // 重尾分布：多数日子小、少数日子极大。这才是真实用量形态——实测本机账本
      // 近 200 天最大 967M / 中位 4M（240 倍跨度）。早先这里用均匀的
      // (0.35 + rnd()*1.5)，分布太好看，反而掩盖了色阶线性分档会失效的问题。
      const v = t.base * weekend * (0.02 + Math.pow(rnd(), 3) * 3.5);
      bySource[t.id] = (bySource[t.id] ?? 0) + v;
      let prov = TOOL_PROVIDER[t.id] ?? "other";
      // zcode 同时打 GLM 与火山方舟：实测本机 GLM 全部来自 zcode，而火山
      // 服务端日账有 20 天、本机侧几乎为空——这里让两者部分重叠，才能在
      // 预览里看出"服务端替换本机"与"服务端补出本机没有的日子"两种形态。
      if (t.id === "zcode") prov = rnd() < 0.22 ? "volcengine" : "glm";
      const m = MODEL_OF[prov];
      (byModel[t.id] ??= {})[m] = ((byModel[t.id] ??= {})[m] ?? 0) + v;
    }
    toolDaily[d] = bySource;

    for (const [source, total] of Object.entries(bySource)) {
      // 来源总量行
      toolRows.push({
        source, kind: "tool", date: d, model: "",
        input: total * 0.02, cache_read: total * 0.78, output: total * 0.2,
        total, unit: "tokens", cost: null, currency: null, cost_estimated: false,
      });
      // 分模型行
      for (const [model, mt] of Object.entries(byModel[source] ?? {})) {
        toolRows.push({
          source, kind: "tool", date: d, model,
          input: mt * 0.02, cache_read: mt * 0.78, output: mt * 0.2,
          total: mt, unit: "tokens", cost: null, currency: null, cost_estimated: false,
        });
      }
    }
  }

  // 服务端日账：只覆盖近 60 天，且**故意包含本机当天完全没有工具行**的日子。
  // 合并逻辑若只遍历本机拆解的结果，这类日子会整段从日历里消失——桩必须能
  // 把这个洞暴露出来，否则预览看到的是"补洞成功"而非"根本没这问题"。
  for (const d of days.slice(-60)) {
    for (const [source, { base, unit }] of Object.entries(PROVIDER_DAILY)) {
      if (rnd() < 0.35) continue;
      const v = base * (0.3 + rnd() * 1.4);
      providerRows.push({
        source, kind: "provider", date: d, model: "",
        input: 0, cache_read: 0, output: 0,
        total: v, unit,
        cost: unit === "usd" ? v : null,
        currency: unit === "usd" ? "USD" : null,
        cost_estimated: false,
      });
    }
  }
  return { days, rows: [...toolRows, ...providerRows], toolDaily };
}

const DATA = buildData();
const ACCOUNTS = [
  { instance_id: "minimax", provider_kind: "minimax", label: "MiniMax Token Plan", accent_color: "#e9477e", enabled: true },
  // instance_id 必须与账本里服务端行的 source 完全一致（真机上 `id()` 就是
  // instance_id，账本 source 写的是同一个值）。早先这里写裸 "volcengine"，
  // 于是 DetailCard 按 provider_id 过滤服务端日账时一条都匹配不到，对着
  // 日历里明明有火山数据的局面显示"该账户尚无服务端日账数据"——桩自己打自己。
  { instance_id: "volcengine-9-0", provider_kind: "volcengine", label: "Volcano Agent Plan", accent_color: "#00dcff", enabled: true },
  { instance_id: "deepseek", provider_kind: "deepseek", label: "DeepSeek API", accent_color: "#3b82f6", enabled: true },
  // 覆盖用例：差分类 Provider（只有额度百分比/余额差分，无按日账本）。
  // 真实账户里目前没有这一类，但没有它就永远看不到 DetailCard 的
  // 「该 Provider 无按日用量数据」空态分支，也点不出那条静默路径。
  { instance_id: "kimi", provider_kind: "kimi", label: "Kimi", accent_color: "#1783ff", enabled: true },
];

const now = Date.now();
const iso = (offsetH: number) => new Date(now + offsetH * 3600_000).toISOString();

function snapshot() {
  interface W {
    h5: number; h5q: number; wk: number; wkq: number;
    mo: number; moq: number; unit: "tokens" | "afp" | "cny" | "credits" | "usd";
  }
  const mk = (id: string, name: string, tier: string | null, w: W, daily?: { used: number; quota: number }) => ({
    provider_id: id,
    provider_display_name: name,
    ...(tier ? { plan_tier: tier } : {}),
    timestamp: iso(0),
    windows: {
      // daily 窗口：只有 AgentPlan 类（AFP 日额度）才有。CalendarSection 靠
      // windows.daily 判定「该账户有服务端日账」，缺了它口径行就不出现——
      // 桩数据必须与真机结构一致，否则预览会骗人。
      ...(daily
        ? {
            daily: {
              used: daily.used, quota: daily.quota, unit: "afp" as const,
              reset_at: iso(20), over_quota: false, cost_source: "provider_reported" as const,
            },
          }
        : {}),
      five_hour: {
        used: w.h5, quota: w.h5q, unit: w.unit, reset_at: iso(2),
        over_quota: false, cost_source: "provider_reported",
      },
      weekly: {
        used: w.wk, quota: w.wkq, unit: w.unit, reset_at: iso(60),
        over_quota: false, cost_source: "provider_reported",
      },
      monthly: {
        used: w.mo, quota: w.moq, unit: w.unit, reset_at: iso(300),
        over_quota: false, cost_source: "provider_reported",
      },
    },
  });
  return [
    mk("minimax", "MiniMax Token Plan", "Max", { h5: 3.4e7, h5q: 5e7, wk: 1.8e8, wkq: 2.5e8, mo: 6.2e8, moq: 8.6e8, unit: "tokens" }),
    mk("deepseek", "DeepSeek API", null, { h5: 1.1e7, h5q: 5e7, wk: 6.4e7, wkq: 2.5e8, mo: 2.1e8, moq: 8.6e8, unit: "tokens" }),
    // provider_id = instance_id，与 ACCOUNTS / 账本 source 三处同源（见上）。
    mk("volcengine-9-0", "Volcano Agent Plan", "Agent", { h5: 8.6e6, h5q: 1e7, wk: 5.2e7, wkq: 6e7, mo: 1.8e8, moq: 2e8, unit: "afp" }, { used: 88, quota: 200 }),
    // 差分类：只有额度百分比/金额，无按日账本 → DetailCard 应显示无按日数据说明。
    // CNY 用真实量级（几十~几百元），别用 token 量级否则会显示成 ¥2200000.00。
    mk("kimi", "Kimi", null, { h5: 22, h5q: 50, wk: 94, wkq: 250, mo: 310, moq: 860, unit: "cny" }),
  ];
}

const USAGE = snapshot();
const STATES = Object.fromEntries(
  USAGE.map((s) => [s.provider_id, { snapshot: s, last_error: null, last_updated_at: iso(0) }]),
);

const SETTINGS = {
  enabled_providers: [],
  accounts: ACCOUNTS,
  tool_data_roots: ["D:\\Lab\\.agentdata"],
  tool_data_dirs: {},
  poll_interval_seconds: 30,
  dashboard_x: 100,
  dashboard_y: 100,
  compact_mode: false,
  autostart: true,
  close_to_tray: true,
  notify_enabled: true,
  notify_warn_percent: 80,
  notify_crit_percent: 95,
  ring_window: "auto",
  edge_snap: true,
  countdown_mode: true,
  display_currency: "auto",
  hub_mode: "off",
  hub_port: 43210,
  hub_base: "",
  report_on: false,
  hub_token: "",
  hub_token_configured: true,
  rate_overrides: {},
  proxy_url: null,
};

const RATES = {
  base: "USD",
  rates: { USD: 1, CNY: 7.24, TWD: 32.1, HKD: 7.81, JPY: 148.2, EUR: 0.92, GBP: 0.79 },
  updated_at: iso(-2),
  stale: false,
  source: "cache",
};

function localTools() {
  const byTool: Record<string, { name: string; total: number; days: string[]; byModel: Record<string, number> }> = {};
  for (const t of TOOLS) byTool[t.id] = { name: t.name, total: 0, days: [], byModel: {} };
  for (const d of DATA.days) {
    for (const [id, v] of Object.entries(DATA.toolDaily[d] ?? {})) {
      const e = byTool[id];
      if (!e) continue;
      e.total += v;
      e.days.push(d);
      const prov = TOOL_PROVIDER[id] ?? "other";
      e.byModel[prov] = (e.byModel[prov] ?? 0) + v;
    }
  }
  const tools = Object.entries(byTool)
    .filter(([, e]) => e.total > 0)
    .map(([id, e]) => {
      const byDay: Record<string, number> = {};
      for (const d of DATA.days) if (DATA.toolDaily[d]?.[id]) byDay[d] = DATA.toolDaily[d][id];
      const models = Object.entries(e.byModel).map(([m, total]) => ({
        model: m, total_tokens: total, cost: 0, currency: "", cost_estimated: false,
        daily: Object.entries(byDay).map(([date, value]) => ({
          date, input: value * 0.02, cache_read: value * 0.78, output: value * 0.2, total: value,
        })),
      }));
      return {
        id, name: e.name, daily: Object.entries(byDay).map(([date, value]) => ({
          date, input: value * 0.02, cache_read: value * 0.78, output: value * 0.2, total: value,
        })),
        total_tokens: e.total, session_count: 120, project_count: 6,
        scanned_at: iso(0), models,
      };
    });
  return { tools, sessions_parsed: 480 };
}

const MACHINE = { device_id: "preview", hostname: "PREVIEW", os: "windows", arch: "x64", version: "0.1.0", pid: 0 };

/** TokenRouter 状态桩：一条演示路由（链头低配额 → 徽标可见），便于预览路由 pane。 */
const ROUTER_STATUS = {
  enabled: true,
  health: { listening: true, port: 43211 },
  routes: [
    {
      id: "route-preview-1",
      name: "Claude Code 主力",
      protocol: "anthropic",
      active_index: 1,
      candidates: [
        {
          account: "anthropic-1",
          model: "claude-sonnet-4-5",
          base_url: "https://api.anthropic.com",
          state: "low_quota",
          remaining_percent: 12,
        },
        {
          account: "minimax-1",
          model: "MiniMax-M2",
          base_url: "https://api.minimaxi.com",
          state: "ok",
          remaining_percent: 86,
        },
      ],
    },
  ],
};

/** @param cmd Tauri command 名（snake_case）或插件命令（plugin:event|listen） */
function mockInvoke(cmd: string, args: any = {}): any {
  switch (cmd) {
    case "get_usage": return USAGE;
    case "get_provider_states": return STATES;
    case "get_settings": return SETTINGS;
    case "get_providers":
      return {
        presets: [],
        accounts: ACCOUNTS.map((a) => ({ ...a, has_credentials: true, live: true })),
      };
    case "get_usage_history": {
      const days = Math.max(1, Math.min(args.days ?? 90, DATA.rows.length));
      const cutoff = new Date(now - days * DAY).toISOString().slice(0, 10);
      return { rows: DATA.rows.filter((r) => r.date >= cutoff) };
    }
    case "get_local_tools": return localTools();
    case "get_hub_devices": return { devices: [], warning: null };
    case "get_device_report": return MACHINE;
    case "get_exchange_rates": return RATES;
    case "refresh_exchange_rates": return RATES;
    case "get_router_status": return ROUTER_STATUS;
    case "fetch_upstream_models":
      return ["claude-sonnet-4-5", "claude-haiku-4-5", "claude-opus-4-1", "MiniMax-M2"];
    // 账户日账视图：真机走 daily_snapshots（有数据），桩里直接由 provider 行派生，
    // 否则切到「账户日账」口径会永远空态，预览与真机不一致。
    case "get_heatmap": {
      const id = args.providerId;
      const days = Math.max(1, Math.min(args.days ?? 31, 400));
      const cutoff = new Date(now - days * DAY).toISOString().slice(0, 10);
      return DATA.rows
        .filter((r) => r.kind === "provider" && r.source === id && r.date >= cutoff)
        .map((r) => ({ date: r.date, value: r.total, unit: r.unit }));
    }

    // 窗口/贴边命令：真机走 Rust 的 nearest_side，桩只需回一个合法边让预览不报错。
    case "sync_peek_window": return "right";
    case "dock_side_of": {
      // 简化桩：只按四边距离取最近，不做角落的拖拽方向裁决（真机那部分在
      // dock.rs 的 nearest_side），预览够用。
      const near = 40;
      const W = 1920, H = 1040;
      const cands: Array<[string, number]> = [
        ["left", args.x],
        ["right", W - (args.x + args.w)],
        ["top", args.y],
        ["bottom", H - (args.y + args.h)],
      ];
      const min = Math.min(...cands.map(([, d]) => d));
      return min <= near ? cands.find(([, d]) => d === min)![0] : null;
    }

    // 事件系统：listen 返回 id，unlisten 空实现。桩不发任何事件。
    case "plugin:event|listen": return nextEventId();
    case "plugin:event|unlisten": return null;

    default:
      // 用 import.meta.env 而非 process.env：本项目没装 @types/node，
      // 后者在 tsconfig 下直接是类型错误。
      if (import.meta.env.DEV) console.debug("[dev-mock] 未覆盖的命令：", cmd, args);
      return null;
  }
}

let eventId = 0;
const nextEventId = () => ++eventId;

export function installDevMock(): void {
  const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  if (!import.meta.env.DEV || hasTauri) return;
  (window as any).__TAURI_INTERNALS__ = {
    invoke: mockInvoke,
    transformCallback: (cb: any) => {
      const id = ++eventId;
      (window as any)[`_cb_${id}`] = cb;
      return id;
    },
    metadata: { currentWindow: { label: "dashboard" }, currentWebview: { label: "dashboard" } },
    plugins: {},
  };
  console.info(
    "%c[dev-mock]%c Tauri IPC 已桩化，真实组件将在浏览器中运行。",
    "background:#4cc2ff;color:#1a1b1e;font-weight:700;border-radius:3px",
    "color:#8b949e",
  );
}
