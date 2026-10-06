# 设计令牌合规修复（Design Token Conformance）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 2026-10-06 设计合规审计发现的问题修掉：语义色归位（禁用 hex → ok/warn/crit token）、两个独立窗口标题区接入 PanelHeader 原子、兜底色与外来中性色收敛、值保持型字号/圆角/白面字面量 token 化，并用一个设计令牌 lint 测试（vitest）把成果锁住。

**Architecture:** 纯前端（Svelte 5 + tokens.css）改动，不动任何业务逻辑与 Rust 侧。每个任务先扩 lint 测试规则（红），再修代码（绿），提交保持绿色。可见变化被压缩到最小并逐条列出。

**Tech Stack:** Svelte 5、Vite 多页（dashboard/settings/trend/tools/peek）、vitest、svelte-check。

**基准文档:** `docs/FRONTEND-DESIGN-STYLE.md`、`docs/COMPONENT-LIBRARY.md`、`src/styles/tokens.css`

---

## 明确的范围决定

- **不在本计划内**：Settings.svelte 整体换肤（文档 §8 已列为需单独审视的任务，本计划仅把它列为 lint 豁免并记录）、HeatmapGrid / CalendarSection 的结构性原子迁移（文档 §8 已列）、App.svelte `pill-breathe` 收敛到 PulseDot（文档 §8 第 4 条）。CalendarSection 只顺手修两处颜色 hex（见 Task 4），不动结构。
- **数据系列色板不是违规**：`DEV_COLORS`（DevicePanel）、`RING_COLORS`（ModelPanel）、`#76a9ff/#ffcc66` 系列（ToolPanel/ToolWindow）、`brandColorFor` 等属于设计文档 §7.4 允许的"可编程强调色"，不进 lint 禁用清单。
- **间距不做 blanket token 化**：只把"值恰好等于 token 值"的字面量换成 var()；6/10/14/5px 等 token 外间距一律保留原值（改值会动布局，留给 Settings 时代统一处理）。间距规则不进 lint（多值简写 regex 误报率高，收益低）。
- **DetailCard 玻璃模糊（已拍板：方案 a）**：兑现文档 §3——给 `.detail` 加 `backdrop-filter: blur(var(--tum-blur-card))` 并降低底色不透明度。这是可见变化，按"UI 改动先出原型"规则先出交互式原型 `docs/mockups/2026-10-06-detail-card-glass.html`（可切换 现状/玻璃、调 blur 半径与底色不透明度），**用户拍板后原型即契约**，具体数值以原型确认为准（Task 8.5）。tokens.css 的 `--tum-blur-card` 保留并开始被真实引用。

## 可见变化点（执行前请过目，均为向已文档化设计语言收敛）

1. DevicePanel 删除确认钮 hover 红 `#ff7a6e` → `--tum-crit`（#ff5f56，更饱和一点）；
2. DevicePanel 告警横幅文字 `#ffc77a` → `--tum-warn`（#ffc83d，几乎不可分辨）、描边 0.35 → 0.45 透明度（与 accent-stroke 规则统一）；
3. 工具窗口 / 趋势窗口标题字号 11px → 10px、字距 0.6px → 1.2px（= PanelHeader 原子样式，与四个面板同款）；
4. ToolWindow 工具选择器从"托盘式无边框胶囊"换成 PillsOrSelect 原子（与模型/工具/日历面板同款：≤3 项带描边胶囊，>3 项自动变下拉）；
5. PillsOrSelect / CalendarSection 原生下拉弹层底色 `#23262d` → `--tum-bg-solid`（#1a1b1e，略深）；
6. RangePills 新增键盘焦点描边（`:focus-visible`，仅键盘出现）；
7. 其余全部为值保持型替换（渲染结果不变或差异 ≤0.015 alpha）。

---

## 文件结构

