# 前端设计风格说明（独立可复用版）

> 来源：TokenUsageMonitor（Windows 11 Tauri 桌面端，Svelte 5）
> 用途：本文件为自包含文档，已内联全部设计令牌 CSS，**可整份拷贝到任意新项目**直接复用。
> 引用方式：把文末「附录 A」的定制 CSS 贴入项目的全局样式即可获得整套设计系统。

## 1. 设计总览

**美学定位：Fluent Glass（类 Windows 11 亚克力/微光玻璃）**

- 原生 Mica 质感 + **CSS 玻璃表面分层**（`rgba` 薄层叠加），营造通透、轻盈的桌面感。
- 深色底 `#1a1b1e` 为主体，半透明悬浮层承载内容，**细节优先于装饰**。
- 层级与状态靠材质（表面明度）、排版、动效承载，而非堆叠装饰。
- 几何：柔和小圆角（8/12/16px）+ 胶囊（pill 999px）分段控件。
- 数字一律使用**等宽 + 表格数字（tabular-nums）**，更新时不抖动。

**核心原则**
1. 同屏采用「无框 + 透明 + 深色玻璃」降低视觉噪音，让数据成为主角。
2. 状态语义用青/琥珀/红统一映射，处处一致（ok/warn/crit）。
3. 每个账户/服务可用一个强调色（accent）做品牌区分。

## 2. 设计令牌（Design Tokens）

完整变量见 `src/styles/tokens.css`。核心如下：

### 2.1 颜色

| 令牌 | 值 | 用途 |
|------|-----|------|
| `--tum-bg` | `#1a1b1e` | 窗口底色（Mica 失败兜底） |
| `--tum-surface` | `rgba(255,255,255,0.055)` | 玻璃卡片默认面 |
| `--tum-surface-hover` | `rgba(255,255,255,0.085)` | 玻璃卡片悬停面 |
| `--tum-border` | `rgba(255,255,255,0.09)` | 常规描边 |
| `--tum-border-strong` | `rgba(255,255,255,0.16)` | 强调分隔线 |
| `--tum-text-primary` | `#e6e8ea` | 主文字 |
| `--tum-text-secondary` | `#b3b9c0` | 次要文字 |
| `--tum-text-muted` | `#8b949e` | 弱化/标签 |

### 2.2 强调色（系统蓝）

```
accent   #4cc2ff   hover #7dd3fc   glow  rgba(76,194,255,0.35)
fill     rgba(76,194,255,0.12)     stroke rgba(76,194,255,0.45)
```

用法：聚焦态、悬停态、品牌点、"ok" 渐变起点。`--tum-accent-fill-strong`（0.22）用于分段控件的激活项。

### 2.3 状态色 + 渐变（ok/warn/crit）

| 状态 | 主干色 | 渐变 |
|------|--------|------|
| ok | `#6ccb5f` | `linear-gradient(90deg, #4cc2ff, #6ccb5f)` |
| warn | `#ffc83d` | `linear-gradient(90deg, #e8b53d, #ffc83d)` |
| crit | `#ff5f56` | `linear-gradient(90deg, #e05248, #ff5f56)` |

语义：**已用量 ≥95% → crit，≥80% → warn**，其余 ok（阈值统一，切换「显示已用/剩余」不改色）。

### 2.4 特殊色

- 琥珀 `#fbbf24`：仅用于品牌点与热力图「今日」描边（遗留湘磷风格点缀）。

### 2.5 间距

`4 / 8 / 12 / 16 / 24px`（`--tum-space-1..5`），组件内统一使用。

### 2.6 圆角

`xs 4 / sm 8 / md 12 / lg 16 / pill 999`。卡片 md=12，玻璃外壳/悬浮窗 lg=16，分段控件 pill。

### 2.7 字体

```
正文 : "Segoe UI Variable", "Segoe UI", system-ui, sans-serif
等宽 : "Cascadia Code", Consolas, monospace
字号 : 10 / 11 / 13 / 16 / 20px（xs / sm / base / lg / xl）
```

- 数字/百分比/余额/时间一律走等宽字体 + `font-variant-numeric: tabular-nums`。
- 标题用 `letter-spacing + text-transform: uppercase` 营造工具感。
- `text-rendering: optimizeLegibility` + `-webkit-font-smoothing: antialiased`。

## 3. 玻璃材质实现要点

```css
/* 外壳：径向强调色微光 覆盖 半透明深底 + 背景模糊 */
background:
  radial-gradient(120% 120% at 0% 0%, rgba(76,194,255,0.07), transparent 42%),
  rgba(24,26,30,0.82);
border: 1px solid var(--tum-border);
border-radius: var(--tum-radius-lg);
backdrop-filter: blur(16px);
```

