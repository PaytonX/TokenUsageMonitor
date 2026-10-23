# glance P0 落地实施计划

> **REQUIRED SUB-SKILL**：逐步执行前先加载 `verification-before-completion`；每个 Step 的
> 「验收」命令输出确认后才允许 commit。推荐用 `subagent-driven-development` 执行本计划
> （每 Step 一个子代理 + 独立 review），也可 inline 逐步执行。

## 元信息

- **分支**：`feat/glance-p0-polish`（已建，自 `fix/usage-attribution-and-cache-doublecount` HEAD 切出）
- **已提交**：a89fe23 = docs/glance-design-research.md；be8bbbe = docs/glance-p0-polish-design.md（用户已批设计）
- **来源设计**：docs/glance-p0-polish-design.md 七项 P0；本计划把设计落成可逐步执行的代码变更
- **commit 顺序**：Step1 → Step7 → Step2 → Step6 → Step5 → Step3 → Step4（图标早落座，4 最险殿后）
- **每 commit 门禁**：`npm test` 全绿 + `npx svelte-check` **0 error 0 warning**；Step 4 另加 `cd src-tauri && cargo test`
- **依赖纪律**：全程不新增 npm / cargo 依赖
- **行号基准**：本文行号在 be8bbbe 时点核实；执行时若有 ±几行漂移，**以内容锚点（引用的代码串）为准**，先 read 后 edit

## 成功标准（全部满足才算完成）

1. 非 Settings 豁免区动效时长/曲线字面量清零：CSS 走 `var(--tum-*)`；JS 定时器走 `src/lib/motion/tokens.ts` 单点桥接。
2. Settings.svelte crit/warn/ok 色系外来 hex（#f87171/#ff6b6b/#ef4444/#34d399/#22c55e/#f59e0b/#fbbf24）清零；`#ef4444`、`#ff6b6b` 加入 design-tokens.test.ts 零豁免黑名单。
3. 全仓字符按钮（↻ ⚙ ⤢ ✕ × — ▢）0 残留，统一走 `src/lib/components/atoms/Icon.svelte`（Lucide 子集手嵌，零依赖）。
4. 设置窗有未保存草稿时，头部 ✕ / Esc 必须先弹内联确认条；页脚「保存并关闭」保持直通；确认条可 Esc 取消。
5. tokens.css 全局 `:where(:focus-visible)` 兜底；ToolWindow / TrendWindow 支持 Esc 关窗。
6. `src/lib/motion/num.ts` 非对称数字补间纯函数 + vitest 直测（注入时钟）；Stat.svelte 支持可选动画模式。
7. Rust 侧 `approach_step` 纯函数 + cargo 单测；拖拽落位 / 把手跟随从瞬移改为指数趋近滑动（8ms 步进、τ=60ms、<0.5px 收手、代际防重入）。
8. 每个 commit 的验收输出（测试摘要）记入 commit message 尾注。

---

## Step 1 — 动效 token 收口 + 字号归阶梯 + crit 色系清理（执行顺序 #1）

**目的**（glance 学习点：全项目仅两组曲线、一切时长皆 token，改一处全站生效）。

**文件**：`src/tokens.css`、`src/App.svelte`、`src/lib/components/MiniPanel.svelte`、
`src/lib/components/ProgressRing.svelte`、`src/lib/components/PillsOrSelect.svelte`、
`src/lib/components/RangePills.svelte`、`src/lib/components/atoms/ZoomButton.svelte`、
`src/Settings.svelte`、`src/lib/design-tokens.test.ts`、新增 `src/lib/motion/tokens.ts`。

### 1a. tokens.css 新增三枚 token

在 `:root` 内既有 `--tum-dur-*` 组末尾（`--tum-dur-reduced-fade: 120ms;` 之后）追加：

```css
  /* glance 学习：四次幂 easeOut（glance 内部形变统一曲线 1-(1-t)^4）。
     比 spring 少回弹、收尾更收敛——用于落位/滑回类「到位」动画。 */
  --tum-ease-out-quart: cubic-bezier(0.25, 1, 0.5, 1);
  /* glance 学习：拖拽落位、关闭确认条等「到位」动画统一时长。 */
  --tum-dur-settle: 200ms;
  /* 420ms 圆环/进度推进曲线的时长 token（值来自 MiniPanel/ProgressRing 既有字面量）。 */
  --tum-dur-ring: 420ms;
```

### 1b. 新建 `src/lib/motion/tokens.ts`

```ts
/**
 * JS 定时器 ← CSS 时长 token 的单点桥接。
 *
 * JS 侧的 setTimeout/rAF 节拍无法直接消费 CSS 变量；把「读 token + fallback」
 * 收在这一个函数里，token 改名/改值时 JS 自动跟上，不再出现双份魔法数。
 * SSR / 测试环境（无 window）以及 CSS 未加载时回退 fallback（fallback 值
 * 必须与 tokens.css 当前值一致，改动 token 时同步改这里）。
 */
export function durMs(name: string, fallback: number): number {
  if (typeof window === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const ms = Number.parseFloat(raw);
  return Number.isFinite(ms) && ms > 0 ? ms : fallback;
}
```

### 1c. App.svelte 胶囊节拍接 token

先 read `src/App.svelte:320-335` 确认现状，然后：

- import 区加入：`import { durMs } from "./lib/motion/tokens";`
- `const PILL_SLIDE_MS = 180;`（:327 附近）→ `const PILL_SLIDE_MS = durMs("--tum-dur-slide-out", 180);`
- `const PILL_PEEK_LEAD_MS = 120;` → `const PILL_PEEK_LEAD_MS = durMs("--tum-dur-instant", 120);`
- `const PILL_REVEAL_MINI_MS = 200;` → `const PILL_REVEAL_MINI_MS = durMs("--tum-dur-base", 200);`
- `PILL_DOCK_DELAY_MS = 320` 保持不变（编排常量，无对应 CSS 过渡）。

### 1d. CSS 字面量 → token（逐处 old → new）