| 文件 | 动作 | 职责 |
|------|------|------|
| `src/lib/design-tokens.test.ts` | 新建 | 设计令牌 lint：扫描全部 .svelte（除 Settings），禁 hex / 禁字面量 px |
| `src/styles/tokens.css` | 修改 | 增补 warn/crit 辅助 token；删除死 token `--tum-blur-card` |
| `src/App.svelte` | 修改 | 路由状态点 token 化；`FOCUS_FALLBACK_COLOR` 写法；view-tab/chip 字号与圆角 |
| `src/lib/components/DevicePanel.svelte` | 修改 | 状态色、字号、圆角、999px、白面 rgba、去 var fallback |
| `src/lib/components/PillsOrSelect.svelte` | 修改 | 下拉弹层色、ps__dot → ColorSwatch |
| `src/lib/components/CalendarSection.svelte` | 修改 | 仅两处颜色 hex（不动结构） |
| `src/lib/components/TrendLineChart.svelte` | 修改 | 兜底色写法、tip-swatch → ColorSwatch |
| `src/lib/components/TrendWindow.svelte` | 修改 | 接 PanelHeader；兜底色；字号/圆角 |
| `src/lib/components/ToolWindow.svelte` | 修改 | 接 PanelHeader（清死导入）；pickers → PillsOrSelect；字号/圆角 |
| `src/lib/components/{ToolPanel,ModelPanel,TrendPanel,MiniPanel}.svelte` | 修改 | 值保持型字号/圆角 |
| `src/lib/series-palette.ts` | 新建 | 工具系列色单一来源（ToolPanel/ToolWindow 共用） |
| `docs/FRONTEND-DESIGN-STYLE.md` | 修改 | 附录 A 补缺失 token；§3 悬浮层表述对齐现实 |
| `docs/COMPONENT-LIBRARY.md` | 修改 | §7 迁移表、§8 待迁移清单、lint 说明 |

---

### Task 0: 基线验证（不写代码）

**Files:** 无改动

- [ ] **Step 1: 确认工作区干净起点**

```bash
cd D:/Lab/AI_Agents/TokenUsageMonitor && git status --short
```

预期：只有审计时的已知改动（components/*、atoms/、COMPONENT-LIBRARY.md）。若有未预期改动，先停下问用户。

- [ ] **Step 2: 跑通现有检查并记录基线**

```bash
npm run build 2>&1 | tail -20   # svelte-check && vite build
npm test 2>&1 | tail -6
```

预期：全部通过。记录 vite 输出里 `dist/dashboard` 的 gzip 前体积（文档基线 81.22 kB），Task 10 要对比不退化。

- [ ] **Step 3: 记录基线结论**（写进本文件末尾"执行记录"节：日期、构建产物大小、测试数）。

---

### Task 1: lint 测试骨架 + App.svelte 路由状态点修复

**Files:**
- Create: `src/lib/design-tokens.test.ts`
- Modify: `src/App.svelte:1384-1392`

- [ ] **Step 1: 写 lint 测试（含禁用 hex 框架，首批只放 App 状态点三色）**

```ts
// src/lib/design-tokens.test.ts
// 设计令牌合规 lint：扫描全部 .svelte（Settings.svelte 整体豁免，单独任务处理）。
// 规则随迁移逐批启用；每条禁用项的豁免必须写原因，目标清零。
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(process.cwd(), "src");

function listSvelte(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, name.name);
    if (name.isDirectory()) out.push(...listSvelte(p));
    else if (name.name.endsWith(".svelte")) out.push(p);
  }
  return out;
}

const FILES = listSvelte(SRC).filter((f) => !f.endsWith("Settings.svelte"));

/** 去掉注释再匹配，避免文档示例误报（如 ColorSwatch 顶部的用法注释）。 */
function stripComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/(^|\s)\/\/[^\n]*/g, "$1");
}

/** 词边界匹配，#fff 不会命中 #fff000。 */
function hasHex(content: string, hex: string): boolean {
  return new RegExp(hex + "(?![0-9a-fA-F])", "i").test(content);
}

/** 禁用 hex → 豁免文件（相对 src）。豁免必须附原因。 */
const FORBIDDEN_HEX: Array<{ hex: string; exempt: string[]; why: string }> = [
  // 旧版 success/crit 别名色（组件库文档 §5 明令禁止）
  { hex: "#34d399", exempt: ["Settings.svelte"], why: "旧 success 绿，改用 --tum-ok" },
  { hex: "#f87171", exempt: ["Settings.svelte"], why: "旧 crit 红，改用 --tum-crit" },
  // 琥珀只允许：Settings 品牌点（遗留）、ColorSwatch 文档注释
  { hex: "#fbbf24", exempt: ["Settings.svelte", "lib/components/atoms/ColorSwatch.svelte"], why: "琥珀仅限品牌点/热力图今日描边" },
];

describe("design token lint", () => {
  it("svelte 源码不含禁用 hex（注释除外）", () => {
    const hits: string[] = [];
    for (const f of FILES) {
      const rel = f.slice(SRC.length + 1).replace(/\\/g, "/");
      const code = stripComments(readFileSync(f, "utf8"));
      for (const rule of FORBIDDEN_HEX) {
        if (rule.exempt.includes(rel)) continue;
        if (hasHex(code, rule.hex)) hits.push(`${rel}: ${rule.hex} (${rule.why})`);
      }
    }
    expect(hits, hits.join("\n")).toEqual([]);
  });
});
```

- [ ] **Step 2: 跑测试确认红**

```bash
npx vitest run src/lib/design-tokens.test.ts
```

预期：FAIL，报 `App.svelte: #34d399 / #f87171 / #fbbf24`（路由状态点），无其他命中。

