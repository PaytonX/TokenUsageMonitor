# 前端组件库使用规范

> 适用范围：TokenUsageMonitor（Windows 11 Tauri 桌面端，Svelte 5）。
> 目标：让新页面 / 新面板在不写一行视觉 CSS 的前提下，仍旧贴合设计系统。

---

## 1. 三层组件结构

```
src/lib/components/
├── atoms/            ← 视觉原子，无业务状态；新页面优先从这里取零件
│   ├── PanelHeader.svelte
│   ├── RangePills.svelte
│   ├── ZoomButton.svelte
│   ├── Stat.svelte
│   ├── ColorSwatch.svelte
│   └── index.ts
├── PulseDot.svelte   ← 状态点（已存在，保留原路径）
├── UsageBar.svelte   ← 用量条（已存在，保留原路径）
├── MiniPanel.svelte            ← 账户迷你条
├── ResetCountdown.svelte       ← 倒计时
├── ProviderLogo.svelte         ← 品牌 logo
├── PillsOrSelect.svelte        ← 胶囊+下拉 复合控件
├── ProgressRing.svelte         ← SVG 环形进度
├── ProviderCard.svelte         ← 业务卡片，已有
├── TrendPanel.svelte            ← 趋势面板（已迁移到原子）
└── ...
```

**规则**

- 业务组件不写视觉外观相关的 CSS，只做"数据 → 原子"的编排；
- 业务组件不直接持有 `__head / __title / __range / __zoom / __stat / __swatch` 这类命名空间，全部走原子 + 自身的 `__chart / __legend / __row` 这类业务专属类名；
- atoms 只承载"外观 + 一两个 prop"，业务状态 / 数据获取放父级。

---

## 2. 原子速查表

### PanelHeader — 面板/窗口标题区

```svelte
<PanelHeader title="趋势看板 · 本机工具">
  <RangePills options={RANGES} value={range} onChange={setRange} />
  <ZoomButton onclick={openWindow} />
</PanelHeader>
```

- 默认 `aria-label` 等于 title；窗口级渲染时给 `label="..."` 提供更具体的描述。
- 右侧槽接收任意子片段，不限于 RangePills / ZoomButton。

### RangePills — 分段区间选择

```svelte
<RangePills
  options={RANGES}                 <!-- [{ key: '7d' | '30d' | ..., label: '近 7 天' }, ...] -->
  value={range}
  onChange={(k) => (range = k)}
  accent={acctAccent}              <!-- 可选；带账户强调色时激活态底色改用 accent 18% 半透 -->
/>
```

- 类型泛型 `T extends string`，传入的 options / value / onChange 都按 `T` 收敛；
- 持久化由父级自行处理（`writePref("tum.xxx.range", k)`）。

### ZoomButton — 放大为独立窗口

```svelte
<ZoomButton title="放大为独立窗口" onclick={openWindow} />
```

- 默认字形 `⤢`；未来需要统一换 SVG 时只改这一处。

### Stat — 数字 + 弱化标签

```svelte
<Stat value={fmtTokens(total)} label="累计 tokens" />
<Stat value={streak} label="连续活跃" suffix="天" />

<!-- 多条横排 -->
<div class="my-stats-row">
  <Stat value={total} label="本机累计" />
  <Stat value={streak} label="连续活跃" suffix="天" />
</div>
```

- 等宽 + 表格数字已在内部，调用方不要重复声明 `font-variant-numeric`；
- 行间距走 `--tum-space-5`（24px）即可，与现状一致。

### ColorSwatch — 彩色小色块

```svelte
<ColorSwatch color={colors[id] ?? 'rgb(138,143,152)'} size={8} />
<ColorSwatch color="#fbbf24" shape="round" size={6} />
```

- `color` 接受 `#RRGGBB` / `rgb()` / `rgba()` / CSS 命名色；
- `shape="square"` 默认 4px 圆角（与既有图例小方块一致）；`round` 用于状态点。

---

## 3. 设计令牌 vs 实际写法

### 3.1 颜色 ──── 只用 token，不写 hex