| 文件:行 | 旧 | 新 |
| --- | --- | --- |
| MiniPanel.svelte:112 | `transition: width 420ms var(--tum-ease-ring);` | `transition: width var(--tum-dur-ring, 420ms) var(--tum-ease-ring);` |
| ProgressRing.svelte:201 | `transition: stroke-dashoffset 420ms var(--tum-ease-ring);` | `transition: stroke-dashoffset var(--tum-dur-ring, 420ms) var(--tum-ease-ring);` |
| PillsOrSelect.svelte:140 | `transition: all 0.15s ease;` | `transition: all var(--tum-dur-fast, 150ms) var(--tum-ease-spring);` |
| RangePills.svelte:80-82 | `transition: background 0.2s ease, color 0.2s ease;` | `transition: background var(--tum-dur-base, 200ms) var(--tum-ease-spring), color var(--tum-dur-base, 200ms) var(--tum-ease-spring);` |
| ZoomButton.svelte:51 | `transition: all 0.15s ease;` | `transition: all var(--tum-dur-fast, 150ms) var(--tum-ease-spring);` |

### 1e. Settings.svelte crit/warn/ok 色系清理

先 `grep -n "#f87171\|#ff6b6b\|#ef4444\|#34d399\|#22c55e\|#f59e0b\|#fbbf24" src/Settings.svelte`，
已知命中（be8bbbe 时点）：795-797（script 常量）、2702、3639/3640/3642/3647、3695、
3797/3807/3862/3868、3860-3862。逐处 read ±3 行后按映射替换：

- 纯 color 值：`#ff6b6b` `#f87171` `#ef4444` → `var(--tum-crit)`；`#f59e0b` `#fbbf24` → `var(--tum-warn)`；`#34d399` `#22c55e` → `var(--tum-ok)`。
- script 里的字符串常量（795-797 判级色）：同映射，直接把字符串换成 `"var(--tum-crit)"` 等（它们流入 inline style，CSS 变量在 inline style 中同样生效）。
- `rgba(248,113,113,x)` 类半透明形态 → `color-mix(in srgb, var(--tum-crit) <x*100>%, transparent)`（WebView2 Chromium ≥120 支持）。
- **明确不动**：3392/3399 `#0a0e1a`、3582 `#8b949e`（中性灰阶，语义映射属 P1，避免误伤）；组件 props 默认 `accent="#4cc2ff"`（属可配置默认值非越界色）。

### 1f. 越界字号归阶梯

`grep -n "font-size: [0-9]*\.5px" src/Settings.svelte src/App.svelte`，按映射逐处替换
（已知 21 处：App.svelte:2167；Settings.svelte 2629/3544/3547/3553/3568 附近/3599/3609/
3627/3647/3662/3690/3730/3734/3791 等）：

| 字面量 | 替换为 |
| --- | --- |
| `10.5px` | `var(--tum-font-size-xs)` |
| `11.5px` | `var(--tum-font-size-sm)` |
| `12.5px` / `13.5px` | `var(--tum-font-size-base)` |
| `15.5px` | `var(--tum-font-size-lg)` |

App.svelte:2167 `.pill__badge` 的 `font-size: 10.5px;` → `var(--tum-font-size-xs)`。

### 1g. design-tokens.test.ts 收紧黑名单

在 FORBIDDEN_HEX 数组中加入 `"#ef4444"` 与 `"#ff6b6b"`（本次已清零，可入零豁免名单）。
Settings.svelte 的整体豁免 **保持不动**（中性灰/字号残留仍在豁免区）。

### 验收

```bash
npm test        # 含 design-tokens lint，全绿
npx svelte-check   # 0 error 0 warning
```

### Commit

```
refactor(tokens): 动效/字号/crit 色系收口 design tokens（glance P0-1）

- 新增 --tum-ease-out-quart / --tum-dur-settle / --tum-dur-ring
- JS 胶囊节拍经 src/lib/motion/tokens.ts 单点读 token
- MiniPanel/ProgressRing/PillsOrSelect/RangePills/ZoomButton 字面量清零
- Settings crit/warn/ok 色系外来 hex 清零；#ef4444/#ff6b6b 入黑名单
- 越界字号（*.5px）归阶梯 var(--tum-font-size-*)
- 验收：npm test 全绿；svelte-check 0E0W
```

---

## Step 7 — 图标体系 Icon.svelte（执行顺序 #2）

**目的**（glance 学习点：内置矢量图标体系，字符字形跨字体渲染不可控、无障碍语义弱）。

**文件**：新增 `src/lib/components/atoms/Icon.svelte`；替换下列全部字符按钮。

### 7a. 新建 `src/lib/components/atoms/Icon.svelte`

```svelte
<!--
  Icon — 内联 SVG 图标（Lucide 子集，零依赖）
  ─────────────────────────────────────────────────────────────
  图形拷贝自 Lucide (https://lucide.dev)，ISC License。手内嵌 path 以保持
  零依赖；viewBox 24 / stroke 2 / currentColor——颜色跟随文字色，大小由
  size 控制。aria-hidden：图标自身不参与无障碍树，语义由按钮的
  aria-label / title 承担（与字符字形时期一致）。

  新增图标：在 PATHS 中加 name → path 片段（多 path 用多个元素拼接）。
-->
<script lang="ts">
  export type IconName =
    | "x"
    | "minus"
    | "square"
    | "refresh"
    | "settings"
    | "maximize";

  const PATHS: Record<IconName, string> = {
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    minus: '<path d="M5 12h14"/>',
    square: '<rect width="18" height="18" x="3" y="3" rx="2"/>',
    refresh:
      '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    settings:
      '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
    maximize:
      '<polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/>',
  };

  interface Props {
    name: IconName;
    /** 渲染边长（px）。默认 14，与原字符字形视觉尺寸对齐。 */
    size?: number;
  }

  let { name, size = 14 }: Props = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <!-- PATHS 是文件内静态常量，非用户输入，@html 安全 -->
  {@html PATHS[name]}
</svg>
```

### 7b. 替换清单（每处先 read 原行，再整串替换；import 逐文件添加）

各文件 import：`import Icon from "./atoms/Icon.svelte";`（或按相对层级
`import Icon from "../atoms/Icon.svelte";` / `import Icon from "$lib/..."`——与该文件既有
atoms 导入风格一致，Stat/ColorSwatch 怎么引它就怎么引）。