- [ ] **Step 3: 修 App.svelte 路由状态点**

`src/App.svelte:1384-1392`，替换为：

```css
  .router-dot--on {
    background: var(--tum-ok);
  }
  .router-dot--warn {
    background: var(--tum-warn);
  }
  .router-dot--err {
    background: var(--tum-crit);
  }
```

注：`.router-dot` 收敛到 PulseDot 原子是文档 §8 已备案的后续项，本次只修语义色（它在 22px shell 按钮内部，PulseDot 的辉光 box-shadow 会溢出按钮，不宜直接换）。

- [ ] **Step 4: 跑测试确认绿**（同 Step 2 命令，预期 PASS）

- [ ] **Step 5: 全量回归 + 提交**

```bash
npm test && npx svelte-check
git add src/lib/design-tokens.test.ts src/App.svelte
git commit -m "fix(dashboard): 路由状态点改用 ok/warn/crit 状态令牌，新增设计令牌 lint 测试"
```

---

### Task 2: tokens.css 增补 warn/crit 辅助 token

**Files:**
- Modify: `src/styles/tokens.css:43-48`（状态别名区）

- [ ] **Step 1: 在 `--tum-danger-fill` 之后追加**

```css
  /* warn/crit 面描边/填充（对齐 accent 的 fill 0.12 / stroke 0.45 惯例）。
     供告警横幅、危险确认钮 hover 等表面使用，避免再自配 rgba。 */
  --tum-warn-fill: rgba(255, 200, 61, 0.12);
  --tum-warn-stroke: rgba(255, 200, 61, 0.45);
  --tum-crit-stroke: rgba(255, 95, 86, 0.45);
```

- [ ] **Step 2: 验证**

```bash
npm run build 2>&1 | tail -8
```

预期：svelte-check 与构建通过（token 未被引用，纯增量）。

- [ ] **Step 3: 提交**

```bash
git add src/styles/tokens.css
git commit -m "feat(tokens): 增补 warn-fill/warn-stroke/crit-stroke 状态表面令牌"
```

---

### Task 3: DevicePanel 状态色收敛

**Files:**
- Modify: `src/lib/components/DevicePanel.svelte:537,541-543,607,649-658`
- Modify: `src/lib/design-tokens.test.ts`（追加规则）

- [ ] **Step 1: lint 追加禁用项（红）**

在 `FORBIDDEN_HEX` 追加：

```ts
  { hex: "#ff7a6e", exempt: [], why: "偏色红，改用 --tum-crit" },
  { hex: "#ffc77a", exempt: [], why: "偏色琥珀，改用 --tum-warn" },
  { hex: "#ff5f56", exempt: ["Settings.svelte"], why: "裸 crit 值，改用 var(--tum-crit)" },
  { hex: "#6ccb5f", exempt: ["Settings.svelte"], why: "裸 ok 值，改用 var(--tum-ok)" },
```

跑 `npx vitest run src/lib/design-tokens.test.ts`，预期 FAIL 报 DevicePanel 多处。

- [ ] **Step 2: 替换 DevicePanel 状态色**

| 位置 | 现状 | 改为 |
|------|------|------|
| :537 | `color: #ff7a6e;` | `color: var(--tum-crit);` |
| :538 | `border-color: rgba(255, 122, 110, 0.5);` | `border-color: var(--tum-crit-stroke);` |
| :541 | `color: #fff;` | 保留（实底确认钮上的白字，豁免见 Task 6 lint） |
| :607 | `color: #ff7a6e;` | `color: var(--tum-crit);` |
| :612 | `color: var(--tum-ok, #6ccb5f);` | `color: var(--tum-ok);` |
| :649 | `color: #ff7a6e;` | `color: var(--tum-crit);` |
| :651 | `background: rgba(255, 170, 60, 0.12);` | `background: var(--tum-warn-fill);` |
| :652 | `border: 1px solid rgba(255, 170, 60, 0.35);` | `border: 1px solid var(--tum-warn-stroke);` |
| :656 | `color: #ffc77a;` | `color: var(--tum-warn);` |
| :542-543 | `background/border-color: var(--tum-danger, #ff5f56);` | `var(--tum-danger)`（去 fallback，tokens.css 必加载） |

- [ ] **Step 3: 测试绿 + 回归 + 提交**

