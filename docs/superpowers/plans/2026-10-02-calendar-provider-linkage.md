# 日历 ↔ Provider 卡片联动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复日历与 Provider 卡片的联动（当前点卡片后日历变空白），把联动语义从「切换数据源」升级为「在总量日历上分层高亮」，并移除 31 天紧凑视图。

**Architecture:** 新增 `src/lib/calendar-linkage.ts` 作为联动的**唯一数据源**——纯函数，负责「provider kind → 账本序列」「账本行 → 按日按 Provider 拆解」「格子 → 配色」。`App.svelte` / `HeatmapGrid.svelte` / `DetailCard.svelte` 三处全部改为调用它，消灭各写各的映射表。P2 阶段日历恒定显示全部工具总量，点卡片只改变格子配色与统计行的第二行，不再切换数据源。

**Tech Stack:** Svelte 5（runes 模式，`svelte.config.js` 已强制 `runes: true`）、TypeScript strict、Vite 5、Vitest（本次新增）、Tauri 2 / Rust（**不改动**）。

---

## 追加：信息架构重构（2026-10-02，用户评审后追加，已实施）

原计划的载体是「总览页卡片 ↔ 同页底部日历」。实机预览后用户判定这是**维度错配**：
总览回答「账户额度还剩多少」（provider 口径），趋势回答「用量烧了多少、哪天烧的」
（时间序列口径），工具口径的日历挂在额度卡片下面并不自洽。

### 已实施（commit 见 git log）

| 变更 | 说明 |
|---|---|
| 日历迁至趋势页 | 新增 `src/lib/components/CalendarSection.svelte`（药丸条 + 口径下拉 + 月历），`TrendPanel`（嵌入）与 `TrendWindow`（独立）共用 |
| 联动载体换成药丸条 | 日历离开总览后卡片↔日历在 400px 窗口无法同屏。总览卡片点击仍预置 `highlightKey`（跨页共享），切到趋势页即生效 |
| 焦点两处同步 | `trend-data.dimOthers()` 让堆叠柱与日历同时压暗，避免「日历变了柱子没变」 |
| provider 溢出降级 | 药丸条 >6 个改用 `高亮 [下拉]`，避免换行吃掉日历的垂直预算 |
| 工具页同构联动 | 新增「全部工具」态（`activeId=null` → 逐日求和的聚合视图）；选中具体工具时上方堆叠图压暗其余，与趋势页交互语言一致 |
| 工具页比例 | 上方堆叠 110px → 130px（约 1:1.4） |
| 顺带修复 | `TrendWindow` 仍用按工具序列（`723e360` 遗漏），与嵌入版按 Provider 口径不一致，且键空间与日历药丸条对不上 —— 已统一为 `fetchProviderCrossToolSeries` |

### 新增的开发期预览手段（commit `0521c52`）

`src/lib/dev-mock.ts` 把 Tauri `invoke` 桩掉，配合 `docs/mockups/preview-frame.html`
按真实窗口尺寸（400×680 / 680×780 / 760×480）定尺预览**真组件**。

**为什么需要**：此前判断布局只能手抄一份简化版组件的 HTML/CSS，必然与真组件漂移。
改为直接跑真前端后，布局、间距、字号、圆角全部是真 CSS——改真代码后刷新即真实效果。

启用条件双重保险：`import.meta.env.DEV`（生产被 tree-shake，已验证 dashboard 产物
零膨胀）且页面无 `__TAURI_INTERNALS__`（`tauri dev` 下自动跳过）。

### 实测得到的口径结论（勿再重复推导）

- 本机账本 30 天 **3.01B tokens，其中 cache_read 占 78.6%**（minimax-code 97.8%）
- 火山服务端日账 30 天 **453M tokens**，与本机**几乎零重叠**（火山有账本的 20 天里本机
  GLM 几乎全为 0；本机 GLM 的 4 天里火山几乎全为 0）
- zcode 的 `model_usage.provider_id` 记着真实 provider（`account:bigmodel-start-plan`
  = 智谱直连），但 TUM 只读 `message.modelId`，**把这层信息丢掉了** → 后续可做的
  归因增强独立任务
- 结论：服务端日账与本机工具是**两个不同问题的答案**，并排对账不相加

---

## 追加：日历合并统一口径（2026-10-02，用户拍板「合并统一，以官方数据为准」，commit `f7efa05`）

上一条结论（「两个不同问题的答案，并排对账不相加」）的前提是**日历可以切口径**。
既然用户拍板不再切，这条结论的前提就不成立了——两套数据被合进同一张日历，
且必须定死「同一 provider 两边都有记录时听谁的」。

### 合并规则（逐 `(provider, date)` 独立判定）

- 该 provider 当天**有**服务端日账 → 层值 = 服务端值（官方口径优先）
- 否则 → 层值 = 本机跨工具归因值
- 总量 = 本机来源总量行之和 + Σ(有服务端数据的 provider 的「服务端值 − 本地值」)

即**替换**而非**相加**，同一 provider 两边都有记录时不会双计。
实现入口 `buildUnifiedBreakdown`（`buildLedgerBreakdown` 降级为其底座）。

### 三个容易做错、且错了不会报错的地方

1. **服务端有账、本机当天完全没有工具行的日子必须补进日历。**
   只遍历 `buildLedgerBreakdown` 的结果会把这类日子整段丢掉——而实测两套数据
   几乎零重叠（服务端 20 天 vs 本机 GLM 4 天），不补洞等于服务端那一侧白拿。
   已加测试 `本机当天完全没有工具行时…` + 回退验证确认测试有牙齿。

2. **单位决定能否并入，不能按「有没有账户日账」判断。**
   火山日明细是 tokens（`providers/volcengine.rs:471` 直接沿用 API 的
   `c.unit`）→ 并入；OpenAI/xAI 是 USD、Kimi 是 CNY → **不并入**，
   折进 token 总量就是 `docs/usage-ledger.md` 明令禁止的跨单位相加。
   它们只在 DetailCard 的近 7 日小柱里按各自单位单独呈现。

3. **`highlightKeyForKind` 不能再按 kind 排除账户日账类型。**
   合并后凡注册表里有的 kind 都能在日历里出现（火山走自己的 tokens 日账，
   OpenAI/xAI 走本机工具归因层）。继续排除会让火山卡片点击后切到趋势页
   看到「整张日历全被压暗」——**静默失败，无报错**。

### 已知边界（钉了测试，不是疏漏）

豆包（`doubao-*`）与火山方舟（`ark-*`）是两个 provider 层。火山账户的服务端
日账只替换 `volcengine` 层；本机 `doubao-*` 的调用是否走同一账户，前端无从判断
（zcode 的 `model_usage.provider_id` 有这层信息，但 TUM 目前只读
`message.modelId`）。实测重叠极小，当下无可见代价，等 provider_id 接上再收紧。

### UI 侧

`CalendarSection` 的「口径」药丸行删除（两行药丸并排真机验收时就被问到谁是谁），
只保留高亮药丸行。`HeatmapGrid` 恒定口径后不再有换源重取，`ledger`/`providerId`
两个 props 消失。窗口尺寸与 `ProviderCard` 视觉零改动。

---

## 硬约束（违反即视为实施错误）

1. **不改卡片视觉。** `src/lib/components/ProviderCard.svelte` 的 `<style>` 块一个字节都不动。卡片聚焦描边 `.card--focused`（`ProviderCard.svelte:278-281`）是**既有行为**，本计划不新增、不修改、不强化。
2. **不改 Rust。** `src-tauri/` 完全不动。所有数据来自既有的 `get_usage_history`（`ipc.rs:113`），它已返回 `source` / `model` / `kind` 字段，后端零改动。
3. **不跨单位相加。** 账户日账（火山 AFP / OpenAI USD）不得与本机工具 token 进入同一张日历的总量。依据 `docs/usage-ledger.md`「差分类混入历史视图曾导致模型页有数、日历为 0」。
4. **无按日数据的卡片，点击后不静默改视图。** 改为在 `DetailCard`（悬浮详情）内说明（方案 B）。

