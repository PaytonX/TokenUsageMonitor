# 设计文档：贴边收起动效重叠 + 模型页签百分比圆环图

日期：2026-09-27
分支：feat/logo-pill-edge-peek
状态：已确认（用户拍板：动效"时间线重叠"方案 + 图表 A 圆环替换顶部选择行）

## 1. 背景与问题

### 1.1 贴边收起动效（用户原话："胶囊突然消失，隔一会，小把手又突然出现一下"）

现状 dockPill() 链路三段式串行，产生三个视觉病灶：

| t | 动作 | 视觉后果 |
|---|------|---------|
| 0ms | await collapsePill()：窗口 168×expanded → 168×56 | 明细（mini rows）被窗口边界瞬间裁掉，一帧瞬消 |
| 0~150ms | pillRevealed=false → .is-docked transform 滑出，--tum-ease-spring（ease-out，起步最快） | 头部像被弹飞出屏，缺"钻入边缘"的加速感 |
| 150+90=240ms 后 | syncPeek() IPC 往返 + emitPeekShow → 把手 opacity 110ms 纯淡入 | 死区空窗（240ms 什么都看不见）后把手突现，无方位感 |

### 1.2 模型页签缺少用量占比视图

ModelPanel.svelte 顶部是 PillsOrSelect 胶囊行（模型名·tokens），只能逐个点选对比，无占比概览；模型多时长尾模型（用量极少）与主力模型平铺，噪声大。

## 2. 目标 / 非目标

**目标**
- F1：收起动效改为"时间线重叠"——滑出、把手就位、窗口收缩三件事并行交叠，全程无视觉空窗。
- F2：模型页签顶部改为百分比圆环图 + 图例，替换 PillsOrSelect 选择行；长尾模型过滤归并为「其他」。

**非目标**
- 不改 Rust 侧（ipc.rs 三态窗口逻辑、peek_rect/dock_window 均不动）。
- 不改 TrendPanel/ToolPanel/DevicePanel；PillsOrSelect 组件本身保留（其他面板可能仍用）。
- 不引入图表库（沿用项目手写 SVG 风格）。

## 3. F1：收起动效"时间线重叠"

### 3.1 新时间线

```
t=0      pillRevealed=false → .is-docked 生效：滑出动画开始（200ms ease-in），
         明细随整个 pill-layer 一起水平钻出屏幕（不再先裁剪）
t=120ms  syncPeek("docked")：窗口 flush 贴边 + 鼠标捕获交换（把手窗口就位）
         → emitPeekShow：把手淡入 160ms ease-out + scaleX(0.4→1) 从边缘长出
t=200ms  滑出结束（内容已完全出屏）→ collapsePill() resize 168×expanded → 168×56
         （裁剪发生在屏幕外，不可见）；pillCollapsing=false 解除防重入
```

视觉：胶囊加速钻入边缘的同时把手从边缘长出，两者在 t≈120~280ms 交叠，无空窗。
死区从 240ms+110ms 压缩到 120ms（且前 200ms 被滑出动画填满）。

### 3.2 App.svelte 改动

**常量（L303-304）**：

```ts
const PILL_SLIDE_MS = 200; // 滑出时长（ease-in 钻边）；与 .pill-layer.is-docked 的 transition 保持一致
const PILL_PEEK_LEAD_MS = 120; // 滑出中途就位把手（同步+复现），与滑出尾部重叠消除空窗
```

PILL_PEEK_BACK_MS 删除（不再使用）。

**dockPill（L672-696）重写**：

```ts
async function dockPill() {
  if (pillDragActive) return;
  const gen = ++peekGen;
  clearMiniTimer();
  pillCollapsing = true;
  // 先滑出：明细随层一起钻出屏幕，resize 延后到滑出结束（裁剪发生在屏外）。
  pillRevealed = false;
  // 滑出中途先就位把手：贴边 flush + 捕获交换 + 把手淡入，与滑出尾部重叠。
  setTimeout(() => {
    if (gen !== peekGen) return;
    void (async () => {
      await syncPeek(); // docked=true：把手接回鼠标（内部顺带 emitPeekShow 自愈）
      if (gen !== peekGen) return;
      void emitPeekShow(pillSide).catch(() => {});
    })();
  }, PILL_PEEK_LEAD_MS);
  // 滑出完全出屏后再收缩窗口：pillCollapsing 必须无条件清除（revealPill 不清它，
  // 若 gen 已换代而我们不清，防重入闩锁会卡死）。
  setTimeout(() => {
    pillCollapsing = false;
    if (gen !== peekGen) return;
    void collapsePill();
  }, PILL_SLIDE_MS);
}
```

保留原函数头部的 ⚠️ 注释（emitPeekShow 是把手唯一复活路径，不得短路）。

**CSS（L1230-1246）**：滑出/滑入方向分离——transition 取"变化后状态"的值，故基础规则=滑入（保持 spring），.is-docked 规则=滑出（ease-in）：