```bash
npx vitest run src/lib/design-tokens.test.ts && npm test && npx svelte-check
git add src/lib/components/DevicePanel.svelte src/lib/design-tokens.test.ts
git commit -m "fix(device): 状态色收敛到 warn/crit 令牌，去除裸 hex 与偏色变体"
```

---

### Task 4: 兜底中性色与外来色收敛（PillsOrSelect / CalendarSection / TrendLineChart / TrendWindow / App）

**Files:**
- Modify: `src/lib/components/PillsOrSelect.svelte:75,98,111-113`
- Modify: `src/lib/components/CalendarSection.svelte:164,179-180`
- Modify: `src/lib/components/TrendLineChart.svelte:58,195`
- Modify: `src/lib/components/TrendWindow.svelte:84`
- Modify: `src/App.svelte:126`
- Modify: `src/lib/design-tokens.test.ts`

- [ ] **Step 1: lint 追加（红）**

```ts
  { hex: "#8a8f98", exempt: [], why: "图例兜底灰一律写 rgb(138,143,152)" },
  { hex: "#e8eaf0", exempt: [], why: "外来文字色，改用 var(--tum-text-primary)" },
  { hex: "#23262d", exempt: [], why: "外来弹层底色，改用 var(--tum-bg-solid)" },
```

并新增 rgba 家族规则（Task 3 质量审查发现的漏网形态：DevicePanel 11 处 `rgba(232, 234, 240, α)` 与 #e8eaf0 同色相）：

```ts
/** 禁用 rgba 字面量（同色相换了形态逃避 hex 规则的）。 */
const FORBIDDEN_RGBA: Array<{ pattern: string; exempt: string[]; why: string }> = [
  { pattern: "rgba(232, 234, 240", exempt: [], why: "外来文字色 rgba 形态，按 alpha 映射到 text-muted/text-secondary" },
];
```

（测试体里对每条 pattern 做一次 `content.includes(pattern)` 检查，豁免逻辑与 hex 相同。）

预期 FAIL 列出上述 5 个文件 + DevicePanel 的命中点。

**DevicePanel rgba(232,234,240,α) 映射表**（感知量化到最近 token，渲染差异 ΔL≈8-15%，标签文字上不可感知）：

| alpha | 现值行 | 改为 |
|-------|--------|------|
| 0.5 / 0.55 / 0.6 | :618 / :424,530 / :635,646 | `var(--tum-text-muted)` |
| 0.65 / 0.7 / 0.72 / 0.75 | :557 / :493 / :503 / :443,591 | `var(--tum-text-secondary)` |
| 0.28（border-color） | :494 | `var(--tum-border-strong)` |

- [ ] **Step 2: 兜底灰统一写法**

- `src/App.svelte:126`：`const FOCUS_FALLBACK_COLOR = "#8a8f98";` → `const FOCUS_FALLBACK_COLOR = "rgb(138,143,152)";`
- `src/lib/components/TrendWindow.svelte:84`：`?? "#8a8f98"` → `?? "rgb(138,143,152)"`
- `src/lib/components/TrendLineChart.svelte:58`：`?? "#8a8f98"` → `?? "rgb(138,143,152)"`
- `src/lib/components/PillsOrSelect.svelte:75`：`?? "#8a8f98"` → `?? "rgb(138,143,152)"`（:75 的 `<i>` 色点在 Step 4 换 ColorSwatch 时一并消失）

- [ ] **Step 3: PillsOrSelect / CalendarSection 弹层色**

`PillsOrSelect.svelte`：

```css
  /* .ps（:98）*/
    color: var(--tum-text-primary);
  /* .ps option（:110-113）。原生 option 需要不透明底，故用 bg-solid 而非半透明 token */
  .ps option {
    background-color: var(--tum-bg-solid);
    color: var(--tum-text-primary);
  }
```

`CalendarSection.svelte:164,180`：`color: #e8eaf0;` → `color: var(--tum-text-primary);`（两处）
`CalendarSection.svelte:179`：`background-color: #23262d;` → `background-color: var(--tum-bg-solid);`

- [ ] **Step 4: 内联色点 → ColorSwatch**

`PillsOrSelect.svelte`：`import ColorSwatch from "./atoms/ColorSwatch.svelte";`，:75 的

```svelte
<i class="ps__dot" style={`background:${dotFor(it.id) ?? "#8a8f98"}`}></i>
```

替换为：

```svelte
<ColorSwatch color={dotFor(it.id) ?? "rgb(138,143,152)"} size={8} shape="round" />
```

并删除 `.ps__dot` 样式块（:149-155）。ColorSwatch 输出的就是 8px 圆点 `<i>`，渲染等价。

