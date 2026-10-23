# glance 对标 P0 打磨设计

> 分支 `feat/glance-p0-polish` · 依据 [glance-design-research.md](glance-design-research.md) §7 P0 清单 · 2026-10-23

## 目标与范围

把 glance 研究中「参数级可直接移植」的 7 项快赢落地，全部是打磨性质：不改业务逻辑、不加依赖、不动 i18n 语义。7 项各一个 commit，串行推进，每项独立可验证。

**非目标**（P1/P2 范畴，见研究报告 §7）：明暗双主题、字体裁剪、设置搜索/持久化改造、窗口盖整边的贴边架构重做、帧调度引擎。

## 分项设计

### 1. 动效 token 收口

**目标**：动画常数单源化——组件里不再出现裸的时长/曲线/外来色/越界字号。

- `tokens.css` 新增：
  - `--tum-ease-out-quart: cubic-bezier(0.25, 1, 0.5, 1)`（与实现一致的口径，取自 easings.net 的 easeOutQuart；glance 内部形变 1-(1-t)⁴ 的贝塞尔近似）
  - `--tum-dur-settle: 200ms`（落位飞回，供第 4 项使用）
  - `--tum-dur-ring: 420ms`（环形进度一圈时长，从组件字面量上提）
- 字面量清零：
  - `App.svelte` `PILL_SLIDE_MS = 180` → 保留 JS 常量但加注释指向 `--tum-dur-slide-out`，并加一条 vitest 断言 `tokens.css` 中该值为 180ms（双源同步靠测试钉住）。
  - `MiniPanel.svelte:112`、`ProgressRing.svelte:201` 的 420ms → `var(--tum-dur-ring)`。
- 外来色归 token：`Settings.svelte` 5 处 `#ff6b6b` / `#f87171`（行 2702、3797、3807、3862、3868）→ `var(--tum-crit)` 及其 alpha 变体（`color-mix()` 或现有 token）；`rt-line__err` / `rt-op--del` / `rt-dot--err` 类一并核对。
- 越界字号归阶梯：`Settings.svelte` 中 10.5/11.5/12.5/15.5px 等小数字号归入现有阶梯（10/11/13/16）。

**验收**：`grep -n "ff6b6b\|f87171" src/` 零命中；`.5px` 字号零命中；svelte-check + vitest 通过。

### 7. 图标体系（紧随其后，后续项直接用）

**目标**：消灭字符按钮（↻⚙⤢✕—▢），建立单源图标组件。

- 新建 `src/lib/components/atoms/Icon.svelte`：
  - props：`name`、`size = 16`、`strokeWidth = 2`；
  - 内嵌 Lucide SVG path（viewBox 24、`fill="none"`、`stroke="currentColor"`、圆角端点），文件头注明 path 来源与 ISC 许可；
  - 仅内置实际用到的 ~8 枚：`refresh` / `settings` / `maximize` / `x` / `minus` / `square` / `restore` / `pin`。
- 零新依赖：不引 lucide-svelte（仓库 deps 只有 @tauri-apps/api，为 8 个图标引库不值）。
- 替换点（aria-label 与 i18n 不动）：`App.svelte:1477/1512/1527/1528`、`Settings.svelte:1171-1174`、`ToolWindow.svelte:145`、`TrendWindow.svelte:143`、`DetailCard.svelte:339`、`ModelPanel.svelte:425`、`ToolPanel.svelte:263`、`DevicePanel.svelte:216`、`ZoomButton.svelte`。

**验收**：字符按钮 grep（`↻|⚙|⤢|✕`）在组件中零命中；每枚图标带 `aria-hidden="true"`（语义由按钮 aria-label 承担）。

### 2. 微交互手感

**目标**：hover/按压/滑块手感对齐 glance 的 6%/240ms/按压反馈。

- `--tum-surface-hover` 0.085 → 0.06（glance 暗色 hover 6%，全局生效一处改）。
- `PillsOrSelect` 分段选择 thumb 位移接 `--tum-dur-slow`（240ms）+ `--tum-ease-slide-in`。
- 按钮按压反馈：`transform: scale(0.97)`（active 态，transition 80ms），只加在 icon button 与胶囊按钮。
- 悬浮钮 LINGER 500ms：核对 peek-reveal 链路，悬浮出现的按钮（如穿透恢复钮）出现前要求指针停留 ≥500ms，防误触（glance 常数）。

**验收**：肉眼走查 + svelte-check；LINGER 常数放 `tokens.css` 注释或常量文件，不散落。

### 6. 全局焦点环 + 各窗 Esc

**目标**：键盘可达性兜底。

