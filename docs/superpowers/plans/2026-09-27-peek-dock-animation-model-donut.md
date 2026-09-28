# 实施计划：贴边收起动效「时间线重叠」+ 模型用量「占比圆环」

> **REQUIRED SUB-SKILL**：本计划按 writing-plans 规范编写——每个任务小步、可独立验证、含精确代码（无占位符）。
> 执行方式：本会话内联按序执行，每步验证通过再进入下一步。

- 日期：2026-09-27 ｜ 分支：feat/logo-pill-edge-peek ｜ 基线：d61ac4a
- 设计文档：docs/superpowers/specs/2026-09-27-peek-dock-animation-model-donut-design.md
- 范围：仅前端（App.svelte / peek/main.ts / ModelPanel.svelte / tokens.css），无 Rust 改动
- 编辑方式约束：所有编辑一律用 PowerShell 字面量替换函数（BOM 感知、行尾自动归一、断言锚点命中次数）；禁用命令解释器包装与交互式命令；注释与提交信息全部中文。
- 提交尾注沿用仓库惯例（见 d61ac4a）：Constraint / Rejected / Confidence / Scope-risk / Not-tested。
- 白名单（提交只含这些）：src/App.svelte、src/peek/main.ts、src/lib/components/ModelPanel.svelte、src/styles/tokens.css、两份 docs/superpowers 文档。

## 现状问题
1. 收起割裂：collapsePill 先缩窗（内容瞬消）→ 240ms 空窗 → 把手突现。
2. 滑出/滑入共用 spring 缓动，收起显得「弹飞」。
3. 模型页签顶部选择行占空间，长尾模型平铺无重点。

## Task 1 — tokens.css：新增收起缓动 token
OLD（L85，唯一命中）：
```css
  --tum-ease-spring: cubic-bezier(0.33, 1, 0.68, 1);
```
NEW：
```css
  --tum-ease-spring: cubic-bezier(0.33, 1, 0.68, 1);
  --tum-ease-dock-exit: cubic-bezier(0.32, 0, 0.67, 0); /* 收起滑出：加速钻边 */
```
验证：Select-String 'tum-ease-dock-exit' → 1 处。

## Task 2 — App.svelte：收起时序常量（L303-304）
OLD：
```ts
  const PILL_SLIDE_MS = 150; // 与 .pill-layer 的 transition 时长一致
  const PILL_PEEK_BACK_MS = 90; // 滑出完成后把手复现的延迟
```
NEW：
```ts
  const PILL_SLIDE_MS = 200; // 与 .pill-layer.is-docked 的滑出时长一致
  const PILL_PEEK_LEAD_MS = 120; // 滑出进行中就让把手接回鼠标并复现（时间线重叠）
```
验证：grep 'PILL_PEEK_BACK_MS' → 0 处。