`TrendLineChart.svelte`：`import ColorSwatch from "./atoms/ColorSwatch.svelte";`，:195 的

```svelte
<i class="tl__tip-swatch" style={`background:${colors[p.id] ?? '#8a8f98'}`}></i>
```

替换为：

```svelte
<ColorSwatch color={colors[p.id] ?? "rgb(138,143,152)"} size={8} class="tl__tip-swatch" />
```

读取 `.tl__tip-swatch` 现有样式块，删掉其中的 `background` 与尺寸声明（ColorSwatch 已带），保留 margin/定位类规则。

- [ ] **Step 5: 测试绿 + 回归 + 提交**

```bash
npx vitest run src/lib/design-tokens.test.ts && npm test && npm run build 2>&1 | tail -8
git add -u
git commit -m "refactor(ui): 兜底灰统一 rgb 写法，下拉弹层与内联色点接入令牌/ColorSwatch"
```

---

### Task 5: TrendWindow / ToolWindow 标题区接入 PanelHeader，ToolWindow 选择器接入 PillsOrSelect

**Files:**
- Modify: `src/lib/components/TrendWindow.svelte`（imports、:139-152 头部、.tw__head/.tw__title CSS）
- Modify: `src/lib/components/ToolWindow.svelte`（:10 imports、:150-173 头部与 pickers、:296-330 CSS）

- [ ] **Step 1: 先跑 UI 基线截图**（本任务有可见变化，改前留档）

```bash
npm run dev   # 后台起 dev server（127.0.0.1:5173）
```

用浏览器 harness 打开 `http://127.0.0.1:5173/trend.html` 与 `/toolwindow.html`，各截一张图存 `docs/mockups/before-2026-10-06/`。（无实例运行时才启动 dev server；机器约束：UI 验证走浏览器，不点桌面坐标。）

- [ ] **Step 2: TrendWindow 头部替换**

imports 增加 `PanelHeader`（从 `./atoms`）。`<header class="tw__head">…</header>`（:139-152）替换为：

```svelte
  <PanelHeader title="用量趋势" label="用量趋势窗口">
    <RangePills
      options={RANGES}
      value={range}
      onChange={(k) => (range = k)}
      label="时间区间"
    />
    <span class="tw__stats">
      累计 <b>{formatCompact(rangeTotal)}</b> · 连续 <b>{streak}</b> 天
    </span>
  </PanelHeader>
```

删除 `.tw__head`（:215-221）与 `.tw__title`（:277-284）两个样式块；`.tw__stats` 保留（其 `flex: 1` 在 `panel-header__actions` 内继续把统计推到右侧）。

- [ ] **Step 3: ToolWindow 头部替换 + 清死导入**

imports 改为 `import { PanelHeader, RangePills, ColorSwatch, PillsOrSelect } from "./atoms";`（atoms/index.ts 需确认已导出 PillsOrSelect——它在 components/ 根而非 atoms/，因此 ToolWindow 顶部单独 `import PillsOrSelect from "./PillsOrSelect.svelte";`，与 ToolPanel 的导入方式保持一致）。

先读 `src/lib/components/ToolPanel.svelte` 里 `<PillsOrSelect` 的用法，照抄其 sentinel 约定；若 ToolPanel 无"全部"项约定，则用：

```svelte
  <PanelHeader title="工具用量" label="工具用量窗口">
    <PillsOrSelect
      items={[
        { id: "__all", label: "全部工具" },
        ...(payload?.tools ?? []).map((t) => ({ id: t.id, label: t.name })),
      ]}
      value={activeId ?? "__all"}
      onPick={(id) => (activeId = id === "__all" ? null : id)}
    />
    <RangePills
      options={RANGES}
      value={range}
      onChange={(k) => (range = k)}
      label="时间区间"
    />
    <span class="tw__stats">本区间 <b>{fmtTokens(rangeTotal)}</b></span>
  </PanelHeader>
```

整段替换 `<header class="tw__head">…</header>`（:150-173）。删除 `.tw__head`、`.tw__title`、`.tw__pickers`、`.tw__picker`、`.tw__picker:hover`、`.tw__picker.is-active` 样式块（:231-246、:293-330）。`.tw__stats` 保留。

- [ ] **Step 4: 验证**

```bash
npx svelte-check && npm test
```

预期：通过；ToolWindow 不再存在未使用导入。浏览器里两窗口标题变 10px/1.2px 字距、工具选择器变为描边胶囊（≤3 工具）或下拉（>3）。

- [ ] **Step 5: 提交**