---

## 已确认的两个额外缺陷（本计划一并修）

**缺陷 A · `p:` 命名空间错配（联动失效的直接原因）**
`App.svelte:1112-1113` 写 `p:${CARD_TOOL_FOR_KIND[kind]}`，值是**工具 id**（`minimax-code`）；而下拉选项 `App.svelte:1133-1136` 产生的是 `p:${p.key}`，值是**provider key**（`minimax`）。`HeatmapGrid.svelte:55,63` 用 `p:` 后缀去和 `providerForModel()` 的返回值比较，永远匹配不上 → 零行 → 日历全空。同时 `<select value>` 找不到对应 `<option>`，下拉显示也错乱。

**缺陷 B · provider 聚合双计（现有 `p:` 模式一直双计，只是被缺陷 A 掩盖了）**
`local/mod.rs:222-262` 的 `persist_all_tools` 为每个工具每天写**两类行**：`model === ""` 的来源总量行，以及每个模型的**分模型行**。而 `HeatmapGrid.svelte:57-65` 循环里对 `p:<key>` 模式**没有**按 model 过滤，两类行都被累加 → **翻倍**。总量行 `model === ""` 经 `providerForModel("", source)` 也会经工具兜底归到同一个 provider key，所以双计是实打实的。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `src/lib/calendar-linkage.ts` | 联动唯一数据源：kind→序列映射、账本行→按日按 Provider 拆解、格子配色、焦点统计 | **新建** |
| `src/lib/calendar-linkage.test.ts` | 上述纯函数的单测 | **新建** |
| `src/lib/components/HeatmapGrid.svelte` | 消费拆解结果渲染分层高亮；删除 compact 分支 | 修改 |
| `src/App.svelte` | 状态改为 `calendarSource` + `highlightKey`；卡片点击写焦点；删除 31 天开关 | 修改 |
| `src/lib/components/DetailCard.svelte` | 改用 `ledgerSeriesForKind`；补「无按日数据」说明（方案 B） | 修改 |
| `package.json` | 新增 `test` 脚本 + vitest 依赖 | 修改 |
| `src/lib/components/ProviderCard.svelte` | **不动** | 禁止修改 |

---

### Task 0: 前端测试基座

项目目前**零前端测试**（无 vitest/playwright、无 CI）。P0/P1/P2 的核心都是纯函数，没有测试基座就只能靠人眼看——所以这是前置任务。

**Files:**
- Modify: `package.json:6-13`
- Create: `vitest.config.ts`

- [ ] **Step 1: 安装 vitest**

```bash
npm install -D vitest@^2.1.8
```

- [ ] **Step 2: 新增 `vitest.config.ts`**

只测纯 TS，不加载 Svelte 插件（避免 `runes: true` 编译器选项介入）：

```ts
// vitest.config.ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // 只覆盖 lib 下的纯逻辑；*.svelte 组件测试不在本计划范围
    include: ["src/lib/**/*.test.ts"],
    environment: "node",
  },
});
```

- [ ] **Step 3: 在 `package.json` 加 test 脚本**

把 `scripts` 块改为：

```json
  "scripts": {
    "dev": "vite",
    "build": "svelte-check && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "test:watch": "vitest",
    "tauri": "tauri",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build"
  },
```

- [ ] **Step 4: 确认基座可运行**

```bash
npm test
```

Expected: `No test files found` 退出码非 0，或显示 0 passed。**只要命令能跑通不报配置错误即可**，此时还没有测试文件是正常的。

- [ ] **Step 5: 提交**

```bash
git add package.json package-lock.json vitest.config.ts
git commit -m "chore(fe): 引入 vitest 测试基座"
```

---

### Task 1: 联动数据源（P0 + P1 核心）

建立唯一数据源。这一步同时修掉缺陷 A 的根因，并把 P1 要求的「三处映射表收敛」一次做完。

**Files:**
- Create: `src/lib/calendar-linkage.ts`
- Create: `src/lib/calendar-linkage.test.ts`

- [ ] **Step 1: 写失败的测试**

把 `src/lib/calendar-linkage.test.ts` 的**文件顶部 import 段**替换为：

```ts
import { describe, expect, it } from "vitest";
import { highlightKeyForKind, ledgerSeriesForKind } from "./calendar-linkage";
```

然后在文件**末尾**追加：

```ts
describe("ledgerSeriesForKind", () => {
  it("普通 kind 映射到跨工具 provider key（不是工具 id）", () => {
    // 回归：723e360 引入 model-provider.ts 后，下拉改用 provider key，
    // 但卡片点击仍写工具 id（minimax-code），导致 p:minimax-code 永不匹配。
    expect(ledgerSeriesForKind("minimax", "minimax-1756-0")).toEqual({
      mode: "cross-tool",
      providerKey: "minimax",
    });
    expect(ledgerSeriesForKind("deepseek", "deepseek-1756-1")).toEqual({
      mode: "cross-tool",
      providerKey: "deepseek",
    });
  });

  it("带后缀的子模式归一化后再判定（火山 API 按量）", () => {
    // instance_id 形如 volcengine_api-<epoch>-<n>，split("-")[0] 得到
    // "volcengine_api"，直接查集合会漏判、掉进兜底分支显示"全部工具"。
    expect(ledgerSeriesForKind("volcengine_api", "volcengine_api-1-0")).toEqual({
      mode: "account-daily",
      providerId: "volcengine_api-1-0",
    });
    expect(ledgerSeriesForKind("kimi_global", "kimi_global-1-0")).toEqual({
      mode: "cross-tool",
      providerKey: "kimi",
    });
  });

  it("有服务端日账的 kind 走 account-daily", () => {
    expect(ledgerSeriesForKind("volcengine", "volcengine-9-0")).toEqual({
      mode: "account-daily",
      providerId: "volcengine-9-0",
    });
  });

  it("未登记的 kind 返回 null（诚实：不猜）", () => {
    expect(ledgerSeriesForKind("some_unknown_thing", "x-1-0")).toBeNull();
  });
});

describe("highlightKeyForKind", () => {
  it("跨工具 kind 返回高亮 key", () => {
    expect(highlightKeyForKind("minimax")).toBe("minimax");
    expect(highlightKeyForKind("minimax_api")).toBe("minimax");
  });

  it("账户日账 kind 不可高亮（单位不同，混算即错）", () => {
    // 火山是 AFP、OpenAI 是 USD，与本机工具 token 不同量纲。
    expect(highlightKeyForKind("volcengine")).toBeNull();
    expect(highlightKeyForKind("openai")).toBeNull();
    expect(highlightKeyForKind("xai")).toBeNull();
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

```bash
npm test
```

Expected: FAIL —— `Cannot find module './calendar-linkage'`。

- [ ] **Step 3: 写实现**

创建 `src/lib/calendar-linkage.ts`：

```ts
// 日历 ↔ Provider 卡片联动的**唯一数据源**。
//
// 存在的理由：2026-10 之前 App / DetailCard 各维护一份 kind→序列映射表，
// 内容还不一致（App 有 deepseek，DetailCard 没有），且 App 那份映射到工具 id
// 而下拉已改用 provider key —— 两者在同一 `p:` 命名空间里对不上，点击卡片后
// 日历恒为空。单一数据源 + 纯函数是这类漂移的唯一根治办法。
//
// 全部为纯函数：不 import api.ts，不发 IPC，可直接单测。