- **悬浮浮层**（详情卡）用 `--tum-blur-card: 24px` 更强的模糊，层级更高。
- 卡片本体不模糊，靠 `--tum-surface` 半透明白面，叠在透明窗口上依然通透。
- 透明窗口 + 圆角 = **真正可见的圆角**（角落无底色）。

## 4. 交互与状态语言

### 4.1 按钮
```
尺寸 22px × 22px；border 常规描边；底色 --tum-surface；
radius sm(8px)；悬停 → 文字变 accent + 描边 accent-stroke + 底 accent-fill；
transition: all 0.15s ease；
图标用等宽字体字符（↻ ⚙ ⤢ ×），不用图片。
```

### 4.2 卡片（ProviderCard）
- 默认：`--tum-surface` 面 + 细描边 + md 圆角，悬停升一档面板明度。
- **聚焦态**：`inset 0 0 0 1px` 强调色描边 + 外侧主 visa，形成内嵌描边，不做填满。
- 账户强调色通过**内联 CSS 变量**注入：
  ```css
  --acct-accent:#12b76a; --acct-accent-stroke:rgba(18,183,106,0.45);
  --acct-accent-fill:rgba(18,183,106,0.12); --acct-accent-glow:rgba(18,183,106,0.35);
  ```
  无强调色时全部回退到 `var(--tum-*)`。

### 4.3 分段控件（Pill Toggle）
- 外框：细描边 + pill 圆角 + 微透明底（`rgba(255,255,255,0.04)`）。
- 激活项：`accent-fill` 底 + 主文字；未激活项 muted 文字；悬停升为主文字。
- 内间距 `padding:2px`，按钮 `padding:2px 8px`，轻量、贴合。

### 4.4 SVG 环形进度（ProgressRing）
- 用 `stroke-dasharray/dashoffset` 绘制，**`transition: stroke-dashoffset 0.6s ease`** 平滑更新。
- ok 状态按账户强调色渐变；warn/crit 用固定状态渐变，保证可识别。
- crit 时加外发光 `filter: drop-shadow(0 0 5px rgba(255,95,86,0.55))`。
- 可叠加**双环**（内=短窗口，外=长窗口），外层不透明度 0.55，区分主次。
- 空数据 `idle`：只画中性暗轨道，不画弧、不发 crit 光。

### 4.5 状态点（PulseDot）与错误条
- 品牌/状态点：`width/height 6-8px` 圆形 + `box-shadow 0 0 8px 同色 glow`（呼吸感）。
- 错误提示条：`danger-fill` 底 + **左侧 2px 竖线** + 圆点，弱化整块警示框。

### 4.6 呼吸/告警动效（crit）
```css
@keyframes breathe {
  0%,100% { box-shadow: 0 0 0 0 rgba(red,0), inset 0 0 0 1px rgba(red,0.4); }
  50%     { box-shadow: 0 0 16px 0 rgba(red,0.5), inset 0 0 0 1px rgba(red,0.85); }
}
```
原则：**发蓝光用大 blur、spread 0**，贴合圆角不硬边外扩；crit 用 `1.6s ease-in-out infinite` 呼吸提示。

## 5. 排版与文字细节

- 界面文案默认中文（本项目面向中文用户），行业标签可用英文大写。
- 过大/过细文案：区块标题用小号值 + 大 letter-spacing 大写（如"日历热力图"），弱标签比正文小一档。
- 空状态：主提示 + 下方更小、更淡、大写的 hint，居中，等宽字体带字距。
- 表格式数据对齐官网用 tabular-nums；时间戳 `HH:mm:ss` 走等宽。

## 6. 无障碍与玻璃的取舍

- 交互元素需可键盘聚焦：`:has(:focus-visible)` 加 `outline 2px accent，outline-offset 2px`；透明态元素悬停/聚焦时恢复可见。
- 玻璃透明度不宜过低，保证文字对比度；状态用**颜色 + 文字/形状**双重编码（不只靠颜色表达告警）。
- 仅用 `icon` 的按钮务必带 `aria-label` + `title`。
- hover 悬浮层之间用**宽限期（如 160ms）**防消失抖动。

## 7. 快速复用清单（新项目怎么套）

1. 把文末「附录 A」**完整 CSS** 贴入全局样式（或 `:root`），即获得整套设计令牌，无需再拷贝源码。
2. 同类组件可参考同类组件实现：`ProgressRing`（SVG 环形）、`ProviderCard`（玻璃卡片）、`UsageBar`（用量条）、PillToggle（分段控件）、`PulseDot`（状态点）、HeatmapGrid（热力图）。
3. 记住三件套即可复刻视觉：
   - 深色半透明底 + `backdrop-filter: blur(16px)` + 细描边 → 玻璃感；
   - 一个系统蓝（`#4cc2ff`）+ ok/warn/crit 状态渐变 → 语义色；
   - 等宽 `tabular-nums` 处理一切数字 + 柔和圆角(8/12/16) + pill 分段 → 工具感与秩序。