```bash
git add src/lib/components/TrendWindow.svelte src/lib/components/ToolWindow.svelte
git commit -m "refactor(windows): 趋势/工具窗口标题区接入 PanelHeader，工具选择器改用 PillsOrSelect"
```

---

### Task 6: 值保持型字面量 token 化（字号 / 圆角 / 白面 / var fallback）

**Files:**
- Modify: `src/lib/design-tokens.test.ts`（追加 px 规则）
- Modify: DevicePanel、ToolPanel、ModelPanel、TrendPanel、TrendWindow、ToolWindow、MiniPanel、PillsOrSelect、CalendarSection、App.svelte

- [ ] **Step 1: lint 追加 px 规则（红）**

在测试文件追加：

```ts
/** 值恰好等于 token 的 px 字面量禁止直写（值保持型替换，不改布局）。
 *  token 外尺寸（8/9/12/14/15/17/10.5px 等）是有意保留的遗留，规则不匹配它们。 */
const FONT_SIZE_PX = /font-size:\s*(10|11|13)px\b/;
const RADIUS_PX = /border-radius:\s*(4|8|12|16|999)px\b/;

describe("design token lint", () => {
  it("字号 token 值不允许裸 px", () => {
    const hits = FILES.flatMap((f) => {
      const rel = f.slice(SRC.length + 1).replace(/\\/g, "/");
      const m = stripComments(readFileSync(f, "utf8")).match(new RegExp(FONT_SIZE_PX, "g"));
      return (m ?? []).map((s) => `${rel}: ${s}`);
    });
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("圆角 token 值不允许裸 px", () => {
    const hits = FILES.flatMap((f) => {
      const rel = f.slice(SRC.length + 1).replace(/\\/g, "/");
      const m = stripComments(readFileSync(f, "utf8")).match(new RegExp(RADIUS_PX, "g"));
      return (m ?? []).map((s) => `${rel}: ${s}`);
    });
    expect(hits, hits.join("\n")).toEqual([]);
  });
});
```

预期 FAIL，命中清单即本任务工作清单。

- [ ] **Step 2: 字号替换（机械，值不变）**

对下列文件执行：`font-size: 11px;` → `font-size: var(--tum-font-size-sm);`，`font-size: 10px;` → `font-size: var(--tum-font-size-xs);`，`font-size: 13px;` → `font-size: var(--tum-font-size-base);`

- DevicePanel（11px×9、10px×2、13px×2 —— 以 lint 报告为准）
- ToolPanel（11px×2、10px×1）、ModelPanel（11px×2、10px×1）
- TrendWindow（11px×4、10px×2）、ToolWindow（11px×5、10px×2）
- MiniPanel（10px×1）、PillsOrSelect（11px×1、10px×2）、CalendarSection（10px×4）
- App.svelte（11px×2、10px×3）

不匹配的 12/14/15/17/10.5/9/8px 一律不动。

- [ ] **Step 3: 圆角替换**

`border-radius: 4px;` → `var(--tum-radius-xs)`、`8px;` → `var(--tum-radius-sm)`、`12px;` → `var(--tum-radius-md)`、`999px;` → `var(--tum-radius-pill)`（DevicePanel:488）。
2/3/6/11px 不动；特例：App.svelte:1644 与 PillsOrSelect:132 的 `11px`（作用在 22px 高元素上，视觉=半高胶囊）→ `var(--tum-radius-pill)`，渲染等价。
DetailCard:541 `border-radius: 2px 2px 0 0;` 是多值简写，regex 不命中，保留。

- [ ] **Step 4: 白面 rgba 与 var fallback（DevicePanel 为主）**

DevicePanel 21 处 `var(--tum-xxx, rgba(255, 255, 255, …))` 一律去掉 fallback 写成 `var(--tum-xxx)`（tokens.css 四个窗口入口都加载，fallback 无意义且 0.12/0.06 与真实 token 值 0.16/0.085 不一致）。纯字面量按映射表替换（alpha 差 ≤0.015，不可感知）：

| 字面量 | 替换 | 说明 |
|--------|------|------|
| `rgba(255, 255, 255, 0.05)` | `var(--tum-surface)` | 0.055 |
| `rgba(255, 255, 255, 0.06)` | `var(--tum-surface)` | 0.055 |
| `rgba(255, 255, 255, 0.08)` | `var(--tum-surface-hover)` | 0.085 |
| `rgba(255, 255, 255, 0.12)`（纯背景） | `var(--tum-border-strong)` | 0.16，仅 DevicePanel:394 一处作背景使用，视觉略亮，接受 |
| 0.02 / 0.03 / 0.04 / 0.07 / 0.10 / 0.14 / 0.22 / 0.55 | 保留 | 无对应 token 的微 alpha（图表轨道、图例底、把手 grip），Task 9 记录为例外 |