import { normalizeKind } from "./brand-glyphs";

/** 某个 provider kind 在统一账本里对应的日序列。 */
export type LedgerSeries =
  /** 跨工具归因的 token 序列：按 provider key 跨工具聚合。 */
  | { mode: "cross-tool"; providerKey: string }
  /** 真实服务端日账：单位非 token（火山 AFP / OpenAI USD / xAI USD）。 */
  | { mode: "account-daily"; providerId: string }
  /** 差分类：无按日数据，联动不生效（不猜、不静默兜底）。 */
  | null;

/** kind → 账本里的 provider key。key 空间必须与 `providerForModel()` 的返回值一致。 */
const KIND_TO_PROVIDER_KEY: Record<string, string> = {
  minimax: "minimax",
  deepseek: "deepseek",
  anthropic: "anthropic",
  gemini: "gemini",
  qwen: "qwen",
  kimi: "kimi",
  doubao: "doubao",
  glm: "glm",
  xai: "xai",
  openai: "openai",
  volcengine: "volcengine",
  spark: "spark",
};

/** 有真实服务端日账的 kind（双写进 usage_daily，kind='provider'）。 */
const ACCOUNT_DAILY_KINDS = new Set(["volcengine", "openai", "xai"]);

/**
 * provider kind → 它该看哪条账本序列。
 *
 * `kind` 来自 `instance_id.split("-")[0]`，带子模式后缀（如 `volcengine_api`），
 * 故先 `normalizeKind` 归一再判定——`brand-glyphs.ts:68` 已有该函数，
 * 含 `kimi_global → kimi` 特判，此前这两处集合没用它是遗漏。
 */
export function ledgerSeriesForKind(kind: string, providerId: string): LedgerSeries {
  const base = normalizeKind(kind);
  if (ACCOUNT_DAILY_KINDS.has(base)) {
    return { mode: "account-daily", providerId };
  }
  const providerKey = KIND_TO_PROVIDER_KEY[base];
  return providerKey ? { mode: "cross-tool", providerKey } : null;
}

/**
 * provider kind → 日历高亮用的 provider key；不可高亮返回 null。
 *
 * 与 `ledgerSeriesForKind` 的区别：账户日账虽有按日数据，但单位与本机工具
 * token 不同量纲，混进同一张日历的总量会犯「跨单位相加」的错误（历史教训见
 * docs/usage-ledger.md）。故日历恒为工具 token 总量，只有跨工具 kind 可高亮。
 */