## Task 3 — App.svelte：dockPill 重写（L672-696，含文档注释整体替换）
OLD：
```ts
  /** 折叠 → 滑出 150ms → 再 90ms 后把手复现、主窗恢复穿透。
   *
   *  ⚠️ 不要在这里加 `if (!pillRevealed) return` 之类的短路：即使胶囊从未滑入
   *  （指针 60ms 内刷过把手就离开，把手已把自己隐去，而 peek-hover 被防抖抑制
   *  或被随后的 peek-leave 取代），也必须照常走到 emitPeekShow —— 这是把手唯一
   *  的「复活」路径。少了它，把手会永久停在 opacity:0 + pointer-events:none，
   *  胶囊再也无法被唤醒（等于应用不可达）。 */
  async function dockPill() {
    // 拖拽期间绝不允许收起链路介入：collapsePill/syncPeek 会把窗口拽回边缘。
    if (pillDragActive) return;
    const gen = ++peekGen;
    clearMiniTimer();
    pillCollapsing = true;
    await collapsePill();
    pillRevealed = false;
    setTimeout(async () => {
      pillCollapsing = false;
      if (gen !== peekGen) return;
      // 先取回最新贴靠边（用户可能刚把胶囊拖到屏幕另一侧），再把边随
      // peek-show 发给把手页，让它纠正圆角朝向并复现自己。
      await syncPeek(); // docked=true：把手接回鼠标
      if (gen !== peekGen) return;
      void emitPeekShow(pillSide).catch(() => {});
    }, PILL_SLIDE_MS + PILL_PEEK_BACK_MS);
  }
```
NEW：
```ts
  /** 时间线重叠收起：t=0 滑出开始（200ms ease-dock-exit，窗口暂不缩）；t=120ms
   *  滑出未结束时把手就提前接回鼠标并从屏幕边缘 scaleX 展开；t=200ms 滑出完成
   *  后再缩窗裁剪。三段互相重叠，消除「胶囊瞬消 → 空窗 → 把手突现」的割裂感。
   *
   *  ⚠️ 不要在这里加 `if (!pillRevealed) return` 之类的短路：即使胶囊从未滑入
   *  （指针 60ms 内刷过把手就离开，把手已把自己隐去，而 peek-hover 被防抖抑制
   *  或被随后的 peek-leave 取代），也必须照常走到 emitPeekShow —— 这是把手唯一
   *  的「复活」路径。少了它，把手会永久停在 opacity:0 + pointer-events:none，
   *  胶囊再也无法被唤醒（等于应用不可达）。 */
  async function dockPill() {
    // 拖拽期间绝不允许收起链路介入：collapsePill/syncPeek 会把窗口拽回边缘。
    if (pillDragActive) return;
    const gen = ++peekGen;
    clearMiniTimer();
    pillCollapsing = true;
    pillRevealed = false;
    // t=120ms：先取回最新贴靠边（用户可能刚把胶囊拖到屏幕另一侧），再把边随
    // peek-show 发给把手页，让它纠正圆角朝向、赶在滑出结束前展开自己。
    setTimeout(async () => {
      if (gen !== peekGen) return;
      await syncPeek(); // docked=true：把手接回鼠标
      if (gen !== peekGen) return;
      void emitPeekShow(pillSide).catch(() => {});
    }, PILL_PEEK_LEAD_MS);
    // t=200ms：滑出完成后才缩窗——此时胶囊已完全滑出窗外，裁剪发生在画面外。
    setTimeout(async () => {
      // 先清锁再校验代际：滑出期间被 revealPill 打断也不能把 pillCollapsing
      // 永久卡在 true（否则 peek-hover 永远无法唤醒胶囊）。
      pillCollapsing = false;
      if (gen !== peekGen) return;
      await collapsePill();
    }, PILL_SLIDE_MS);
  }
```

## Task 4 — App.svelte：滑入/滑出缓动分离（L1232-1246）
OLD：
```css
  .pill-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    transition: transform 150ms var(--tum-ease-spring);
  }

  .pill-layer.is-docked[data-side="right"] {
    transform: translateX(100%);
  }

  .pill-layer.is-docked[data-side="left"] {
    transform: translateX(-100%);
  }
```
NEW：
```css
  .pill-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    /* 滑入（唤醒）：transition 读自「变化后」的状态——is-docked 被移除后
       命中本条 spring，回弹滑回。 */
    transition: transform 150ms var(--tum-ease-spring);
  }

  /* 滑出（收起）：命中 is-docked 时按 ease-dock-exit 加速钻边，200ms。 */
  .pill-layer.is-docked {
    transition: transform 200ms var(--tum-ease-dock-exit);
  }

  .pill-layer.is-docked[data-side="right"] {
    transform: translateX(100%);
  }

  .pill-layer.is-docked[data-side="left"] {
    transform: translateX(-100%);
  }
```

## Task 5 — peek/main.ts：把手从屏幕边缘 scaleX 展开（4 小步）
5a OLD：`  host.className = "peek";` → NEW：`  host.className = "peek is-hidden";`
5b OLD：
```css
    .peek {
      width: 7px;
      height: 58px;
      display: flex;
      align-items: center;
      justify-content: center;
      /* 与 --tum-glass 保持一致：透明窗里 backdrop-filter 采不到桌面，
         只用高不透明度深色保证对比度。 */
      background: rgba(22, 25, 31, 0.92);
      border: 1px solid rgba(255, 255, 255, 0.16);
      transition: opacity 110ms ease;
      cursor: default;
    }
```
NEW：在 border 行后加 `transform: scaleX(1);` `transform-origin: right center;`，并把 transition 换为：
```css
      transition: opacity 160ms cubic-bezier(0.33, 1, 0.68, 1),
        transform 160ms cubic-bezier(0.33, 1, 0.68, 1);
```
5c OLD：`    .peek[data-side="left"] { border-radius: 0 4px 4px 0; border-left: none; }`
NEW：`    .peek[data-side="left"] { border-radius: 0 4px 4px 0; border-left: none; transform-origin: left center; }`
5d OLD：`    .peek.is-hidden { opacity: 0; }`
NEW：`    .peek.is-hidden { opacity: 0; transform: scaleX(0.4); }`
约束：L51-54 关于「绝不在此改 pointer-events」的注释必须原样保留。
验证：tsc 编译 + grep scaleX → 2 处。