- [ ] **Step 5: 测试绿 + 全量回归 + 浏览器目检**

```bash
npx vitest run src/lib/design-tokens.test.ts && npm test && npm run build 2>&1 | tail -8
```

浏览器 harness 过一遍 dashboard / trend / tools 三页，确认无布局位移。

- [ ] **Step 6: 提交**

```bash
git add -u
git commit -m "refactor(ui): 值保持型字号/圆角/白面字面量替换为设计令牌"
```

---

### Task 7: RangePills 键盘焦点态

**Files:**
- Modify: `src/lib/components/atoms/RangePills.svelte:83-85`（.pills__btn:hover 之后）

- [ ] **Step 1: 追加焦点样式**

```css
  .pills__btn:focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: 2px;
  }
```

- [ ] **Step 2: 验证 + 提交**

浏览器里用 Tab 键走到时间区间，确认出现 accent 描边（设计文档 §6 规范样式）。

```bash
npx svelte-check
git add src/lib/components/atoms/RangePills.svelte
git commit -m "fix(atoms): RangePills 补键盘焦点描边"
```

---

### Task 8: 工具系列色单一来源

**Files:**
- Create: `src/lib/series-palette.ts`
- Modify: `src/lib/components/ToolPanel.svelte:247-249`、`src/lib/components/ToolWindow.svelte:20-22`

- [ ] **Step 1: 新建共享常量**

```ts
// src/lib/series-palette.ts
// 工具用量三系列（输入/缓存/输出）的颜色唯一来源；ToolPanel 与 ToolWindow 共用，
// 防止两处色板漂移。数据系列色属设计文档允许的可编程强调色，不走 --tum-* 令牌。
export const TOOL_SERIES = [
  { key: "input", label: "输入", color: "#76a9ff" },
  { key: "cache_read", label: "缓存", color: "#ffcc66" },
  { key: "output", label: "输出", color: "#4cc2ff" },
] as const;
```

- [ ] **Step 2: 两处改为导入**（删除各自本地数组，改 `import { TOOL_SERIES } from "../series-palette";`，组件内引用点同步改名）

- [ ] **Step 3: 验证 + 提交**

```bash
npm test && npx svelte-check
git add src/lib/series-palette.ts src/lib/components/ToolPanel.svelte src/lib/components/ToolWindow.svelte
git commit -m "refactor(tools): 系列色提取为共享常量，消除面板/窗口双份定义"
```

---

### Task 8.5: DetailCard 玻璃模糊实现（前置：原型已拍板）

> 门槛：`docs/mockups/2026-10-06-detail-card-glass.html` 经 dev server 预览并由用户确认材质/半径/不透明度后才能执行。数值按下表填入用户拍板值；下文以原型默认（blur 24px、底色 0.78）示意。

**Files:**
- Modify: `src/lib/components/DetailCard.svelte:336-344`（.detail 表面）

- [ ] **Step 1: 替换 .detail 表面材质**

```css
  .detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-radius: var(--tum-radius-md);
    border: 1px solid var(--tum-border-strong);
    /* 玻璃外壳：径向强调色微光 + 半透明深底 + 背景模糊（对齐 tum-glass 语言，
       blur 半径/不透明度取原型拍板值；backdrop-filter 只采样窗内内容，
       在透明窗里对窗下桌面无效，属预期） */
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
      rgba(24, 26, 30, 0.78);
    backdrop-filter: blur(var(--tum-blur-card));
    -webkit-backdrop-filter: blur(var(--tum-blur-card));
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }
```

（若拍板值 ≠ 24px / 0.78，同步微调 tokens.css 的 `--tum-blur-card` 或此处 alpha。）

- [ ] **Step 2: 验证**

```bash
npm run dev
```

浏览器打开 `http://127.0.0.1:5173/`，悬停 Provider 卡呼出详情浮层：确认浮层下内容被柔化、文字对比度充足、无边缘撕裂/性能抖动。

- [ ] **Step 3: 提交**

```bash
git add src/lib/components/DetailCard.svelte
git commit -m "feat(detail): 悬浮详情卡兑现玻璃材质——blur(var(--tum-blur-card)) + 半透明深底"
```

---

### Task 9: 文档同步

**Files:**
- Modify: `src/styles/tokens.css:77-78`、`docs/FRONTEND-DESIGN-STYLE.md`、`docs/COMPONENT-LIBRARY.md`

- [ ] **Step 1: 校验令牌被真实引用**