| 位置 | 旧（字形所在串） | 新 |
| --- | --- | --- |
| App.svelte:1477 | `>✕</button>`（.pill__close） | `<Icon name="x" size={11} /></button>` |
| App.svelte:1512 | `>↻</button>`（.shell__btn，refresh） | `<Icon name="refresh" size={13} />` |
| App.svelte:1527 | `>⚙</button>`（.shell__btn，settings） | `<Icon name="settings" size={13} />` |
| App.svelte:1528 | `>⤢</button>`（.shell__btn，collapseToMini） | `<Icon name="maximize" size={13} />` |
| App.svelte:1529 | `>×</button>`（.shell__btn--close） | `<Icon name="x" size={13} />` |
| Settings.svelte:1171 | `>—</button>`（.settings__win-btn 最小化） | `<Icon name="minus" size={12} />` |
| Settings.svelte:1172 | `>▢</button>`（.settings__win-btn 最大化） | `<Icon name="square" size={12} />` |
| Settings.svelte:1174 | `✕`（.settings__close 多行按钮体） | `<Icon name="x" size={12} />` |
| Settings.svelte:1672 | `>×</button>`（.preset-picker__close） | `<Icon name="x" size={12} /></button>` |
| Settings.svelte:1929 | `>×</button>`（移除数据根） | `<Icon name="x" size={12} /></button>` |
| Settings.svelte:2177 | `>✕</button>`（.rt-op--del 删行） | `<Icon name="x" size={11} /></button>` |
| ToolWindow.svelte:143 | `>—</button>` | `<Icon name="minus" size={12} /></button>` |
| ToolWindow.svelte:144 | `>▢</button>` | `<Icon name="square" size={12} /></button>` |
| ToolWindow.svelte:145 | `>✕</button>` | `<Icon name="x" size={12} /></button>` |
| TrendWindow.svelte:141 | `>—</button>` | `<Icon name="minus" size={12} /></button>` |
| TrendWindow.svelte:142 | `>▢</button>` | `<Icon name="square" size={12} /></button>` |
| TrendWindow.svelte:143 | `>✕</button>` | `<Icon name="x" size={12} /></button>` |
| DevicePanel.svelte:216 | `>↻</button>`（.dev__refresh） | `<Icon name="refresh" size={13} /></button>` |
| DevicePanel.svelte:284 | `>×</button>`（.dev__remove） | `<Icon name="x" size={12} /></button>` |
| DetailCard.svelte:339 | `>↻ {$t("detail.refreshThis")}</button>` | `<Icon name="refresh" size={13} /> {$t("detail.refreshThis")}</button>` |
| ModelPanel.svelte:425 | `>↻</button>`（.mp__refresh） | `<Icon name="refresh" size={13} /></button>` |
| ToolPanel.svelte:263 | `>↻</button>`（.tool__refresh） | `<Icon name="refresh" size={13} /></button>` |
| ZoomButton.svelte:33 | `>⤢</button>` | `<Icon name="maximize" size={12} /></button>` |

ZoomButton.svelte 额外清理：`glyph` prop（:14-24 中的 `glyph?: string` 与解构默认值
`glyph = "⤢"`）删除——全仓无调用方传它（grep 核实）；样式 :47-48 的
`font-family: var(--tum-font-mono);` 与 `font-size: var(--tum-font-size-sm);` 随字形
一并删除。App.svelte:1525 的 `⇄`（router 状态点旁）**保留**——它是状态指示符的一部分，
不是按钮字形。

### 验收

```bash
grep -n "↻\|⚙\|⤢\|✕\|×\|—\|▢" src/App.svelte src/Settings.svelte src/lib/components/*.svelte src/lib/components/atoms/*.svelte
# 预期仅剩：App.svelte:1525 ⇄（保留）、DetailCard 文案内的 ✗（Settings.svelte:1120 formatTestResult 的文本前缀，保留——它是文字内容不是按钮字形）
npm test && npx svelte-check
```

### Commit

```
refactor(icons): 字符按钮统一替换为内嵌 Lucide SVG Icon 组件（glance P0-7）

- 新增 atoms/Icon.svelte：x/minus/square/refresh/settings/maximize 六枚，
  Lucide ISC 手嵌，viewBox24/stroke2/currentColor，aria-hidden
- 替换 App/Settings/ToolWindow/TrendWindow/DevicePanel/DetailCard/
  ModelPanel/ToolPanel/ZoomButton 全部字符按钮（23 处）
- ZoomButton 移除已无调用方的 glyph prop 与字形专用样式
- 保留 App.svelte:1525 ⇄（状态指示符）与 formatTestResult 文本前缀 ✗
- 验收：grep 字形仅剩两处白名单；npm test 全绿；svelte-check 0E0W
```

---

## Step 2 — 微交互：hover 减淡 + 按压反馈（执行顺序 #3）

**目的**（glance 学习点：hover 是「轻微回应」不是「高亮爆炸」；可按的都要有按压位移）。

**文件**：`src/tokens.css`；`.shell__btn`（App.svelte 样式区）、`.pill__close`
（App.svelte:2194-2197）、`.settings__win-btn` / `.settings__close`（Settings.svelte）、
`.tw__bar-btn`（ToolWindow.svelte / TrendWindow.svelte 各自样式区）、`.zoom-btn`
（ZoomButton.svelte:36-52）、`.rt-op`（Settings.svelte）、`.dev__refresh`（DevicePanel.svelte）、
`.mp__refresh`（ModelPanel.svelte）、`.tool__refresh`（ToolPanel.svelte）。

### 2a. tokens.css

`--tum-surface-hover: rgba(255, 255, 255, 0.085);`（:23 附近，空格以 read 为准）→
`--tum-surface-hover: rgba(255, 255, 255, 0.06);`
（注释同行补充：`/* glance：hover 是轻微回应（INK 55% 级），减淡一档 */`）

### 2b. 按压反馈（统一模式）

对上列每个按钮类，在其样式块内：

1. 若已有 `transition:` 声明 → 向列表追加一段 `transform var(--tum-dur-fast, 150ms) var(--tum-ease-spring)`（注意原列表尾逗号）；
   若没有 → 新增一行 `transition: transform var(--tum-dur-fast, 150ms) var(--tum-ease-spring);`
2. 追加按压规则（小按钮 0.94、.settings__footer 等大按钮不用加——本步只覆盖图标/窗控钮）：