| 场景 | 写法 | 禁止 |
|------|------|------|
| 玻璃面 | `var(--tum-surface)` | `rgba(255,255,255,0.05)` |
| 强玻璃面 | `var(--tum-surface-hover)` | `rgba(255,255,255,0.08)` |
| 描边 | `var(--tum-border)` | `rgba(255,255,255,0.09)` |
| 主文字 | `var(--tum-text-primary)` | `#e6e8ea` |
| 次文字 | `var(--tum-text-secondary)` | `#b3b9c0` |
| 弱文字 | `var(--tum-text-muted)` | `#8b949e` |
| 强调 | `var(--tum-accent)` | `#4cc2ff` |
| 状态 ok | `var(--tum-ok)` / `var(--tum-success)` | `#6ccb5f` / `#34d399` |
| 状态 warn | `var(--tum-warn)` / `var(--tum-warning)` | `#ffc83d` / `#fbbf24` |
| 状态 crit | `var(--tum-crit)` / `var(--tum-danger)` | `#ff5f56` / `#f87171` |

**品牌强调色例外：** provider / 账户型没有专属 token，通过 `--acct-accent` 等局部变量绑入；

```html
<article style="--acct-accent:#12b76a; --acct-accent-stroke:rgba(18,183,106,0.45);
                    --acct-accent-fill:rgba(18,183,106,0.12); --acct-accent-glow:rgba(18,183,106,0.35);">
  <ProgressRing ... />  <!-- 内部走 var(--acct-accent) -->
</article>
```

### 3.2 字号 ──── 只用 token，不写 px

| token | px | 用途 |
|-------|----|----|
| `--tum-font-size-xs` | 10 | 等宽副标题、tag、stat label |
| `--tum-font-size-sm` | 11 | 弱说明、等宽小字 |
| `--tum-font-size-base` | 13 | 默认正文 |
| `--tum-font-size-lg` | 16 | 次级标题 |
| `--tum-font-size-xl` | 20 | 大标题 |

数值类一律套 `.tum-numeric` 或 `.tum-mono`，自动等宽 + 表格数字：

```html
<span class="tum-numeric">{pct}%</span>
```

### 3.3 间距 / 圆角 ──── 只用 token

| token | px | 用途 |
|-------|----|----|
| `--tum-space-1` | 4 | 紧凑行内 |
| `--tum-space-2` | 8 | 默认 gap |
| `--tum-space-3` | 12 | 卡片内 |
| `--tum-space-4` | 16 | 卡片外 |
| `--tum-space-5` | 24 | 段落间距 |

| token | px | 用途 |
|-------|----|----|
| `--tum-radius-xs` | 4 | 小色块、小按钮 |
| `--tum-radius-sm` | 8 | 控件按钮 |
| `--tum-radius-md` | 12 | 卡片 |
| `--tum-radius-lg` | 16 | 玻璃外壳 / 浮窗 |
| `--tum-radius-pill` | 999 | 胶囊 / 分段 |

---

## 4. 已有"天然原子"

这些组件已具备原子性质，新页面直接复用，不要再造轮子：

| 组件 | 用途 | 关键 prop |
|------|------|-----------|
| `PulseDot` | 状态点 / 品牌点 | `active`, `tone: 'ok'\|'warn'\|'crit'`, `accent` |
| `UsageBar` | 用量进度条（标题 + 数字 + 进度） | `usage: WindowUsage`, `label` |
| `MiniPanel` | 账户迷你条列表 | `snapshots`, `countdown`, `accentFor` |
| `ResetCountdown` | 重置倒计时 | `resetAt`, `label` |
| `ProviderLogo` | 品牌 logo | `kind`, `size`, `accent` |
| `PillsOrSelect` | 胶囊 / 下拉自适应 | `items`, `value`, `onChange`, `dotFor` |
| `ProgressRing` | SVG 环形进度 | `value`, `tone`, `size`, `accent`, `breathe` |

---

## 5. 反模式清单 ──── 不要在新代码里写

| 反模式 | 改用 |
|--------|------|
| `<i style="background:#xxx">` 内联色块 | `<ColorSwatch color={...}>` |
| 自己写 `padding: 2px 8px; border-radius: 999px` 等分段按钮 | `<RangePills>` |
| 自己写 18×18 `⤢` 按钮 + hover 描边 | `<ZoomButton>` |
| `<b>X</b><span>Y</span>` 自定义统计行 | `<Stat value="X" label="Y">` |
| 散落多处 `font-size: 10px / 11px / 13px` | `var(--tum-font-size-XX)` |
| 散落多处 `#34d399 / #fbbf24 / #f87171` | `var(--tum-ok) / --tum-warn / --tum-crit)` |
| 自己实现 `breathe / pulse / ripple` 关键帧 | 复用 `PulseDot`，或新增"动画原子"并接入统一语言 |
| 给每个 panel 写新 `__head / __title / __head-right` CSS | `<PanelHeader title="...">` |