- `tokens.css` 加 `:where(:focus-visible)` 兜底环（`outline: 2px solid var(--tum-accent-stroke)` + 2px offset）——零特异性，已做组件级焦点环的 8 处样式自然胜出。
- 核对 `ToolWindow.svelte` / `TrendWindow.svelte` 的 Esc 关闭；缺失则补（仅窗口页，透明 dashboard 不响应 Esc）。

### 5. 设置关闭拦截

**目标**：有未保存更改时 ✕ / Esc 不再静默丢弃。

- `Settings.svelte:1110` `handleClose`：`hasDraft > 0` 时改为弹内联确认条（复用页脚区域，不新增弹窗层）：
  - 「保存并关闭」→ `persistSettings()` + 关闭（焦点默认落此，主按钮样式）
  - 「放弃并关闭」→ 复用 `handleDiscard()`（refreshAll 重读）+ 关闭
  - 「取消」→ 关闭确认条，回到表单
- `Settings.svelte:1157` Esc：仅剩 picker/presetMenu 打开时维持原状；否则 `hasDraft > 0` 弹确认条，再按 Esc/点取消关闭。
- `Settings.svelte:2382` 页脚「保存并关闭」保持直通（它本身就是保存动作）。
- i18n：新增 4 键（确认条标题 + 三按钮文案），中英同补。

**验收**：改一项设置 → ✕ 出确认条；三按钮行为各测一遍；无更改时 ✕ 直通；svelte-check + vitest。

### 3. 数字补间

**目标**：用量/费用数字滚动感（glance「画显示值」思想）。

- 新建 `src/lib/motion/num.ts`：纯函数 + rAF，上升近瞬时（τ≈60ms 指数趋近）、下落线性限速（每秒最多回落配置比例，调用点配）。非对称曲线是刻意设计（涨要跟手、跌要可读），不用 svelte/motion（锁文件 Tween 版本不确定，且框架补间不适配非对称曲线）。
- 接入：胶囊用量/费用（`App.svelte`）、`Stat.svelte`；`.tum-numeric`（tabular-nums）已全局，无需再动。
- 测试：vitest 注入虚拟时钟直测收敛与限速行为。

**验收**：util 测试覆盖（收敛、下落限速、目标回跳）；肉眼确认胶囊数字滚动。

### 4. 把手 / 落位动画（最险，殿后）

**目标**：消灭两处瞬移——把手窗出现位置、拖拽松手后的落位。

- **把手跟随（Rust 侧）**：`ipc.rs:689` `reposition_peek` 由一次性 `set_position` 改为指数趋近步进：每 8ms 一步（glance FOLLOW_MS），`α = 1 - exp(-dt/τ)`，τ=60ms，位移 <0.5px 收手；步进器抽纯函数放 `dock.rs` 或独立模块，cargo 单测。总时长封顶 ~300ms 防长尾。
- **拖拽落位 FLIP（前端）**：拖拽松手 → Rust 贴边后，胶囊层以 `--tum-dur-settle`（200ms）+ `--tum-ease-out-quart` 从松手位置飞回槽位（FLIP：先量松手时窗口位、贴边后反向 transform 再归零），替代「贴边瞬移 + 原地动画」。
- **降级预案**：若实测发抖（IPC/调度抖动），把手跟随退回显隐淡入淡出（`--tum-dur-fade`），落位 FLIP 退回现状，并在 commit message 记录实测数据。

**验收**：cargo test（步进纯函数）+ 手测拖拽/呼出/跨边重建三链路；perf_probe 分段计时无回归。

## commit 顺序与验证

```
1 tokens 收口 → 7 图标 → 2 微交互 → 6 焦点环/Esc → 5 关闭拦截 → 3 数字补间 → 4 把手/落位
```

- 图标（7）提前：后续按压反馈、确认条按钮都要用到。
- 第 4 项最险（跨 Rust/前端 + 位置动画），单独殿后，失败可整体回退不影响前 6 项。
- 每 commit：`npm test` + `npx svelte-check`（0 错 0 警）；第 4 项另过 `cargo test`。

## 风险

| 风险 | 缓解 |
| --- | --- |
| 把手步进动画在真实 IPC 链路上抖动 | 纯函数先单测；实测抖则按降级预案退回淡入淡出 |
| `--tum-surface-hover` 全局改动影响对比度 | 走查暗色下所有 hover 面；0.085→0.06 是 glance 实测值，方向是更收敛 |
| FLIP 与 dockPill 现有 timeline 打架 | FLIP 只作用于「拖拽落位」路径，hover 呼出路径不动 |
| 字面量双源（JS 180ms vs CSS token）漂移 | vitest 断言钉住 |