```css
  .<该类>:active {
    transform: scale(0.94);
  }
```

例外：`.pill__close` 用 0.9（18px 小圆钮，比例感更强）；`.zoom-btn` 在 Step 1 已改为
`transition: all`（覆盖 transform），只需追加 `:active` 规则。

已知锚点：App.svelte `.pill__close` 的 transition 列表在 :2194-2197（三段 opacity/
background/color），追加第四段 transform；`.shell__btn` 样式块先 read 定位。

### 2c. 明确不做（记录进 commit message）

- **分段控件 thumb 滑移**：全仓不存在 thumb 结构（PillsOrSelect/RangePills 是逐 pill
  变色），设计中的 thumb 项如实收窄——时长 token 化已在 Step 1 完成。
- **glance 悬浮钮 LINGER 500ms**：不适用——本项目没有「鼠标穿透后驻留恢复」表面
  （docked 胶囊走 strip-visible 显隐，已在 hover 状态机内）。

### 验收

```bash
npx svelte-check && npm test
# 手动冒烟（可选，不阻塞）：npm run tauri:dev，指针悬停/按压 header 图标钮
```

### Commit

```
feat(interaction): hover 减淡一档 + 图标钮按压 scale 反馈（glance P0-2）

- --tum-surface-hover 0.085 → 0.06（glance INK 55% 级「轻微回应」）
- shell__btn/pill__close/settings__win-btn/settings__close/tw__bar-btn/
  zoom-btn/rt-op/dev__refresh/mp__refresh/tool__refresh 增加 :active scale
- thumb 滑移不适用（无 thumb 结构）；LINGER 不适用（无穿透恢复表面）
- 验收：svelte-check 0E0W；npm test 全绿
```

---

## Step 6 — 全局焦点环兜底 + 子窗口 Esc 关闭（执行顺序 #4）

**目的**（glance 学习点：设置全键盘可操作；焦点环只在键盘导航时出现，全局兜底零特异性）。

**文件**：`src/tokens.css`、`src/lib/components/ToolWindow.svelte`、`src/lib/components/TrendWindow.svelte`。

### 6a. tokens.css 追加（文件末尾 `prefers-reduced-motion` 块之前）

```css
  /* glance 学习：键盘可达性全局兜底。:where() 特异性为 0，组件自带的
     :focus-visible 样式（.pill__close、.zoom-btn 等）自然胜出；这里只兜住
     没有组件级焦点样式的裸按钮/链接。 */
  :where(button, a, select, input, textarea, [tabindex]):focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: 2px;
  }
```

### 6b. ToolWindow.svelte

script 内（`function onDrag(...)` 之后、`</script>` 之前）加：

```ts
  // glance 学习：子窗口也要能键盘关掉。Esc 关窗；原生 select 弹层打开时
  // keydown 由弹层消费，不会误触这里。
  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }
```

`<div class="tw">` 之前（script 结束标签后）加：

```svelte

<svelte:window onkeydown={onWindowKeydown} />
```

### 6c. TrendWindow.svelte

同 6b（函数体一致；两文件各自 close() 已存在：ToolWindow:124 / TrendWindow:122）。

### 验收

```bash
npx svelte-check && npm test
# 手动冒烟：Tab 键遍历主面板可见焦点环；ToolWindow/TrendWindow 按 Esc 关闭
```

### Commit

```
feat(a11y): 全局 :focus-visible 零特异性兜底 + 子窗口 Esc 关闭（glance P0-6）

- tokens.css :where(button,a,select,input,textarea,[tabindex]):focus-visible
  2px accent 环，组件级样式自然覆盖
- ToolWindow/TrendWindow 新增 svelte:window Esc → close()
- 验收：svelte-check 0E0W；npm test 全绿
```

---

## Step 5 — 设置关闭拦截（执行顺序 #5）

**目的**（glance 学习点：有未保存状态时关闭必须先问，且用行内确认条而非系统弹窗）。

**文件**：`src/Settings.svelte`、`src/lib/i18n/zh-CN.ts`、`src/lib/i18n/en.ts`。

### 5a. 状态与入口（Settings.svelte script）

`async function handleClose()`（:1110-1115）**保持原样**（页脚「保存并关闭」直通它）。
其上方插入：

```ts
  // glance 学习：有未保存草稿时关闭必须先问。头部 ✕ 与 Esc 走 requestClose；
  // 页脚「保存并关闭」是显式意图，保持直通 handleClose。
  let closeConfirm = $state(false);

  function requestClose() {
    if (hasDraft && !closeConfirm) {
      closeConfirm = true;
      return;
    }
    closeConfirm = false;
    void handleClose();
  }

  async function confirmCloseSave() {
    closeConfirm = false;
    await handleClose();
  }

  async function confirmCloseDiscard() {
    closeConfirm = false;
    await handleDiscard();
    await closeSettings();
  }

  // 确认条出现时焦点落主按钮（键盘直达「保存并关闭」）。
  function autofocus(node: HTMLElement) {
    node.focus();
  }
```

（`hasDraft` 为既有 `$derived`，:221 附近；`handleDiscard` :1090；`closeSettings` 来自
`src/lib/api.ts:212`，该文件已 import。）

### 5b. Esc 处理扩展（:1155-1162）

旧：

```svelte
<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && (pickerOpen || presetMenuFor)) {
      pickerOpen = false;
      presetMenuFor = null;
    }
  }}
/>
```

新：

```svelte
<svelte:window
  onkeydown={(e) => {
    if (e.key !== "Escape") return;
    if (pickerOpen || presetMenuFor) {
      pickerOpen = false;
      presetMenuFor = null;
      return;
    }
    if (closeConfirm) {
      closeConfirm = false;
      return;
    }
    if (hasDraft) requestClose();
  }}
/>
```

（无草稿时 Esc 不关窗——与既有行为一致，不扩大改动面。）

### 5c. 头部 ✕ 按钮改入口 + aria 修正（:1173-1175）

旧：

```svelte
      <button class="settings__close" onclick={handleClose} aria-label={$t("settings.footer.saveClose")}>
        <Icon name="x" size={12} />
      </button>
```

新：

```svelte
      <button class="settings__close" onclick={requestClose} aria-label={$t("common.close")}>
        <Icon name="x" size={12} />
      </button>
```