export function highlightKeyForKind(kind: string): string | null {
  const base = normalizeKind(kind);
  if (ACCOUNT_DAILY_KINDS.has(base)) return null;
  return KIND_TO_PROVIDER_KEY[base] ?? null;
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
npm test
```

Expected: 7 passed。

- [ ] **Step 5: 类型检查**

```bash
npx svelte-check --threshold warning
```

Expected: `0 errors and 0 warnings`。

- [ ] **Step 6: 提交**

```bash
git add src/lib/calendar-linkage.ts src/lib/calendar-linkage.test.ts
git commit -m "feat(fe): 联动唯一数据源 calendar-linkage（kind→账本序列映射）"
```

---

### Task 2: 账本按 Provider 拆解（修缺陷 B）

P2 需要「某日总量 + 某日各 Provider 拆解」。这里顺带修掉现有 `p:` 模式的双计。

**Files:**
- Modify: `src/lib/calendar-linkage.ts`（追加）
- Modify: `src/lib/calendar-linkage.test.ts`（追加）

- [ ] **Step 1: 写失败的测试**

把 `src/lib/calendar-linkage.test.ts` 的**文件顶部 import 段**替换为：

```ts
import { describe, expect, it } from "vitest";
import {
  buildLedgerBreakdown,
  focusStats,
  highlightKeyForKind,
  ledgerSeriesForKind,
} from "./calendar-linkage";
```

（后续任务只在文件末尾追加 `describe` 块，import 段不再改动。）

然后在文件**末尾**追加：

```ts
/** 构造一行账本。model 为 "" 即来源总量行。 */
const row = (
  source: string,
  date: string,
  model: string,
  total: number,
) => ({
  source,
  kind: "tool" as const,
  date,
  model,
  input: total,
  cache_read: 0,
  output: 0,
  total,
  unit: "tokens",
  cost: null,
  currency: null,
  cost_estimated: false,
});

describe("buildLedgerBreakdown", () => {
  it("总量只取来源总量行，不与分模型行相加", () => {
    // persist_all_tools 每个工具每天写两类行。旧实现两类都累加 → 翻倍。
    const rows = [
      row("minimax-code", "2026-10-01", "", 1000),      // 来源总量行
      row("minimax-code", "2026-10-01", "MiniMax-M3", 600), // 分模型行
      row("minimax-code", "2026-10-01", "deepseek-v4", 400), // 分模型行
    ];
    const map = buildLedgerBreakdown(rows);
    const d = map.get("2026-10-01")!;
    expect(d.total).toBe(1000);            // 不是 2000
    expect(d.byProvider).toEqual({ minimax: 600, deepseek: 400 });
  });

  it("分模型合计与来源总量对账一致（回归护栏）", () => {
    const rows = [
      row("zcode", "2026-10-02", "", 900),
      row("zcode", "2026-10-02", "GLM-5.3", 500),
      row("zcode", "2026-10-02", "未标记模型", 400),
    ];
    const d = buildLedgerBreakdown(rows).get("2026-10-02")!;
    const sum = Object.values(d.byProvider).reduce((s, v) => s + v, 0);
    expect(sum).toBe(d.total);
  });

  it("无模型信息的行按工具兜底归因（MiniMax Code → MiniMax）", () => {
    const rows = [row("minimax-code", "2026-10-03", "未标记模型", 700)];
    const d = buildLedgerBreakdown(rows).get("2026-10-03")!;
    expect(d.byProvider).toEqual({ minimax: 700 });
  });

  it("provider 账本行不计入本机工具口径", () => {
    const rows = [
      { ...row("volcengine-1-0", "2026-10-01", "", 50), kind: "provider" as const },
    ];
    expect(buildLedgerBreakdown(rows).size).toBe(0);
  });

  it("按日期升序无关，map 按需累加", () => {
    const rows = [
      row("codex", "2026-10-05", "", 300),
      row("codex", "2026-10-05", "gpt-5", 300),
      row("claude-code", "2026-10-05", "", 200),
    ];
    const d = buildLedgerBreakdown(rows).get("2026-10-05")!;
    expect(d.total).toBe(500);
    expect(d.byProvider).toEqual({ openai: 300 });
  });
});

describe("focusStats", () => {
  const win = ["2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"];
  const byProvider = { minimax: 100, deepseek: 200 };
  const days: Record<string, { total: number; byProvider: Record<string, number> }> = {
    "2026-10-01": { total: 400, byProvider },
    "2026-10-02": { total: 200, byProvider: { deepseek: 200 } },
    "2026-10-03": { total: 300, byProvider: { minimax: 100 } },
    "2026-10-04": { total: 100, byProvider: {} },
  };

  it("统计该 provider 的活跃天数/合计/峰值/占比", () => {
    const s = focusStats(days, win, "minimax", 1000);
    expect(s).toEqual({ days: 2, sum: 200, peak: 100, share: 20 });
  });

  it("窗口内无该 provider 用量时全部为 0，占比 0", () => {
    const s = focusStats(days, win, "kimi", 1000);
    expect(s).toEqual({ days: 0, sum: 0, peak: 0, share: 0 });
  });

  it("总量为 0 时占比不产生 NaN", () => {
    const s = focusStats(days, win, "minimax", 0);
    expect(s.share).toBe(0);
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

```bash
npm test
```

Expected: FAIL —— `buildLedgerBreakdown is not a function` / 导出不存在。

- [ ] **Step 3: 写实现**

在 `src/lib/calendar-linkage.ts` 顶部 import 段补一行，并把以下内容追加到文件末尾：

```ts
import type { UsageDailyRow } from "./api";
import { providerForModel } from "./model-provider";
```

追加内容：

```ts
/** 某一天的用量拆解。 */
export interface DayBreakdown {
  /** 当日全部本机工具 token 总量。 */
  total: number;
  /** 当日按 provider key 的拆解。 */
  byProvider: Record<string, number>;
}

/** 某个 provider 在窗口内的口径统计，用于日历统计行的第二行。 */
export interface FocusStats {
  /** 窗口内有该 provider 用量的天数。 */
  days: number;
  sum: number;
  peak: number;
  /** 占窗口总量的百分比（0-100，整数）。 */
  share: number;
}

/**
 * 账本行 → 按日、按 Provider 拆解。
 *
 * 关键口径（两行互不重叠，这是修掉旧实现翻倍 bug 的核心）：
 * - `model === ""` 的**来源总量行** → 只计入 `total`；
 * - `model !== ""` 的**分模型行** → 只计入 `byProvider`。
 *
 * 理由：provider 归因信息只存在于模型行（总量行没有模型名）；而总量必须用
 * 总量行，否则分模型行求和会因未覆盖模型而漏计。两者分别取，加起来才是完整
 * 一天。旧实现（HeatmapGrid.svelte:57-65）对 `p:<key>` 不按 model 过滤，
 * 两类行同时累加 → 翻倍。
 *
 * 仅消费 `kind === 'tool'` 行：provider 账本行单位非 token，混算即错。
 */
export function buildLedgerBreakdown(
  rows: UsageDailyRow[],
): Map<string, DayBreakdown> {
  const out = new Map<string, DayBreakdown>();
  const slot = (date: string): DayBreakdown => {
    let e = out.get(date);
    if (!e) {
      e = { total: 0, byProvider: {} };
      out.set(date, e);
    }
    return e;
  };
  for (const r of rows) {
    if (r.kind !== "tool") continue;
    const e = slot(r.date);
    if (r.model === "") {
      e.total += r.total;
    } else {
      const key = providerForModel(r.model, r.source);
      e.byProvider[key] = (e.byProvider[key] ?? 0) + r.total;
    }
  }
  return out;
}

/** 某 provider 在 `windowDates` 窗口内的口径统计。 */
export function focusStats(
  byDate: Map<string, DayBreakdown>,
  windowDates: string[],
  providerKey: string,
  windowTotal: number,
): FocusStats {
  let days = 0;
  let sum = 0;
  let peak = 0;
  for (const d of windowDates) {
    const v = byDate.get(d)?.byProvider[providerKey] ?? 0;
    if (v > 0) days += 1;
    sum += v;
    if (v > peak) peak = v;
  }
  return {
    days,
    sum,
    peak,
    share: windowTotal > 0 ? Math.round((sum / windowTotal) * 100) : 0,
  };
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
npm test
```

Expected: 15 passed（7 个来自 Task 1 + 8 个新增）。

- [ ] **Step 5: 提交**

```bash
git add src/lib/calendar-linkage.ts src/lib/calendar-linkage.test.ts
git commit -m "fix(fe): 账本按 Provider 拆解——总量/模型行分离，修掉 p: 模式双计"
```

---

### Task 3: 格子配色（分层高亮的纯逻辑）

把「某格该画成什么颜色」从 Svelte 组件里抽成可测的纯函数。这是 P2 视觉的核心，必须能单测。

**Files:**
- Modify: `src/lib/calendar-linkage.ts`（追加）
- Modify: `src/lib/calendar-linkage.test.ts`（追加）

- [ ] **Step 1: 写失败的测试**

把 `src/lib/calendar-linkage.test.ts` 的**文件顶部 import 段**替换为：

```ts
import { describe, expect, it } from "vitest";
import {
  buildLedgerBreakdown,
  DIM_FACTOR,
  focusStats,
  highlightKeyForKind,
  ledgerSeriesForKind,
  LEVEL_ALPHA,
  paintCell,
} from "./calendar-linkage";
```

然后在文件**末尾**追加：

```ts
const BRAND: [number, number, number] = [91, 140, 255]; // minimax #5b8cff

describe("paintCell · total 模式", () => {
  it("按总量分级取 alpha，level 4 带辉光", () => {
    const p = paintCell({ mode: "total", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.bg).toBe("rgba(76,194,255,0.94)");
    expect(p.shadow).toContain("0 0 4px");
  });

  it("level 0 = 浅灰空档（保持网格可见）", () => {
    expect(paintCell({ mode: "total", level: 0, isToday: false, focusValue: 0, brandRgb: BRAND }).bg)
      .toBe("rgba(76,194,255,0)");
  });

  it("今天叠加琥珀描边", () => {
    const p = paintCell({ mode: "total", level: 2, isToday: true, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toContain("var(--tum-amber)");
  });
});

describe("paintCell · highlight 模式", () => {
  it("焦点日用品牌色 + 亮描边，alpha 仍按总量分级（大小信息不丢）", () => {
    const p = paintCell({ mode: "highlight", level: 3, isToday: false, focusValue: 500, brandRgb: BRAND });
    expect(p.bg).toBe("rgba(91,140,255,0.66)");
    expect(p.shadow).toContain("rgba(91,140,255,0.95)");
  });

  it("非焦点日压暗到 DIM_FACTOR 倍，但仍是同一个色相", () => {
    const p = paintCell({ mode: "highlight", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.bg).toBe(`rgba(76,194,255,${LEVEL_ALPHA[4] * DIM_FACTOR})`);
    expect(p.shadow).not.toContain("inset 0 0 0 1px rgba(91,140,255");
  });

  it("非焦点日不再有 level-4 辉光（避免和焦点日抢注意力）", () => {
    const p = paintCell({ mode: "highlight", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toBe("none");
  });

  it("焦点日若恰是今天，两个描边都在", () => {
    const p = paintCell({ mode: "highlight", level: 2, isToday: true, focusValue: 1, brandRgb: BRAND });
    expect(p.shadow).toContain("rgba(91,140,255,0.95)");
    expect(p.shadow).toContain("var(--tum-amber)");
  });

  it("非焦点日若恰是今天，保留今天描边（今天必须始终可辨）", () => {
    const p = paintCell({ mode: "highlight", level: 1, isToday: true, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toContain("var(--tum-amber)");
  });

  it("压暗系数是严格小于 1 的正数（不能把非焦点日抹成不可见）", () => {
    expect(DIM_FACTOR).toBeGreaterThan(0);
    expect(DIM_FACTOR).toBeLessThan(1);
    expect(LEVEL_ALPHA[4] * DIM_FACTOR).toBeGreaterThan(0.1);
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

```bash
npm test
```

Expected: FAIL —— `paintCell` 未导出。

- [ ] **Step 3: 写实现**

追加到 `src/lib/calendar-linkage.ts` 末尾：

```ts
/** 强调色（系统蓝），与 HeatmapGrid 原有色阶一致。 */
const ACCENT_RGB: [number, number, number] = [76, 194, 255];

/** 总量色阶的 5 档 alpha。0 = 无用量。 */
export const LEVEL_ALPHA = [0, 0.22, 0.42, 0.66, 0.94] as const;

/** 非焦点格子的压暗系数：保留可辨的色相层次，不至于抹平成黑。 */
export const DIM_FACTOR = 0.22;

export type PaintMode = "total" | "highlight";

export interface CellPaint {
  bg: string;
  shadow: string;
}

const rgba = (rgb: [number, number, number], a: number): string =>
  `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${a})`;

/**
 * 某一天的格子该画成什么样子。
 *
 * `total` 模式 = 修复后的基线行为：按当日**总量**分级，全系统蓝。
 * `highlight` 模式 = P2 方案二：总量色阶不变，只改色相与透明度——
 *   该 provider 当天有用量 → 品牌色 + 亮描边（alpha 仍由总量分级决定，
 *   所以"那天烧了多少"的信息不丢）；
 *   否则 → 同一色相压暗到 {@link DIM_FACTOR}，退到背景层。
 *
 * 「今天」的琥珀描边在两种模式下都保留——它是时间锚点，不该被联动吞掉。
 */
export function paintCell(args: {
  mode: PaintMode;
  level: 0 | 1 | 2 | 3 | 4;
  isToday: boolean;
  /** 当日该 provider 的用量；0 表示当天没有它的消耗。 */
  focusValue: number;
  brandRgb: [number, number, number];
}): CellPaint {
  const { mode, level, isToday, focusValue, brandRgb } = args;
  const today = isToday ? "inset 0 0 0 1.5px var(--tum-amber)" : "";

  if (mode === "highlight" && focusValue > 0) {
    return {
      bg: rgba(brandRgb, LEVEL_ALPHA[level]),
      shadow: [`inset 0 0 0 1px ${rgba(brandRgb, 0.95)}`, today]
        .filter(Boolean)
        .join(","),
    };
  }
  if (mode === "highlight") {
    return {
      bg: rgba(ACCENT_RGB, LEVEL_ALPHA[level] * DIM_FACTOR),
      shadow: today || "none",
    };
  }
  return {
    bg: rgba(ACCENT_RGB, LEVEL_ALPHA[level]),
    shadow:
      [level === 4 ? `0 0 4px ${rgba(ACCENT_RGB, 0.45)}` : "", today]
        .filter(Boolean)
        .join(",") || "none",
  };
}

/** `#RRGGBB` → `[r,g,b]`；非法输入回落到强调色。 */
export function hexToRgbTriplet(hex: string): [number, number, number] {
  if (/^#[0-9a-f]{6}$/i.test(hex)) {
    return [
      parseInt(hex.slice(1, 3), 16),
      parseInt(hex.slice(3, 5), 16),
      parseInt(hex.slice(5, 7), 16),
    ];
  }
  return ACCENT_RGB;
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
npm test
```

Expected: 24 passed（15 + 9）。

- [ ] **Step 5: 提交**

```bash
git add src/lib/calendar-linkage.ts src/lib/calendar-linkage.test.ts
git commit -m "feat(fe): 格子分层高亮配色纯函数 paintCell"
```

---

### Task 4: `HeatmapGrid` 消费拆解结果

**Files:**
- Modify: `src/lib/components/HeatmapGrid.svelte:1-192`（script 段）
- Modify: `src/lib/components/HeatmapGrid.svelte:194-272`（markup 段）

- [ ] **Step 1: 替换 script 段的 import 与 Props**

把 `src/lib/components/HeatmapGrid.svelte` 从 `<script lang="ts">` 到 `} = $props();` 的整段（即现有的 import 段 + `interface Props` + 解构）替换为：

```svelte
<script lang="ts">
  import { getHeatmap, getUsageHistory } from "../api";
  import { formatUsage, type HeatmapCell, type UsageUnit } from "../types";
  import {
    buildLedgerBreakdown,
    focusStats,
    hexToRgbTriplet,
    paintCell,
    type DayBreakdown,
    type PaintMode,
  } from "../calendar-linkage";
  import { providerColor, providerLabel } from "../model-provider";

  interface Props {
    /** provider 日账模式的数据键（volcengine/openai/xai 等真实服务端日账）。 */
    providerId?: string;
    /** 账本模式：'all' = 合并全部本机工具。设置了 ledger 时优先于 providerId。 */
    ledger?: "all" | string | null;
    /** P2 分层高亮：要高亮的 provider key；null = 不高亮（基线行为）。 */
    highlightKey?: string | null;
    /** Shown instead of the default empty hint. */
    emptyHint?: string;
    /** Unit fallback before the first fetch resolves. */
    unit?: UsageUnit;
  }

  let {
    providerId = "",
    ledger = null,
    highlightKey = null,
    emptyHint,
    unit = "tokens" as UsageUnit,
  }: Props = $props();
```

- [ ] **Step 2: 替换 script 段剩余全部逻辑**

Step 1 之后，`<script>` 段里从 `const WEEKS = 5;`（现为第一条语句）一直到 `let displayUnit = $derived(grid.windowCells[0]?.unit ?? unit);` 结束的**全部内容**整体替换为：

```ts
  const CAL_WEEKS = 26; // 近 6 个月的周列数（周一对齐）
  const CAL_DAYS = 200; // 与 CAL_WEEKS 匹配的取数天数

  type Day = {
    date: string;
    value: number;
    level: 0 | 1 | 2 | 3 | 4;
    isToday: boolean;
  } | null;

  let cells = $state<HeatmapCell[]>([]);
  let breakdown = $state<Map<string, DayBreakdown>>(new Map());
  let loaded = $state(false);
  let fetchSeq = 0;

  $effect(() => {
    const seq = ++fetchSeq;
    loaded = false;
    cells = [];
    breakdown = new Map();
    const fetcher = ledger
      ? getUsageHistory(CAL_DAYS).then(({ rows }) => {
          // 一次性拆解：总量走来源总量行，provider 归因走分模型行。
          const map = buildLedgerBreakdown(rows);
          breakdown = map;
          return [...map.entries()]
            .sort((a, b) => a[0].localeCompare(b[0]))
            .map(([date, d]) => ({ date, value: d.total, unit: "tokens" as UsageUnit }));
        })
      : getHeatmap(providerId, CAL_DAYS);
    fetcher
      .then((rows) => {
        if (seq === fetchSeq) cells = rows;
      })
      .catch(() => {
        if (seq === fetchSeq) cells = [];
      })
      .finally(() => {
        if (seq === fetchSeq) loaded = true;
      });
  });

  function localDateKey(d: Date): string {
    const y = d.getFullYear();
    const m = `${d.getMonth() + 1}`.padStart(2, "0");
    const day = `${d.getDate()}`.padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  let byDate = $derived.by(() => {
    const map = new Map<string, HeatmapCell>();
    for (const c of cells) map.set(c.date, c);
    return map;
  });

  let paintMode = $derived<PaintMode>(highlightKey ? "highlight" : "total");
  let brandRgb = $derived(hexToRgbTriplet(providerColor(highlightKey ?? "minimax")));

  let grid = $derived.by(() => {
    const today = new Date();
    const todayKey = localDateKey(today);
    const todayDow = (today.getDay() + 6) % 7; // 0=Mon, 6=Sun
    const start = new Date(today);
    start.setDate(today.getDate() - todayDow - (CAL_WEEKS - 1) * 7);
    const startKey = localDateKey(start);
    const windowCells = cells.filter((c) => c.date >= startKey && c.date <= todayKey);
    const windowDates = windowCells.map((c) => c.date);
    const cols: Day[][] = [];
    const monthLabels: Array<string | null> = [];
    let prevMonth = -1;
    for (let wi = 0; wi < CAL_WEEKS; wi++) {
      const col: Day[] = [];
      for (let di = 0; di < 7; di++) {
        const d2 = new Date(start);
        d2.setDate(start.getDate() + wi * 7 + di);
        if (d2 > today) {
          col.push(null);
          continue;
        }
        const dateStr = localDateKey(d2);
        const value = byDate.get(dateStr)?.value ?? 0;
        col.push({
          date: dateStr,
          value,
          level: levelFor(value, windowCells),
          isToday: dateStr === todayKey,
        });
      }
      cols.push(col);
      const monday = new Date(start);
      monday.setDate(start.getDate() + wi * 7);
      const m = monday.getMonth();
      monthLabels.push(wi > 0 && m !== prevMonth ? `${m + 1}月` : null);
      prevMonth = m;
    }
    const total = windowCells.reduce((s, c) => s + c.value, 0);
    const peak = Math.max(0, ...windowCells.map((c) => c.value));
    const activeDays = windowCells.filter((c) => c.value > 0).length;
    const stats = highlightKey
      ? focusStats(breakdown, windowDates, highlightKey, total)
      : null;
    return { cols, windowCells, windowDates, monthLabels, total, peak, activeDays, stats };
  });

  function levelFor(value: number, all: HeatmapCell[]): 0 | 1 | 2 | 3 | 4 {
    if (value <= 0) return 0;
    const max = Math.max(1, ...all.map((c) => c.value));
    const t = value / max;
    if (t < 0.25) return 1;
    if (t < 0.5) return 2;
    if (t < 0.75) return 3;
    return 4;
  }

  function paint(day: NonNullable<Day>) {
    return paintCell({
      mode: paintMode,
      level: day.level,
      isToday: day.isToday,
      focusValue: highlightKey
        ? (breakdown.get(day.date)?.byProvider[highlightKey] ?? 0)
        : 0,
      brandRgb,
    });
  }

  /** tooltip：当日总量 + 按 provider 构成明细。 */
  function titleFor(day: NonNullable<Day>): string {
    const parts = Object.entries(breakdown.get(day.date)?.byProvider ?? {})
      .filter(([, v]) => v > 0)
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => `  ${providerLabel(k)} ${formatUsage(v, displayUnit)}`);
    return [`${day.date} · ${formatUsage(day.value, displayUnit)}`, ...parts].join("\n");
  }

  const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];
  let legendColors = $derived(
    [1, 2, 3, 4].map((l) =>
      paintCell({ mode: "total", level: l as 1 | 2 | 3 | 4, isToday: false, focusValue: 0, brandRgb }),
    ),
  );
  let maxValue = $derived(Math.max(0, ...grid.windowCells.map((c) => c.value)));
  let displayUnit = $derived(grid.windowCells[0]?.unit ?? unit);
</script>
```

**注意声明顺序**：`titleFor()` 里用到 `displayUnit`，所以 `let displayUnit = $derived(...)` 必须排在 `titleFor` 之前（本块已按此顺序排列）。Svelte 编译器的 TDZ 检查对 runes 尤其严格，顺序写反会直接报 `Cannot access 'displayUnit' before initialization`。

- [ ] **Step 3: 替换 markup**

把 `<script>` 段**之后**、`<style>` 段**之前**的整段 markup（即现有的 `<div class="heatmap" ...>` 到与之配对的收尾 `</div>`）替换为：

```svelte
<div class="heatmap">
  {#if loaded && maxValue <= 0}
    <div class="heatmap__empty">{emptyHint ?? "该来源暂无热力图数据"}</div>
  {:else}
    <div class="cal">
      <div class="cal__top">
        <span class="cal__corner"></span>
        <div class="cal__months">
          {#each grid.monthLabels as label}
            <span class="cal__month" class:cal__month--has={!!label}>{label ?? ""}</span>
          {/each}
        </div>
      </div>
      <div class="cal__body">
        <div class="cal__weekdays">
          {#each WEEKDAY_LABELS as wd}
            <span class="cal__wd">{wd}</span>
          {/each}
        </div>
        <div class="cal__cols">
          {#each grid.cols as col}
            <div class="cal__col">
              {#each col as day}
                {#if day}
                  {@const p = paint(day)}
                  <div
                    class="cal__cell"
                    style={`background:${p.bg};box-shadow:${p.shadow}`}
                    title={titleFor(day)}
                  ></div>
                {:else}
                  <div class="cal__cell cal__cell--future"></div>
                {/if}
              {/each}
            </div>
          {/each}
        </div>
      </div>
      <div class="cal__foot">
        <div class="cal__stats">
          <span class="cal__stat">累计 <b>{formatUsage(grid.total, displayUnit)}</b></span>
          <span class="cal__stat">峰值 <b>{formatUsage(grid.peak, displayUnit)}</b></span>
          <span class="cal__stat">活跃 <b>{grid.activeDays} 天</b></span>
        </div>
        <div class="heatmap__legend">
          <span class="heatmap__legend-text">少</span>
          {#each legendColors as c}
            <span class="heatmap__legend-cell" style={`background:${c.bg}`}></span>
          {/each}
          <span class="heatmap__legend-text">多</span>
          {#if highlightKey}
            <span class="heatmap__legend-sep"></span>
            <span
              class="heatmap__legend-cell"
              style={`background:rgba(${brandRgb.join(",")},0.66);box-shadow:inset 0 0 0 1px rgba(${brandRgb.join(",")},0.95)`}
            ></span>
            <span class="heatmap__legend-text">{providerLabel(highlightKey)}</span>
          {/if}
        </div>
      </div>
    </div>
    {#if highlightKey && grid.stats}
      <!-- P2 追加行：总量恒定，焦点 provider 自己的口径单列一行 -->
      <div class="cal__stats cal__stats--focus">
        <span class="cal__stat">{providerLabel(highlightKey)} 活跃 <b>{grid.stats.days} 天</b></span>
        <span class="cal__stat">累计 <b>{formatUsage(grid.stats.sum, displayUnit)}</b></span>
        <span class="cal__stat">峰值 <b>{formatUsage(grid.stats.peak, displayUnit)}</b></span>
        <span class="cal__stat">占总量 <b>{grid.stats.share}%</b></span>
      </div>
    {/if}
  {/if}
</div>
```

- [ ] **Step 4: 补两条新样式**

在 `<style>` 段末尾（`}` 之前）追加：

```css
  .heatmap__legend-sep {
    width: 1px;
    height: 12px;
    background: var(--tum-border-strong);
    margin: 0 4px;
  }
  .cal__stats--focus {
    margin-top: 3px;
    color: var(--tum-text-secondary);
  }
  .cal__stats--focus b {
    color: var(--tum-text-primary);
  }
```

- [ ] **Step 5: 删除 compact 视图的 CSS**

删除 `<style>` 段中的 `.heatmap__cols`、`.heatmap__col`、`.heatmap__cell`、`.heatmap__cell--future`、`.heatmap__cell:hover`、`.cal__foot .heatmap__legend` 规则（`.heatmap`、`.heatmap__empty`、`.heatmap__legend*`、`.cal*` 保留）。

- [ ] **Step 6: 类型检查 + 测试**

```bash
npx svelte-check --threshold warning && npm test
```

Expected: `0 errors and 0 warnings` + `24 passed`。

- [ ] **Step 7: 提交**

```bash
git add src/lib/components/HeatmapGrid.svelte
git commit -m "feat(fe): HeatmapGrid 消费账本拆解，渲染分层高亮与焦点统计"
```

---

### Task 5: `App.svelte` 状态与卡片点击

**Files:**
- Modify: `src/App.svelte:484-496`（常量与状态）
- Modify: `src/App.svelte:1106-1119`（onSelect）
- Modify: `src/App.svelte:1126-1178`（日历区）

- [ ] **Step 1: 替换常量与状态**

把 `src/App.svelte:484-496` 从 `// 日历区数据选择：` 到 `let heatmapProviders = $state<...>([]);` 整段替换为：

```ts
  // 日历区状态拆成两个正交概念（P2）：
  //  - calendarSource = **数据口径**：全部工具合并，还是某个账户的服务端日账。
  //  - highlightKey    = **高亮焦点**：在口径不变的前提下高亮哪个 provider。
  // 旧实现把两者塞进同一个 heatmapTabId，点卡片既切口径又切数据，
  // 导致「日历变了」无法归因；拆分后点卡片只改焦点，数字恒定。
  const HEATMAP_TOOLS = "__tools__";
  let calendarSource = $state<string>(HEATMAP_TOOLS);
  let highlightKey = $state<string | null>(null);
```

- [ ] **Step 2: 替换卡片 onSelect**

把 `src/App.svelte:1106-1119` 的 onSelect 替换为：

```ts
              onSelect={() => {
                const kind = snap.provider_id.split("-")[0];
                const series = ledgerSeriesForKind(kind, snap.provider_id);
                // 无按日数据的 kind（差分类）：不猜、不静默改视图，交给
                // DetailCard 说明。点击既不改焦点也不改口径。
                if (!series) return;
                // 再点一次已聚焦的卡片 = 取消高亮，让焦点可撤销。
                if (focus === snap.provider_id && highlightKey !== null) {
                  focus = "all";
                  highlightKey = null;
                  return;
                }
                focus = snap.provider_id;
                if (series.mode === "cross-tool") {
                  // 切回工具口径：账户日账单位不同，不能在高亮态下混算。
                  calendarSource = HEATMAP_TOOLS;
                  highlightKey = series.providerKey;
                } else {
                  calendarSource = series.providerId;
                  highlightKey = null;
                }
              }}
```

- [ ] **Step 3: 补 import**

在 `src/App.svelte` 顶部 import 区（`import { brandColorFor, EXPERIMENTAL_KINDS } from "./lib/brand-glyphs";` 附近）加：

```ts
import { ledgerSeriesForKind } from "./lib/calendar-linkage";
```

- [ ] **Step 4: 替换日历区 markup**

把 `src/App.svelte:1126-1178` 的 `<section class="shell__heatmap">` 整段替换为：

```svelte
    {#if snapshots.length > 0}
      <section class="shell__heatmap" data-tauri-drag-region={false}>
        <div class="heatmap__head">
          <span class="heatmap__title">日历热力图</span>
          <PillsOrSelect
            items={[
              { id: HEATMAP_TOOLS, label: "全部工具" },
              ...snapshots
                .filter((s) => ledgerSeriesForKind(s.provider_id.split("-")[0], s.provider_id)?.mode === "account-daily")
                .map((s) => ({
                  id: s.provider_id,
                  label: `${s.provider_display_name} · 账户日账`,
                })),
            ]}
            value={calendarSource}
            onPick={(id) => {
              calendarSource = id;
              // 手动切口径时清掉高亮：高亮只在工具口径下有意义。
              if (id !== HEATMAP_TOOLS) highlightKey = null;
            }}
            dotFor={(id) =>
              id === HEATMAP_TOOLS ? "#4cc2ff" : colorOf(id)
            }
          />
          {#if highlightKey}
            <span
              class="heatmap__follow"
              style={`--follow:${providerColor(highlightKey)}`}
            >
              高亮 {providerLabel(highlightKey)}
              <button
                type="button"
                class="heatmap__follow-x"
                aria-label="取消高亮"
                onclick={() => {
                  highlightKey = null;
                  focus = "all";
                }}
              >×</button>
            </span>
          {/if}
        </div>
        <HeatmapGrid
          ledger={calendarSource === HEATMAP_TOOLS ? "all" : null}
          providerId={calendarSource === HEATMAP_TOOLS ? "" : calendarSource}
          {highlightKey}
          emptyHint={calendarSource === HEATMAP_TOOLS
            ? "暂无本机工具用量——使用 Claude Code / ZCode 等工具后会自动记录"
            : "该来源暂无热力图数据"}
        />
      </section>
    {/if}
```

- [ ] **Step 5: 删除 31 天视图状态与开关**

删除 `src/App.svelte:127-137` 的 `HEATMAP_VIEW_KEY` / `heatmapView` / 持久化 `$effect`，以及 `src/App.svelte:1152-1165` 的 `.heatmap__view` 按钮组。删除后 `.heatmap__view` / `.heatmap__view-btn` 的 CSS 规则也一并删除。

- [ ] **Step 6: 删除已失效的 `refreshHeatmapProviders` 整条链路**

Step 4 的新 markup 不再需要 `p:<key>` 下拉选项（高亮由卡片设置，不走下拉），因此
`heatmapProviders` 的**唯一消费者已经消失**，`refreshHeatmapProviders` 整个函数
连同两个调用点一并删除：

1. 删除 `src/App.svelte:360-378` 从 `// 日历区的 Provider 胶囊：` 注释起、
   到 `unlistenFns.push(await onToolsUpdated(() => void refreshHeatmapProviders()));`
   为止的整段（含 `const refreshHeatmapProviders = async () => {...}`、
   `void refreshHeatmapProviders();` 与 `onToolsUpdated` 的订阅）。

   > `getUsageHistory` 在 `App.svelte` 中**仅**被该函数使用，删除后需同步移除
   > import（见 Step 6b）；趋势/模型页各自在自己的组件里独立取数，不受影响。

2. 把 `src/App.svelte:5` 的
   `import { providerColor, providerForModel, providerLabel } from "./lib/model-provider";`
   改为
   `import { providerColor, providerLabel } from "./lib/model-provider";`
   （`providerForModel` 的唯一调用点在被删的函数里；`providerColor` / `providerLabel`
   仍被新的 follow chip 使用，保留。）

3. 从 `src/App.svelte` 顶部 `./lib/api` 的 import 列表中移除 `getUsageHistory`。

4. 删除 `let heatmapProviders = $state<{ key: string; label: string; total: number }[]>([]);`
   声明（Step 1 已删除其消费方，此处补删声明本身）。

- [ ] **Step 7: 补 follow chip 样式**

在 `src/App.svelte` 的 `<style>` 段追加：

```css
  .heatmap__follow {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--tum-font-size-xs);
    font-weight: 600;
    color: var(--follow);
    background: color-mix(in srgb, var(--follow) 16%, transparent);
    border: 1px solid color-mix(in srgb, var(--follow) 45%, transparent);
    border-radius: var(--tum-radius-pill);
    padding: 2px 4px 2px 8px;
  }
  .heatmap__follow-x {
    appearance: none;
    border: 0;
    background: rgba(0, 0, 0, 0.3);
    color: inherit;
    cursor: pointer;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    font-size: 10px;
    line-height: 1;
    display: grid;
    place-items: center;
  }
  .heatmap__follow-x:hover {
    background: rgba(0, 0, 0, 0.55);
  }
```

- [ ] **Step 8: 类型检查 + 测试**

```bash
npx svelte-check --threshold warning && npm test
```

Expected: `0 errors and 0 warnings` + `24 passed`。

- [ ] **Step 9: 提交**

```bash
git add src/App.svelte
git commit -m "feat(fe): 卡片点击改为设置高亮焦点，口径与焦点解耦，支持再点取消"
```

---

### Task 6: 方案 B — `DetailCard` 说明无按日数据

> **⚠️ 2026-10-02 修订**：日历迁至趋势页后（见下方「追加：信息架构重构」），本任务的
> 触发路径变了。差分类 Provider（如 Kimi）点卡片**不再驱动日历**，而是：
> 既不改 `focus`、也不改 `highlightKey`——`ledgerSeriesForKind` 返回 `null` 时
> 直接 `return`。静默仍在，只是发生在总览页而不是日历上。
>
> 因此说明的**位置与措辞**需要重新确认：说「不参与日历联动」已不准确（用户此时
> 根本没在日历上）。更贴切的表述是「该 Provider 无按日用量数据，无法按天查看」。
> 下方代码按新措辞给出，实现前先与用户确认文案。

卡片视觉零改动（硬约束 1），说明放在用户本来就会看的悬浮详情里。

**Files:**
- Modify: `src/lib/components/DetailCard.svelte:107-143`

- [ ] **Step 1: 替换常量与取数逻辑**

把 `src/lib/components/DetailCard.svelte:107-143` 从 `// --- 近 7 日迷你柱状：统一账本口径 ---` 到 `});` 结束的整个 `$effect` 块替换为：

```svelte
  // --- 近 7 日迷你柱状 + 无按日数据说明（统一账本口径）---
  // 数据源选择由 calendar-linkage 单一决定，不再在本组件维护映射表
  // （旧 LEDGER_TOOL_FOR_KINDS 只含 minimax，与 App 侧不一致）。
  let weekCells = $state<HeatmapCell[]>([]);
  let weekSeq = 0;
  // 差分类 provider 无按日数据：小图不画，并给出说明（方案 B —— 卡片视觉不动）。
  let noDailyNote = $state<string | null>(null);

  $effect(() => {
    const providerId = snapshot.provider_id;
    const kind = providerId.split("-")[0];
    const series = ledgerSeriesForKind(kind, providerId);
    const seq = ++weekSeq;
    if (!series) {
      noDailyNote = "该 Provider 无按日用量数据，不参与日历联动";
      weekCells = [];
      return;
    }
    getUsageHistory(7)
      .then(({ rows }) => {
        if (seq !== weekSeq) return;
        noDailyNote = null;
        const picked = rows.filter((r) => {
          if (r.model !== "") return false; // 只取来源总量行，避免双计
          if (series.mode === "account-daily") {
            return r.kind === "provider" && r.source === series.providerId;
          }
          return r.kind === "tool" && providerForModel("", r.source) === series.providerKey;
        });
        weekCells = picked.map((r) => ({
          date: r.date,
          value: r.total,
          unit: (r.unit === "usd" ? "usd" : r.unit === "cny" ? "cny" : "tokens") as UsageUnit,
        }));
      })
      .catch(() => {
        if (seq === weekSeq) weekCells = [];
      });
  });
```

- [ ] **Step 2: 补 import**

在 `src/lib/components/DetailCard.svelte` import 区加：

```ts
  import { ledgerSeriesForKind } from "../calendar-linkage";
  import { providerForModel } from "../model-provider";
```

- [ ] **Step 3: 在小图位置渲染说明**

找到渲染近 7 日小柱状图的 `{#if weekCells.length}` 块，在其前面插入：

```svelte
    {#if noDailyNote}
      <p class="detail__note">{noDailyNote}</p>
    {/if}
```

- [ ] **Step 4: 补样式**

在 `<style>` 段追加：

```css
  .detail__note {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    line-height: 1.5;
    padding: 2px 0;
  }
```

- [ ] **Step 5: 类型检查 + 测试**

```bash
npx svelte-check --threshold warning && npm test
```

Expected: `0 errors and 0 warnings` + `24 passed`。

- [ ] **Step 6: 提交**

```bash
git add src/lib/components/DetailCard.svelte
git commit -m "feat(fe): 详情卡改用统一映射表；无按日数据时给出说明（方案 B）"
```

---

### Task 7: 全量验收

- [ ] **Step 1: 静态检查全绿**

```bash
npm test
npx svelte-check --threshold warning
npx vite build
```

Expected: `24 passed` / `0 errors and 0 warnings` / `✓ built`。

- [ ] **Step 2: 确认硬约束未被违反**

```bash
git diff HEAD~6 --stat -- src/lib/components/ProviderCard.svelte
git diff HEAD~6 --stat -- src-tauri/
```

Expected: 两条命令**均无输出**。有输出即说明动了卡片或后端，违反硬约束。

- [ ] **Step 3: 确认无残留死代码**

用 Grep 工具（或 `rg`）在 `src/` 下搜索：

```
heatmapTabId|heatmapProviders|CARD_TOOL_FOR_KIND|LEDGER_TOOL_FOR_KIND|PROVIDER_DAILY_KINDS|PROVIDER_LEDGER_KINDS|heatmapView|HEATMAP_VIEW_KEY|heatmap__view
```

Expected: **无任何匹配**。有匹配说明有漏删的死代码、旧引用或残留 CSS。

- [ ] **Step 4: 真机验收（手动，需用户执行）**

启动 `npm run tauri:dev`，按下列清单逐项确认：

| # | 操作 | 预期 |
|---|---|---|
| 1 | 点 MiniMax 卡片 | 日历出现 `高亮 MiniMax ×` chip；MiniMax 有用量的格子变品牌蓝+亮描边，其余压暗；**累计/峰值/活跃三数不变** |
| 2 | 看统计行 | 下方追加 `MiniMax 活跃 N 天 / 累计 X / 峰值 Y / 占总量 Z%`，其中 X 应等于总累计的一半左右 |
| 3 | 再点一次同一张卡片 | 高亮清除，chip 消失，恢复全系统蓝 |
| 4 | 点 chip 上的 × | 同上清除 |
| 5 | 点 DeepSeek 卡片 | 同 1，但换成 DeepSeek 品牌蓝 |
| 6 | 点 Kimi 卡片 | **日历完全不变**；悬浮详情卡出现「该 Provider 无按日用量数据，不参与日历联动」 |
| 7 | 鼠标悬停任意格子 | tooltip 首行是当日总量，下方按 Provider 列出构成与数值 |
| 8 | 用下拉切到「Volcano · 账户日账」 | 日历切为该账户日账，**高亮 chip 消失**（不同量纲不混算） |
| 9 | 观察日历区 | **只有月历**，`31天` / `月历` 两个切换按钮已不存在 |
| 10 | 观察任意 Provider 卡片 | 样式与改动前完全一致（仅既有 `.card--focused` 描边） |

- [ ] **Step 5: 对账验证（可选但推荐）**

在 DevTools 或临时脚本里对同一 31 天窗口跑：

```
总量（model === "" 求和） ≈ Σ 各 provider（model !== "" 归因后求和）
```

误差应为 0（见 Task 2 测试 `分模型合计与来源总量对账一致`）。若不等，说明账本里有未覆盖模型的行，需要回头查 `local/` 扫描器而非改前端。

- [ ] **Step 6: 提交（如有收尾改动）**

```bash
git add -A
git commit -m "chore: 日历联动改造收尾"
```

---

## 验收标准（Definition of Done）

| 项 | 判据 |
|---|---|
| 联动恢复 | 点跨工具 Provider 卡片，日历高亮该 provider，**不再变空白** |
| 口径恒定 | 高亮态下累计/峰值/活跃三数与未高亮时完全一致 |
| 无双计 | 总量 = 来源总量行求和；provider 拆解 = 分模型行求和；两者不互相污染 |
| 归一化 | `volcengine_api` / `kimi_global` / `minimax_api` 等子模式 kind 均能正确判定 |
| 诚实兜底 | 差分类 Provider 点击不改视图，详情卡给出说明 |
| 可撤销 | 再点卡片 / 点 chip 的 × 均能清除高亮 |
| 单一数据源 | 全局 grep 不到任何第二份 kind→序列映射表 |
| 卡片零改动 | `git diff --stat -- ProviderCard.svelte` 无输出 |
| 后端零改动 | `git diff --stat -- src-tauri/` 无输出 |
| 31 天移除 | 日历区只剩月历，compact 分支代码已删 |
| 质量闸门 | `npm test` 24 passed；`svelte-check` 0 error 0 warning；`vite build` 成功 |