## Task 6 — ModelPanel.svelte：圆环数据派生（L114 后插入；同时删除 L10 的 PillsOrSelect import）
6a OLD：
```ts
  import PillsOrSelect from "./PillsOrSelect.svelte";
  import TrendLineChart from "./TrendLineChart.svelte";
```
NEW：
```ts
  import TrendLineChart from "./TrendLineChart.svelte";
```
6b OLD：
```ts
  let active = $derived(modelList.find((m) => m.id === activeId) ?? null);
```
NEW（在该行后追加）：
```ts

  // ---- 用量占比圆环（替代顶部模型选择行）----
  // 长尾过滤：模型 ≤6 个时全部实名；否则保留 Top5 且份额 ≥2%，其余并入「其他」。
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

  let hoverSliceId: string | null = $state(null);

  let ringSlices = $derived.by((): RingSlice[] => {
    const grand = modelList.reduce((s, m) => s + m.total, 0);
    if (grand <= 0) return [];
    const direct =
      modelList.length <= RING_MAX_SLICES + 1
        ? modelList
        : modelList.filter((m, i) => i < RING_MAX_SLICES && m.total / grand >= RING_MIN_SHARE);
    const otherTotal = grand - direct.reduce((s, m) => s + m.total, 0);
    const slices: RingSlice[] = direct.map((m, i) => ({
      id: m.id,
      name: m.name,
      total: m.total,
      share: m.total / grand,
      color: i === 0 ? accent : RING_COLORS[(i - 1) % RING_COLORS.length],
      isOther: false,
      members: [],
    }));
    if (otherTotal / grand >= RING_MIN_SHARE) {
      slices.push({
        id: "__other__",
        name: "其他",
        total: otherTotal,
        share: otherTotal / grand,
        color: RING_OTHER_COLOR,
        isOther: true,
        members: modelList.filter((m) => !direct.includes(m)).map((m) => m.name),
      });
    }
    return slices;
  });

  // 手写 SVG 圆环：周长 C=2π×42，每片弧长=份额×C−2px 留缝（最小 1px 防消失），
  // dashoffset=负累计弧长依次排布；配合弧线组 rotate(-90°) 从 12 点方向起笔。
  let ringArcs = $derived.by((): RingArc[] => {
    const C = 2 * Math.PI * 42;
    let cum = 0;
    return ringSlices.map((s) => {
      const arc = Math.max(1, s.share * C - 2);
      const out: RingArc = { ...s, dasharray: `${arc} ${C - arc}`, dashoffset: -cum };
      cum += s.share * C;
      return out;
    });
  });

  // 圆环中心：hover 哪片显示哪片，否则显示份额最高的一片。
  let centerSlice = $derived(ringSlices.find((s) => s.id === hoverSliceId) ?? ringSlices[0] ?? null);
```