（按钮体在 Step 7 已换 Icon；此处只改 onclick 与 aria-label——原 aria 误用了页脚文案。）

### 5d. 确认条 markup（插在 conflict-warn `{/if}` 之后、`<footer>` 之前，:2375 附近）

```svelte
  {#if closeConfirm}
    <div
      class="close-confirm"
      role="alertdialog"
      aria-label={$t("settings.closeConfirm.title")}
    >
      <span class="close-confirm__text">{$t("settings.closeConfirm.body")}</span>
      <span class="close-confirm__actions">
        <button
          type="button"
          class="btn btn--primary"
          use:autofocus
          disabled={saving || busy !== null}
          onclick={confirmCloseSave}
        >{$t("settings.closeConfirm.saveClose")}</button>
        <button
          type="button"
          class="btn btn--ghost"
          disabled={saving || busy !== null}
          onclick={confirmCloseDiscard}
        >{$t("settings.closeConfirm.discardClose")}</button>
        <button
          type="button"
          class="btn btn--ghost"
          onclick={() => (closeConfirm = false)}
        >{$t("common.cancel")}</button>
      </span>
    </div>
  {/if}
```

### 5e. 确认条样式（追加到 Settings.svelte `.conflict-warn` 样式块之后）

```css
  .close-confirm {
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
    margin: 0 var(--tum-space-4) var(--tum-space-3);
    padding: 10px 14px;
    border: 1px solid var(--tum-warn);
    border-radius: var(--tum-radius-md);
    background: color-mix(in srgb, var(--tum-warn) 12%, transparent);
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
  }

  .close-confirm__actions {
    display: inline-flex;
    gap: var(--tum-space-2);
    margin-left: auto;
  }
```

（若 `--tum-space-*` / `--tum-radius-md` 名称与 tokens.css 实际不符——执行时先 read
tokens.css 核对，用真实存在的同类 token，禁造新 token。）

### 5f. i18n（zh-CN.ts footer 块 :76 之后插入；en.ts :74 之后对应插入）

zh-CN.ts：

```ts
  "settings.closeConfirm.title": "有未保存的更改",
  "settings.closeConfirm.body": "设置还没有保存，关闭会丢失这些改动。",
  "settings.closeConfirm.saveClose": "保存并关闭",
  "settings.closeConfirm.discardClose": "放弃并关闭",
```

en.ts：

```ts
  "settings.closeConfirm.title": "Unsaved changes",
  "settings.closeConfirm.body":
    "Your settings haven't been saved yet — closing will lose these changes.",
  "settings.closeConfirm.saveClose": "Save and close",
  "settings.closeConfirm.discardClose": "Discard and close",
```

### 验收

```bash
npm test    # dicts.test.ts 奇偶校验必须过
npx svelte-check
# 手动冒烟：改一项设置 → 头部 ✕ 出确认条；Esc 取消；「放弃并关闭」丢弃并关窗；
#           无草稿时 ✕ 直接关；页脚「保存并关闭」始终直通
```

### Commit

```
feat(settings): 有草稿关闭先确认——内联确认条替代静默保存（glance P0-5）

- requestClose 门控：hasDraft && !closeConfirm 时 ✕/Esc 弹确认条
- 三选：保存并关闭（直通 handleClose）/ 放弃并关闭（handleDiscard+close）/
  取消；Esc 可取消确认条
- 头部 ✕ aria-label 修正 common.close；页脚「保存并关闭」保持直通
- i18n zh/en 四键，dicts 奇偶校验通过
- 验收：npm test 全绿；svelte-check 0E0W
```

---

## Step 3 — 数字补间（执行顺序 #6）

**目的**（glance 学习点：频谱「上升瞬时、下降限速」——画显示值而非瞬时值，数字不砸下来）。

**文件**：新增 `src/lib/motion/num.ts` + `src/lib/motion/num.test.ts`；改
`src/lib/components/atoms/Stat.svelte`。

### 3a. 新建 `src/lib/motion/num.ts`

```ts
/**
 * 非对称数字补间 —— glance 频谱条思路的前端移植。
 *
 * 上升（数值变大）：指数趋近 τ=60ms（glance 滚动 τ），视觉上「立即到位」
 * 但保留一丝惯性感；下降（数值变小）：线性限速（每秒最多 fallLimitPerSec），
 * 防止用量清零时数字「砸」下来——额度消耗是慢过程，回撤更要慢。
 *
 * 纯函数 + 显式时钟：rAF 只负责驱动，vitest 注入假时钟直测。
 * 首步（updatedAt 为 null）直接贴住目标——挂载时显示真实值，不从 0 爬。
 */

export interface TweenConfig {
  /** 上升指数趋近时间常数（ms）。glance：60。 */
  riseTauMs?: number;
  /** 下降线性限速（单位/秒）。按调用点语义配置（如 25 tokens/s）。 */
  fallLimitPerSec?: number;
}

const DEFAULTS: Required<TweenConfig> = {
  riseTauMs: 60,
  fallLimitPerSec: 40,
};

export function createTween(initial = 0, config: TweenConfig = {}) {
  const cfg = { ...DEFAULTS, ...config };
  let value = initial;
  let updatedAt: number | null = null;

  /** 推进一步。now：毫秒时间戳；target：目标值。返回新的显示值。 */
  function step(target: number, now: number): number {
    const dt = updatedAt === null ? 0 : Math.max(0, now - updatedAt);
    updatedAt = now;
    if (dt === 0) {
      value = target;
      return value;
    }
    if (target >= value) {
      const alpha = 1 - Math.exp(-dt / cfg.riseTauMs);
      const next = value + (target - value) * alpha;
      // 末段吸附：差值 < 0.01 视为到位，避免无限小数长尾
      value = target - next < 0.01 ? target : next;
    } else {
      const maxDelta = (cfg.fallLimitPerSec * dt) / 1000;
      value = Math.max(target, value - maxDelta);
    }
    return value;
  }

  return {
    step,
    /** 当前显示值。 */
    get value() {
      return value;
    },
    /** 硬置显示值（时钟不动，下一步正常补间）。 */
    reset(v: number) {
      value = v;
    },
  };
}
```

### 3b. 新建 `src/lib/motion/num.test.ts`