确认 `--tum-blur-card` 已由 Task 8.5 落地引用（`grep -rn "tum-blur-card" src/ | grep -v tokens.css` 应命中 DetailCard.svelte）。不再删除该令牌。

- [ ] **Step 2: FRONTEND-DESIGN-STYLE.md 修订**

1. §3 中「悬浮浮层（详情卡）用 `--tum-blur-card: 24px` 更强的模糊」保持并补注一句：「blur 只采样窗内内容（透明窗对窗下桌面无效）；材质参数以 2026-10-06 原型拍板稿为准」。
2. §2.3/附录 A 增补新令牌：`--tum-warn-fill / --tum-warn-stroke / --tum-crit-stroke`（含用途注释）。
3. 附录 A（自称自包含）补齐 tokens.css 实际存在而附录缺失的内容：`--tum-glass`、`--tum-ease-spring / --tum-ease-dock-exit / --tum-ease-ring`、`--tum-scanline: none`、`#app` 规则、`html, body` 的 `-webkit-user-select: none`、文末整段 `@media (prefers-reduced-motion: reduce)` 块，并附一行注释说明它们的用途。
4. §4.5 补一句例外声明：实底按钮/品牌 logo 上的 `#fff` 白字是允许例外（DevicePanel 确认钮、ProviderLogo）。

- [ ] **Step 3: COMPONENT-LIBRARY.md 修订**

1. §7 迁移表追加两行：TrendWindow（tw__head → PanelHeader）、ToolWindow（tw__head → PanelHeader + tw__pickers → PillsOrSelect）。
2. §8 清单更新：删除已完成的项；保留 HeatmapGrid、CalendarSection（结构迁移）、Settings、App pill-breathe；新增一行「剩余 token 外字号（8/9/12/14/15/17px）与微 alpha 白面（0.02-0.14）为已记录的遗留，勿在新代码模仿」。
3. §6 checklist「键盘可达」条目后追加：`npx vitest run src/lib/design-tokens.test.ts` 是硬门禁，禁 hex / 裸 token 值 px 会被 CI 拦截。

- [ ] **Step 4: 验证 + 提交**

```bash
npm run build 2>&1 | tail -8 && npm test
git add src/styles/tokens.css docs/FRONTEND-DESIGN-STYLE.md docs/COMPONENT-LIBRARY.md
git commit -m "docs: 设计文档与令牌实现对齐——补附录A缺口、修悬浮层表述、记录遗留例外"
```

---

### Task 10: 终验

**Files:** 无改动（如有微调，单独提交）

- [ ] **Step 1: 全量检查**

```bash
npm run build 2>&1 | tail -20 && npm test 2>&1 | tail -6
```

预期：svelte-check 零错误、vitest 全绿、dashboard 产物体积不高于 Task 0 基线（±0.5 kB 内浮动可接受，本计划净变化预期为负）。

- [ ] **Step 2: 浏览器 harness 逐窗目检**（dev server 下）

- `http://127.0.0.1:5173/`（dashboard：路由状态点颜色、DevicePanel 告警横幅、悬停浮层玻璃模糊与文字对比度）
- `/trend.html`、`/toolwindow.html`（标题区、工具选择器、Tab 键焦点描边）
- `/settings.html`（确认未误伤——本计划不改 Settings）

peek 把手无代码改动，跳过（其样式由 Tauri 窗口几何驱动，浏览器里无法真实呈现）。

- [ ] **Step 3: 收尾**

在 `docs/COMPONENT-LIBRARY.md` §8 下追加一行执行记录（日期 + 本次清理量），与本计划的"执行记录"互链。确认 git log 每个提交均绿色。

---

## 自审记录（计划完成时已核）

1. **覆盖度**：审计报告的修复优先级 1-4 全部有对应任务（1→Task 1、2→Task 2/3、3→Task 5/9、4→Task 4/7）；Settings/HeatmapGrid/CalendarSection 结构迁移明确划出范围并有文档落点。审计中"tooltip 数字非等宽"一条经复核不成立（`.tl__tip` 容器已有 `var(--tum-font-mono)`），已从计划剔除。
2. **无占位**：所有替换均给出前/后代码或精确映射表；两处"先读再抄"（ToolPanel 的 PillsOrSelect 约定、`.tl__tip-swatch` 现有 CSS）是刻意的防漂移步骤，不是缺信息。
3. **类型一致性**：`TOOL_SERIES` 在 Task 8 定义并被 ToolPanel/ToolWindow 同名引用；lint 的 `FORBIDDEN_HEX / FONT_SIZE_PX / RADIUS_PX` 在 Task 1/3/4/6 增量扩展，豁免结构一致。