---

## 6. 新建页面 checklist

> 新建一个面板/窗口前，对一遍这张清单；偏离任何一项都属于"先回到这里再写代码"。

- [ ] **标题区** 用 `<PanelHeader>`；不要自己再写 `__head / __title / __head-right`；
- [ ] **时间区间** 用 `<RangePills>`；不要自己写 `__range / __range:hover / __range.is-active`；
- [ ] **放大按钮** 用 `<ZoomButton>`；不要自己写 18×18 `⤢` 按钮 + hover；
- [ ] **统计行** 用 `<Stat>`；行间距走 `--tum-space-5`；
- [ ] **图例 / 色块** 用 `<ColorSwatch>`；颜色 fallback 用 `rgb(138,143,152)`（设计令牌的中性色之一），不要硬编 `#8a8f98`；
- [ ] **状态点** 用 `<PulseDot>`；不要自己写 `.dot + @keyframes`；
- [ ] **用量条** 用 `<UsageBar>`；标题 + 数字 + 进度已封装；
- [ ] **字号** 全部走 `var(--tum-font-size-*)`；数字一律 `.tum-numeric`；
- [ ] **颜色** 全部走 `var(--tum-*)`；仅当账户强调色例外，注入 `--acct-accent*`；
- [ ] **间距 / 圆角** 全部走 `var(--tum-space-*)` / `var(--tum-radius-*)`；
- [ ] **业务专属 CSS** 只覆盖 `__chart / __legend / __grid / __row` 这类业务结构命名，不碰外观；
- [ ] **动画** 复用既有 `breath --tum-ease-*` 曲线；时长走 200ms（状态切换）/ 600ms（环形）/ 1.6s（呼吸）/ 2.6s（闲置呼吸）；
- [ ] **键盘可达** 交互元素加 `:focus-visible { outline: 2px var(--tum-accent); outline-offset: 2px; }`；
- [ ] **Reduced motion** 已被 `tokens.css` 全局处理，不要在新 CSS 里再加 `prefers-reduced-motion`。

---

## 7. 迁移进度 ──── 已落地

| 组件 | 替换前 | 替换后 | Δ 行数 | 涉及原子 |
|------|------|------|------|---------|
| TrendPanel.svelte | 327 | 197 | −130 (−40%) | PanelHeader + RangePills + ZoomButton + Stat + ColorSwatch |
| ModelPanel.svelte | 672 | 582 | −90 (−13%) | PanelHeader + RangePills + Stat |
| ToolPanel.svelte | 554 | 402 | −152 (−27%) | PanelHeader + RangePills + ZoomButton + Stat + ColorSwatch |
| DevicePanel.svelte | 691 | 621 | −70 (−10%) | PanelHeader + Stat + ColorSwatch |
| ToolWindow.svelte | 457 | 397 | −60 (−13%) | RangePills + ColorSwatch |
| TrendWindow.svelte | 438 | 347 | −91 (−21%) | RangePills + ColorSwatch |

合计移除 **−593 行**重复 CSS（5 个原子 ~250 行）。`dashboard` 入口产物从 84.51 kB → 81.22 kB（−3.29 kB / −3.9%）。

## 8. 仍待迁移的重复区

候选（按 ROI 排序）：

1. **HeatmapGrid.svelte** ──── 内含 `__stat b + __stat span` 与 `__legend-dot` 的内联色块，预计可节省 ~30 ~ 50 行；本轮未动，下一顺手做。
2. **CalendarSection.svelte** ──── 包含 `__head / __legend / __key` 三件套，与原子高度同构；但日历有自定义日期格逻辑，建议作为单独任务审视。
4. **App.svelte** ──── 顶部胶囊条（`pill-breathe`）与 PulseDot 是同一语言的两次实现，未来可以收敛到 PulseDot；本轮不动。
5. **Settings.svelte** ──── 体积最大（140KB），重复最多（`pane__title / section__title / rt-card__title` 等都走大写 + 弱化），但因为字号 / 圆角 / 间距与 token 体系差别大（全用 11.5 / 12.5 / 9px 这种 token 外的尺寸），建议单独审视后再决定是否统一；本轮不动。

迁移前先 `svelte-check` 跑通，再 `vitest run` 确认无回归，最后 `vite build` 确认产物大小不退化。