```ts
import { describe, expect, it } from "vitest";
import { createTween } from "./num";

describe("createTween", () => {
  it("首步直接贴住目标（挂载不从 0 爬）", () => {
    const tw = createTween(0);
    expect(tw.step(100, 1000)).toBe(100);
  });

  it("上升按 τ=60ms 指数趋近", () => {
    const tw = createTween(0);
    tw.step(0, 0); // 首步贴 0
    const v1 = tw.step(100, 60); // dt=60 → alpha = 1 - e^(-1) ≈ 0.6321
    expect(v1).toBeCloseTo(100 * (1 - Math.exp(-1)), 2);
    const v2 = tw.step(100, 120); // 从 63.21 继续
    expect(v2).toBeCloseTo(100 - 100 * Math.exp(-2), 1);
  });

  it("上升末段吸附到目标（无小数长尾）", () => {
    const tw = createTween(0);
    tw.step(0, 0);
    let v = 0;
    for (let i = 1; i <= 600; i++) v = tw.step(100, i * 1);
    expect(v).toBe(100);
  });

  it("下降被线性限速（每秒最多 fallLimitPerSec）", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    const v = tw.step(0, 60); // dt=60ms → 最多降 40*0.06 = 2.4
    expect(v).toBeCloseTo(97.6, 6);
  });

  it("下降最终收敛到目标且不越过", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    let v = 100;
    let now = 0;
    while (v !== 0 && now < 60_000) {
      now += 16;
      v = tw.step(0, now);
    }
    expect(v).toBe(0);
  });

  it("数值来回摆动：升降各自遵循自己的曲线", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.step(100, 60); // 上升：指数
    const down = tw.step(0, 120); // 下降：限速 40*0.06=2.4/s 段
    expect(down).toBeCloseTo(100 - 2.4, 6);
  });
});
```

### 3c. Stat.svelte 支持可选动画模式

script 段替换为：

```svelte
<script lang="ts">
  import type { Snippet } from "svelte";
  import { createTween } from "../../motion/num";

  interface Props {
    value: string | number;
    label?: string;
    /** value 后挂单位（"天" / "%" 等），等宽渲染避免抖动。 */
    suffix?: string;
    /** 提供时对该数字做非对称补间（上升瞬时/下降限速），value 被忽略。 */
    animate?: number;
    /** animate 模式下的格式化函数；缺省四舍五入为整数。 */
    format?: (n: number) => string;
    /** 容器用，把多个 Stat 横排成一行。 */
    children?: Snippet;
  }

  let { value, label, suffix, animate, format, children }: Props = $props();

  const prefersReduced =
    typeof matchMedia !== "undefined"
      ? matchMedia("(prefers-reduced-motion: reduce)")
      : null;

  let shown = $state(animate ?? 0);
  const tween = createTween(animate ?? 0, { fallLimitPerSec: 25 });

  $effect(() => {
    if (animate === undefined) return;
    if (prefersReduced?.matches) {
      shown = animate;
      return;
    }
    let raf = requestAnimationFrame(function frame(now) {
      const v = tween.step(animate, now);
      shown = v;
      if (v !== animate) raf = requestAnimationFrame(frame);
    });
    return () => cancelAnimationFrame(raf);
  });
</script>
```

value 渲染行（.stat__value 内）改为：

```svelte
  <b class="stat__value"
    >{animate !== undefined
      ? format
        ? format(shown)
        : String(Math.round(shown))
      : value}{#if suffix}<span class="stat__suffix">{suffix}</span>{/if}</b
  >
```

样式不动（tabular-nums 已全局）。

### 3d. 接入点（如实收窄）

执行时 `grep -rn "<Stat" src/ --include="*.svelte"`，在**能拿到原始 number**（非已
格式化字符串）的调用点里挑 1-2 处接入 `animate` + `format`（如 DevicePanel 总计 tokens：
`<Stat animate={total} format={fmtTokens} .../>`——fmtTokens 需同步改收 number 输出
string）。若所有调用点都只持有格式化字符串且原始数值不在作用域内，则本 commit 只交付
能力不接线，在 commit message 里记录「接线属 P1（`docs/glance-p0-followups.md` §3）」——**不要**为了接线去重构数据流。

### 验收

```bash
npm test        # 新增 6 个补间用例必须过
npx svelte-check
```

### Commit

```
feat(motion): 非对称数字补间 num.ts + Stat 动画模式（glance P0-3）

- createTween：上升 τ=60ms 指数趋近、下降线性限速、首步贴目标、末段吸附
- 纯函数显式时钟，vitest 6 用例直测（含限速/收敛/摆动）
- Stat.svelte 增 animate/format 可选 props；prefers-reduced-motion 直通
- 接线：仅限已持有原始 number 的调用点；否则记 P1（见 `docs/glance-p0-followups.md` §3，不做数据流重构）
- 验收：npm test 全绿；svelte-check 0E0W
```

---

## Step 4 — Rust 滑动落位（执行顺序 #7，最险殿后）

**目的**（glance 学习点：拖拽松手后面板「滑」回边缘，不是瞬移；跟随节拍 8ms、τ=60ms）。

**文件**：`src-tauri/src/dock.rs`（纯函数 + 测试）、`src-tauri/src/ipc.rs`（滑动 runner
+ 两处落位改造）。前端 **零改动**（动画时机由 Rust 依 pill_drag 自判）。

### 4a. dock.rs 追加纯函数