## Task 7 — ModelPanel.svelte：模板替换（L179-183 的 PillsOrSelect 块）
NEW：
```svelte
    <div class="mp__ring">
      {#if ringArcs.length > 0}
        <svg
          class="ring"
          viewBox="0 0 110 110"
          role="img"
          aria-label="各模型用量占比"
          onpointerleave={() => { hoverSliceId = null; }}
        >
          <g class="ring__dial">
            <circle class="ring__track" cx="55" cy="55" r="42" />
            {#each ringArcs as arc (arc.id)}
              <circle
                class="ring__seg"
                class:is-hover={hoverSliceId === arc.id}
                class:is-active={!arc.isOther && activeId === arc.id}
                cx="55"
                cy="55"
                r="42"
                stroke={arc.color}
                stroke-dasharray={arc.dasharray}
                stroke-dashoffset={arc.dashoffset}
                onpointerenter={() => { hoverSliceId = arc.id; }}
                onclick={() => {
                  if (arc.isOther) return;
                  activeId = arc.id;
                  writePref("tum.model.tab", arc.id);
                }}
              >
                <title>{arc.isOther ? `其他：${arc.members.join("、")}` : `${arc.name} · ${fmtTokens(arc.total)}（${(arc.share * 100).toFixed(1)}%）`}</title>
              </circle>
            {/each}
          </g>
          <text class="ring__center-num" x="55" y="53">
            {centerSlice ? (centerSlice.isOther ? "其他" : fmtTokens(centerSlice.total)) : "—"}
          </text>
          <text class="ring__center-label" x="55" y="66">
            {centerSlice ? (centerSlice.isOther ? "长尾模型" : centerSlice.name) : "暂无用量"}
          </text>
        </svg>
        <div class="ring-legend">
          {#each ringArcs as arc (arc.id)}
            <button
              type="button"
              class="ring-legend__item"
              class:is-active={!arc.isOther && activeId === arc.id}
              title={arc.isOther ? arc.members.join("、") : `${arc.name} · ${fmtTokens(arc.total)}`}
              disabled={arc.isOther}
              onpointerenter={() => { hoverSliceId = arc.id; }}
              onpointerleave={() => { hoverSliceId = null; }}
              onclick={() => {
                if (arc.isOther) return;
                activeId = arc.id;
                writePref("tum.model.tab", arc.id);
              }}
            >
              <span class="ring-legend__dot" style="background: {arc.color}"></span>
              <span class="ring-legend__name">{arc.name}</span>
              <span class="ring-legend__pct">{(arc.share * 100).toFixed(0)}%</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
```
要点：「其他」切片 disabled、点击不选中（不联动下方走势）；实名切片点击即选中并联动逐日线图。

## Task 8 — ModelPanel.svelte：样式（追加在 .mp__empty--err 之后）
NEW（在原块后追加）：
```css

  /* ---- 用量占比圆环 ---- */
  .mp__ring {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: none;
  }
  .ring {
    width: 118px;
    height: 118px;
    flex: none;
  }
  /* 只旋转弧线组，中心文字保持水平。 */
  .ring__dial {
    transform: rotate(-90deg);
    transform-origin: 55px 55px;
  }
  .ring__track {
    fill: none;
    stroke: var(--tum-border);
    stroke-width: 13;
    opacity: 0.35;
  }
  .ring__seg {
    fill: none;
    stroke-width: 13;
    transition: stroke-width 0.15s ease;
    cursor: pointer;
  }
  .ring__seg.is-hover,
  .ring__seg.is-active {
    stroke-width: 15;
  }
  .ring__center-num {
    font-family: var(--tum-font-mono);
    font-size: 15px;
    font-weight: 600;
    fill: var(--tum-text-primary);
    text-anchor: middle;
    dominant-baseline: middle;
  }
  .ring__center-label {
    font-family: var(--tum-font);
    font-size: 9px;
    fill: var(--tum-text-muted);
    text-anchor: middle;
  }
  .ring-legend {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .ring-legend__item {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    padding: 2px 4px;
    border-radius: var(--tum-radius-xs);
    cursor: pointer;
    font-family: var(--tum-font);
    color: var(--tum-text-secondary);
    text-align: left;
    transition: background 0.15s ease, color 0.15s ease;
  }
  .ring-legend__item:hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .ring-legend__item.is-active {
    color: var(--tum-text-primary);
  }
  .ring-legend__item:disabled {
    cursor: default;
  }
  .ring-legend__dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .ring-legend__name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .ring-legend__pct {
    flex: none;
    font-family: var(--tum-font-mono);
    font-size: 10px;
    color: var(--tum-text-muted);
    font-variant-numeric: tabular-nums;
  }
```

## Task 9 — 构建验证
npm run build（tsc + vite）。预期 0 error。

## Task 10 — 白名单提交
git add 上述白名单文件（排除 ProviderLogo.svelte / release/ / docs/mockups/），中文提交信息 + 仓库惯例尾注。