```css
.pill-layer {
  transition: transform 150ms var(--tum-ease-spring); /* 滑入：快进感 */
}
.pill-layer.is-docked {
  transition: transform 200ms var(--tum-ease-dock-exit); /* 滑出：加速钻边 */
}
```

**tokens.css**：新增 `--tum-ease-dock-exit: cubic-bezier(0.32, 0, 0.67, 0);`（标准 ease-in）。

### 3.3 peek/main.ts 把手动画

- 初始类名改为 `peek is-hidden`（新建窗口默认隐藏，等 peek-show 显形——把手每次收起都会销毁重建，此路径覆盖主流程；丢失 peek-show 时由 syncPeek 的自愈兜底）。
- .peek 增加 transform 过渡：opacity/transform 160ms cubic-bezier(0.33,1,0.68,1)；
- 新增 `.peek.is-hidden { opacity:0; transform: scaleX(0.4); }`，默认态 transform: scaleX(1)；
- transform-origin 贴边侧：[data-side="right"] → right center，[data-side="left"] → left center（从屏幕边缘往里长）。
- prefers-reduced-motion 覆盖选择器已包含 .peek，无需额外处理。
- hover 隐藏（is-hidden）也走缩回边缘动画，与胶囊滑入形成呼应。

## 4. F2：模型页签百分比圆环图

### 4.1 数据派生（ModelPanel.svelte）

```ts
const RING_MAX_SLICES = 5;   // 独立切片上限
const RING_MIN_SHARE = 0.02; // 占比低于 2% 归入「其他」
```

- 模型总数 ≤ 6：全部独立直出（不出「其他」）。
- 否则：前 5 名且占比 ≥ 2% 为独立切片；其余（第 6 名以后 + 占比 <2% 的）合并为「其他」，灰色、置底。
- ringSlices = $derived.by(...)：输出 { id, name, total, share, color, isOther, members? }[]，share 基于累计 tokens（与现选择行排序口径一致）。
- 色板：[accent, "#8f7bf2", "#5fd4a2", "#f2b35b", "#f27b9b", "#7b93f2"]（accent 打头保证主题联动）；「其他」用 var(--tum-text-muted)。

### 4.2 圆环 SVG（手写，stroke-dasharray 圆弧）

- viewBox 0 0 110 110，r=42，stroke-width=13，整环 rotate(-90deg) 从顶部起笔；
- 轨道圈淡色打底；切片 dasharray = share×C−gap 与剩余弧长（gap≈2px，极小切片保底 1px）；
- 中心文本：默认显示累计 tokens 总量（fmtTokens）+「累计 tokens」小字；hover 切片/图例时切换为该切片 模型名 / 占比 / tokens；
- 交互：
  - 点击独立切片或图例 = onPick(modelId)（联动下方 stats + 线图，activeId/writePref 逻辑不变）；
  - 点击「其他」不选中（无唯一目标）；
  - hover 切片 ↔ 图例双向高亮（hoverId $state）；「其他」图例行 title 属性列出合并成员名。

### 4.3 布局

mp__head 之后、mp__stats 之前：

```
[头部：模型用量 | 近7/30/90天 ↻]
[mp__ring：圆环(~130px) | 图例纵列（色点+模型名+占比%）]
[mp__stats]（不变）
[mp__chart：TrendLineChart]（不变）
```

删除 PillsOrSelect 的使用与 import（组件文件保留）。loading/error/empty 三态分支保持原位。

## 5. 风险与对策

| 风险 | 对策 |
|------|------|
| 滑出中把手 hover 唤回竞态 | 滑出期间捕获在主窗（syncPeek 未调用前把手不可点）；gen 代际检查贯穿两个 setTimeout；pillCollapsing 无条件清除避免闩锁卡死 |
| revealPill 换代后 collapsePill 误缩窗口 | timer 内 pillCollapsing=false 先于 gen 检查，collapsePill() 在 gen 检查之后——换代即跳过收缩 |
| 把手新建窗口无动画 | 初始 is-hidden + peek-show 显形，覆盖每次收起都销毁重建的主路径；syncPeek 自愈兜底 |
| ease-in 滑出让"擦过把手"更难唤回 | 防重入解除时机不变（动画结束），L392 判定链不动 |
| 圆环切片过多过碎 | RING_MAX_SLICES + RING_MIN_SHARE 双阈值归并；gap 保底宽度 |
| 多币种成本 | 圆环只用 tokens 口径（累计），不涉币种折算；成本仍走 fmtCost 三态 |

## 6. 验证

- npm run build（tsc + vite，覆盖 App.svelte / peek/main.ts / ModelPanel.svelte 类型与打包）；
- 手动清单：① hover 胶囊→移开→观察"钻入+把手长出"交叠无空窗；② 把手 hover 唤回正常；③ 拖拽贴边、左侧贴边镜像正常；④ reduced-motion 下无动画；⑤ 模型页签圆环/图例/「其他」hover tooltip、点击选中联动线图；⑥ 7/30/90 天切换后圆环联动（切片为累计口径，仅线图随区间刷新）。