```rust
/// 指数趋近单步（glance FOLLOW 思路：8ms 步进、τ=60ms）。
///
/// 返回 (新值, 是否到位)：差值 < 0.5px 视为到位并直接吸附目标（防像素长尾）。
/// dt 与 tau 同单位（毫秒）。纯函数，cargo test 直测。
pub fn approach_step(current: f64, target: f64, dt_ms: f64, tau_ms: f64) -> (f64, bool) {
    if (target - current).abs() < 0.5 {
        return (target, true);
    }
    let alpha = 1.0 - f64::exp(-dt_ms / tau_ms);
    let next = current + (target - current) * alpha;
    if (target - next).abs() < 0.5 {
        (target, true)
    } else {
        (next, false)
    }
}

#[cfg(test)]
mod glide_tests {
    use super::approach_step;

    #[test]
    fn settles_to_target_when_already_there() {
        let (v, done) = approach_step(100.0, 100.0, 8.0, 60.0);
        assert_eq!(v, 100.0);
        assert!(done);
    }

    #[test]
    fn first_step_matches_exponential() {
        // dt=8, τ=60 → α = 1 - e^(-8/60)；从 100 向 300 的首步幅度按理论值校验
        let (v, done) = approach_step(100.0, 300.0, 8.0, 60.0);
        let expected = 100.0 + 200.0 * (1.0 - (-8.0_f64 / 60.0).exp());
        assert!((v - expected).abs() < 1e-9);
        assert!(!done);
    }

    #[test]
    fn converges_to_target_within_cap() {
        let mut cur = 0.0;
        for _ in 0..500 {
            let (v, done) = approach_step(cur, 400.0, 8.0, 60.0);
            cur = v;
            if done {
                break;
            }
        }
        assert_eq!(cur, 400.0);
    }

    #[test]
    fn snaps_when_within_half_pixel() {
        let (v, done) = approach_step(399.8, 400.0, 8.0, 60.0);
        assert_eq!(v, 400.0);
        assert!(done);
    }
}
```

（dock.rs 若已有 `#[cfg(test)] mod tests`，把四个用例并入即可。）

### 4b. ipc.rs：滑动 runner

import 区补 `use std::sync::atomic::{AtomicU64, Ordering};` 与
`use std::time::Duration;`（已有则跳过）。`snap_to_edges`（:699）之前插入：

```rust
/// 滑动动画代际：主窗与把手窗各一个计数器。新动画使旧线程退出，
/// 避免两个线程互抢 set_position。
static GLIDE_GEN_MAIN: AtomicU64 = AtomicU64::new(0);
static GLIDE_GEN_PEEK: AtomicU64 = AtomicU64::new(0);

const GLIDE_STEP_MS: u64 = 8; // glance：拖拽/跟随节拍
const GLIDE_TAU_MS: f64 = 60.0; // glance：跟随时间常数
// 600ms 封顶 = τ=60ms 的 10 个时间常数：残余 e^-10 ≈ 4.5e-5，5000px 位移也收在
// 0.23px 内，必由 <0.5px 吸附规则落地。（50 步/400ms 只够 393px 内落地。）
const GLIDE_MAX_STEPS: u32 = 75;

/// 让窗口从当前位置指数趋近 `to`（物理像素）。不阻塞调用方；
/// 短命线程 + 8ms 步进；期间同窗新请求会取消旧动画（代际计数）。
fn glide_window_to(window: WebviewWindow, to: (i32, i32), gen: &AtomicU64) {
    let my_gen = gen.fetch_add(1, Ordering::SeqCst) + 1;
    let Ok(from) = window.outer_position() else { return };
    if (from.x, from.y) == to {
        return;
    }
    std::thread::spawn(move || {
        let mut cur = (f64::from(from.x), f64::from(from.y));
        for _ in 0..GLIDE_MAX_STEPS {
            if gen.load(Ordering::SeqCst) != my_gen {
                return; // 已被同窗新动画取代
            }
            std::thread::sleep(Duration::from_millis(GLIDE_STEP_MS));
            let (nx, d1) = crate::dock::approach_step(
                cur.0,
                f64::from(to.0),
                f64::from(GLIDE_STEP_MS),
                GLIDE_TAU_MS,
            );
            let (ny, d2) = crate::dock::approach_step(
                cur.1,
                f64::from(to.1),
                f64::from(GLIDE_STEP_MS),
                GLIDE_TAU_MS,
            );
            cur = (nx, ny);
            let _ = window.set_position(tauri::PhysicalPosition::new(
                nx.round() as i32,
                ny.round() as i32,
            ));
            if d1 && d2 {
                return;
            }
        }
    });
}
```

### 4c. ipc.rs：apply_anchor 增加滑动版（:556 后追加）

```rust
/// [`apply_anchor`] 的滑动版：目标矩形算法一致，把一次性 set_position 换成
/// glide_window_to。返回语义与 apply_anchor 一致（true = 需要落盘）。
///
/// 拖拽中不滑动：拖拽期是 OS 拉拽与吸附的实时博弈，必须瞬时；滑动只属于
/// 「松手落位」（sync_peek_window docked）路径。
pub fn apply_anchor_glide(window: &WebviewWindow, a: DockAnchor) -> bool {
    let Ok(pos) = window.outer_position() else { return false };
    let Ok(size) = window.outer_size() else { return false };
    let Ok(Some(monitor)) = window.current_monitor() else { return false };
    let area = monitor.work_area();
    let (nx, ny) = anchor_rect(
        a,
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    );
    if nx == f64::from(pos.x) && ny == f64::from(pos.y) {
        return false;
    }
    glide_window_to(
        window.clone(),
        (nx.round() as i32, ny.round() as i32),
        &GLIDE_GEN_MAIN,
    );
    true
}
```

（`anchor_rect` 与 `apply_anchor` 同文件既有引用，签名照抄 :542-550。）

### 4d. sync_peek_window docked 分支改用滑动版（:1125）

旧：

```rust
                if apply_anchor(&dash, anchor) {
```

新：

```rust
                if apply_anchor_glide(&dash, anchor) {
```

（`set_window_mode` :808 的 `apply_anchor` **保持瞬时**——模式切换有自身编排时序，
不掺滑动。）

### 4e. reposition_peek 改滑动（:689-697），拖拽中保持瞬时

旧体 :692-696 替换：

```rust
    let Some(pos) = peek_placement(window, anchor.side, anchor.along) else { return };
    if peek.outer_position().map(|cur| cur == pos).unwrap_or(false) {
        return;
    }
    let dragging = window
        .app_handle()
        .try_state::<AppState>()
        .map(|s| s.pill_drag.load(Ordering::SeqCst))
        .unwrap_or(false);
    if dragging {
        let _ = peek.set_position(pos);
    } else {
        glide_window_to(peek, (pos.x, pos.y), &GLIDE_GEN_PEEK);
    }
```