4. 需要惊艳感时，为每类数据/账户给一个可编程强调色（accent），而非全局单一色。

## 8. 已验证的约束（项目实测）

- 数字等宽表格数字避免刷新抖动（Dashboard 每秒时间戳、环形百分比、余额）。
- 透明窗口下圆角真正可见，需处理动画外发光不出现矩形硬边。
- 悬浮详情层提到窗口级渲染，规避小卡片内溢出裁剪问题。

---

## 附录 A：完整设计令牌（可直接拷贝）

以下为 TokenUsageMonitor 全局样式，自包含、无外部依赖。贴入新项目的全局 CSS 即获得整套设计系统。

```css
:root {
  /* 主题背景 - 本套为深色玻璃主题 */
  --tum-bg: #1a1b1e;
  --tum-bg-solid: #1a1b1e;

  /* 玻璃表面层（白色 rgba 叠于背景之上） */
  --tum-surface: rgba(255, 255, 255, 0.055);
  --tum-surface-hover: rgba(255, 255, 255, 0.085);
  --tum-border: rgba(255, 255, 255, 0.09);
  --tum-border-strong: rgba(255, 255, 255, 0.16);

  /* 文字 */
  --tum-text-primary: #e6e8ea;
  --tum-text-secondary: #b3b9c0;
  --tum-text-muted: #8b949e;

  /* 强调色：系统蓝 */
  --tum-accent: #4cc2ff;
  --tum-accent-hover: #7dd3fc;
  --tum-accent-glow: rgba(76, 194, 255, 0.35);
  --tum-accent-fill: rgba(76, 194, 255, 0.12);
  --tum-accent-fill-strong: rgba(76, 194, 255, 0.22);
  --tum-accent-stroke: rgba(76, 194, 255, 0.45);

  /* 状态色 + 渐变 */
  --tum-ok: #6ccb5f;
  --tum-warn: #ffc83d;
  --tum-crit: #ff5f56;
  --tum-grad-ok: linear-gradient(90deg, #4cc2ff, #6ccb5f);
  --tum-grad-warn: linear-gradient(90deg, #e8b53d, #ffc83d);
  --tum-grad-crit: linear-gradient(90deg, #e05248, #ff5f56);

  /* 状态别名（兼容） */
  --tum-success: var(--tum-ok);
  --tum-success-fill: rgba(108, 203, 95, 0.12);
  --tum-warning: var(--tum-warn);
  --tum-danger: var(--tum-crit);
  --tum-danger-fill: rgba(255, 95, 86, 0.10);

  /* 琥珀（可选品牌点缀） */
  --tum-amber: #fbbf24;
  --tum-amber-glow: rgba(251, 191, 36, 0.35);

  /* 间距 */
  --tum-space-1: 4px;
  --tum-space-2: 8px;
  --tum-space-3: 12px;
  --tum-space-4: 16px;
  --tum-space-5: 24px;

  /* 圆角 */
  --tum-radius-xs: 4px;
  --tum-radius-sm: 8px;
  --tum-radius-md: 12px;
  --tum-radius-lg: 16px;
  --tum-radius-pill: 999px;

  /* 字体 */
  --tum-font: "Segoe UI Variable", "Segoe UI", system-ui, -apple-system, sans-serif;
  --tum-font-mono: "Cascadia Code", "Cascadia Mono", Consolas, monospace;
  --tum-font-size-xs: 10px;
  --tum-font-size-sm: 11px;
  --tum-font-size-base: 13px;
  --tum-font-size-lg: 16px;
  --tum-font-size-xl: 20px;

  /* 悬浮层模糊 */
  --tum-blur-card: 24px;
}

/* 全局基础 */
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body {
  height: 100%;
  overflow: hidden;
  background: transparent;
  font-family: var(--tum-font);
  font-size: var(--tum-font-size-base);
  color: var(--tum-text-primary);
  user-select: none;
  -webkit-font-smoothing: antialiased;
  text-rendering: optimizeLegibility;
}

/* 数字统一等宽表格数字，防更新抖动 */
.tum-numeric, input[type="number"], .tum-mono {
  font-family: var(--tum-font-mono);
  font-variant-numeric: tabular-nums;
  font-feature-settings: "tnum" 1, "zero" 1;
}

/* 玻璃外壳 */
.tum-glass {
  background:
    radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
    rgba(24, 26, 30, 0.82);
  border: 1px solid var(--tum-border);
  border-radius: var(--tum-radius-lg);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}

/* 账户强调色注入：通过内联 CSS 变量覆盖 */
/* --acct-accent:#12b76a;
   --acct-accent-stroke:rgba(18,183,106,0.45);
   --acct-accent-fill:rgba(18,183,106,0.12);
   --acct-accent-glow:rgba(18,183,106,0.35); */
```