（`AppState` 与 `Ordering` 已在 ipc.rs 作用域内；`pill_drag` 见 :1210-1212 注释。
把手 7×58 细条滑动与 lib.rs:833 Moved-watcher 兼容：滑动期间 Moved 事件反复调
reposition_peek → 代际计数不断取代旧动画、目标恒定 → 视觉连续，无反馈环路——
docked 态 Moved 不会再触发 apply_anchor。）

### 4f. 已知交互与风险（写给 reviewer）

- 滑动 ~200-400ms 与 dockPill 的 t=200ms 缩窗并行：set_size / set_position 是不同
  Win32 调用，可共存；缩窗以当前左上原点为准，与今日瞬时落位后的行为一致（无回归）。
- `pill_last_move_ms` / 停稳检测（lib.rs:822-833）不受影响：滑动产生的是Moved 事件，
  但 docked 分支只调 reposition_peek（幂等）。
- 降级预案（若实测发抖）：把 `GLIDE_STEP_MS` 提到 16 或将 4e 改回瞬时，单独 commit
  记录数据。

### 验收

```bash
cd src-tauri && cargo test   # 含 glide_tests 4 用例
npm test && npx svelte-check
# 手动冒烟：拖胶囊到边缘松手 → 窗口滑回贴边；同边改 along 松手 → 把手细条滑移；
#           拖拽进行中经过边缘 → 吸附保持瞬时无拉扯
```

### Commit

```
feat(dock): 拖拽落位/把手跟随改指数趋近滑动（glance P0-4）

- dock.rs approach_step 纯函数 + 4 用例（理论首步/收敛/吸附/恒等）
- ipc.rs glide_window_to：8ms 步进、τ=60ms、<0.5px 收手、75 步封顶、
  主窗/把手双代际计数防重入
- sync_peek_window docked 分支 → apply_anchor_glide；reposition_peek
  滑动（pill_drag 时瞬时）；set_window_mode 保持瞬时
- 已知：滑动与 dockPill 缩窗并行无冲突（不同 Win32 调用）；降级预案见计划
- 验收：cargo test 全绿；npm test 全绿；svelte-check 0E0W
```

### 落地偏差（实施记录，commit 5de52ea）

实施与本文上列代码有八处差异，其中七处是审查驱动的最小修正，第 7 条是架构级替换。
本轮明确不做、留给 P1/P2 的项统一记在 [`docs/glance-p0-followups.md`](glance-p0-followups.md)
（M2 hover/按压、M3 数字补间接线、M5 落位滑动等的收尾都在那里）。

1. `GLIDE_MAX_STEPS` 50 → **75**：50 步/400ms 只够 393px 内进入吸附区
   （需 7.5·ln(2·gap) 步），而真实落位位移可达整屏宽；75 步=600ms=10τ，
   5000px 残差 0.23px 仍由吸附规则落地。
2. `persist_placement_inner` 只在浮动态（`anchor.is_none()`）读写
   `dashboard_x/y`：贴边态几何由锚点反算；滑动落位期间读 live 位置会存下
   半路过渡坐标，而浮动态启动恢复确实读该字段（`Settings::get_xy_now()`）。
3. `set_window_mode` 开头推进 `GLIDE_GEN_MAIN`：模式切换是瞬时几何改写，
   不作废在跑滑动会被旧线程按旧尺寸写回（贴右 168 → 400 宽会右侧探出屏）。
   不可改挂 `Resized` 事件——t=180ms 的 collapsePill 缩窗会把正常落位截停。
4. `glide_window_to` 增 `label: Option<&'static str>`：仅主窗打
   `glide start` / `glide end` 两条 `tum.window` debug 日志
   （reason=settled|cancelled|capped + frames + elapsed_ms），把手窗静默，
   否则每次落位 26~37 行会把有用信息淹掉。
5. `fetch_add` 早于早退分支的意图补注释：目标等于当前位置也推进代际，
   目的是让在跑的同窗旧动画停下。
6. 常量注释边界值 400px → 393px（50 步实际覆盖范围）。
7. **落位实现整体替换：前端 FLIP → Rust 主窗/把手指数趋近滑动**（架构替换，
   不只是参数调整）。最终实现为 `ipc.rs::glide_window_to` + `dock.rs::approach_step`：
   8ms 步进、τ=60ms、<0.5px 吸附、75 步=600ms 封顶、主窗/把手双代际取消；
   总时长上限由设计时的 ~300ms 改为 **600ms**。前端因此**零改动**——
   `syncPeek(animate)` 未加，胶囊层没有 FLIP，`--tum-dur-settle` /
   `--tum-ease-out-quart` 至今无消费者（tokens.css 注释已按实情改写）。
8. **数字补间未接胶囊用量/费用**：只接了 `DevicePanel` 的「全端汇总」两个数字
   （`aggTotal` / `aggToday`，`fallLimitPerSec={25_000}`，经 `Stat` 的 `animate` 模式）。
   胶囊与其余 `Stat` 调用点的原始 number 不在作用域内（调用点已格式化成字符串），
   接线要改数据流，记 P1（见 `docs/glance-p0-followups.md` §3）。


审查期实测结论（供后续回归参考）：被取代线程恰好多写 1 次，回跳上界为一帧
位移的 12.5%，但真实落位剩余 ≤16 逻辑 px 且 16px 内不换边 → 实际 ≤2~4 物理 px、
单帧不可见；每窗 1 线程跑 26~43 帧，spawn 49.7µs，125/s 压测 peak_live=2，
无线程风暴；`round` 只用于输出无累积误差（5000 步累计 0.557px）。

---

## 执行方式

- **subagent-driven**（推荐）：每 Step 一个子代理，执行前给它对应 Step 全文 + 元信息；
  Lead 对每个 commit 做 diff review + 跑门禁。
- **inline**：Lead 顺序自做，每 Step 结束跑验收、commit，再做下一步。

## 自审记录

- 每 Step 均含：目的（对应 glance 学习点）、文件清单、完整 old/new 代码或确定性映射
  表、验收命令、commit message——无占位符。
- 行号以 be8bbbe 核实，均附内容锚点；执行时先 read 后 edit。
- 如实收窄三处并已记录：thumb 滑移（无 thumb 结构）、LINGER（无穿透恢复表面）、
  num 接线（仅限原始 number 在作用域的调用点）。
- 风险最大项 Step 4 殿后且带降级预案；Step 1-3/5-7 均为前端可回滚小步。
