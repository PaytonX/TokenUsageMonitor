# 设计文档：全局焦点 + 迷你胶囊 + 出屏钳制 + 靠边淡出

- 日期：2026-09-15
- 状态：设计已经用户逐项确认（Q1–Q4、实现路径、两轮设计稿），待评审
- 对象：TokenUsageMonitor（Tauri 2.x 单窗口，dashboard / compact 双形态，透明、置顶、无边框）

## 1. 背景与问题

用户报告四类问题：

1. 多 provider 并存时，热力图与百分比圆环无法指定"看哪一个"——圆环恒为聚合值；热力图内层 tabs 只在"该 provider 有热力图数据"时出现（MiniMax 永远没有）。
2. 收起后的小窗希望有更美观、更紧凑的形态。
3. 希望小窗靠边时能自动"收敛"（弱化存在感）。
4. 小窗停在屏幕边缘再展开时，dashboard 有一半跑到屏幕外（`set_window_mode` 只 `set_size` 不重定位，Tauri 无自动钳制）。

## 2. 决策记录

| # | 问题 | 用户决策 |
|---|------|----------|
| Q1 | provider 选择放在哪一层 | **方案 A：全局焦点**——dashboard 顶部加 chips 行，选择结果同时驱动圆环、热力图与胶囊 |
| Q2 | compact 形态选型 | **样式②：胶囊贴纸**，约 150×44 单行 |
| Q3 | 贴边行为 | **③：停留淡出**——贴边且无操作 2.5s 后淡出，位置不变（不采用半隐/缩圆点） |
| Q4 | 胶囊上能否切换 provider | **D：不切换**，胶囊跟随全局焦点显示 |
| 路径 | 实现分层 | **方案一：前端为主**——焦点/胶囊/淡出全在 App.svelte；后端只做窗口管理（尺寸、钳制、启动钳制） |

## 3. 详细设计

### 3.1 全局焦点（focus）

状态与持久化

- App.svelte 新增 `focus: string`：取值 `"all"` 或 provider id（`minimax` / `deepseek` / `volcengine`，与后端 `KNOWN_PROVIDER_IDS` 对齐）。
- 持久化到 `localStorage`，key：`tum.focus`。首次使用默认 `"all"`（保持现有聚合行为，零感知升级）。
- 校验与回退：`focus` 非 `"all"` 且不在当前 snapshots 的 id 集合内 → 回退 `"all"`（provider 被停用/消失时自动回退，沿用现有 fallback `$effect` 思路，作用对象改为 focus）。
- **校验时机**：仅当 snapshots 非空时执行校验。避免启动竞态——快照尚未从后端到达时若用空集合校验，会把有效的记忆值误回退并覆盖存储。

UI：chips 行

- 位置：header 之下、热力图卡片之上；`snapshots` 为空时整行隐藏。
- 结构：第一个 chip 为「全部」，其后按 snapshots 顺序逐 provider；样式复用 `.heatmap__tab` / `--active` 激活态。
- chip 内容 = 色点 + 名称，色点取 `PROVIDER_COLORS`：

```ts
const PROVIDER_COLORS: Record<string, string> = {
  minimax: "#ff5c5c",
  deepseek: "#4d6bfe",
  volcengine: "#12b76a",
};
const FOCUS_FALLBACK_COLOR = "#8a8f98";
```

（区分度优先于品牌色；未知 id 用 fallback 灰。）

圆环联动

- `focus === "all"` → 维持现有聚合逻辑 `Math.min(...remainingPercent)`。
- `focus === 具体 provider` → 该 snapshot 的 `remainingPercent`；snapshot 缺失 → 圆环显示 `--`（进度 0 + 中心文字 `--`）。

热力图联动

- `focus === "all"` → 现有内层 tabs 行为不变。内层激活 tab 状态由 `activeProviderId` 更名为 `heatmapTabId`，职责收窄为"聚合视角下的内层 tab"；其 fallback `$effect` 保留原逻辑。
- `focus === 具体 provider`：
  - 该 provider 有热力图数据 → 直接渲染其热力图，隐藏内层 tabs；
  - 无热力图数据（如 MiniMax）→ 热力图区域显示 muted 占位文案：「该来源暂无热力图数据」。

### 3.2 迷你胶囊（compact 重设计）

尺寸与布局

- `ipc.rs::set_window_mode` compact 分支：96×136 → **150×44**（逻辑像素）。
- 单行布局：`[迷你圆环 24px / stroke 3] [百分比] ｜ [色点] [provider 名] [✕]`
  - `focus === "all"` → 色点用 `FOCUS_FALLBACK_COLOR`、名称「全部」、百分比 = 聚合 min；
  - snapshots 为空或对应 snapshot 缺失 → 百分比显示 `--`；
  - 百分比规则与 3.1 一致。
- `.shell--compact` 相应重写：单行 flex、`border-radius: 22px`、收紧 padding，保留毛玻璃背景。

交互（自实现，禁用 `data-tauri-drag-region`——它会在 mousedown 接管拖拽并吞掉 click）

- `pointerdown`：记录起点坐标与 pointerId，`didDrag = false`；
- `pointermove`（按住中）位移 > **4px** → 调 `getCurrentWindow().startDragging()`（仅一次），置 `didDrag = true`；此后由系统接管拖拽，逻辑不依赖后续 pointer 事件；
- `pointerup` 且 `!didDrag` → `toggleMode()` 展开为 dashboard；
- `✕` 按钮：hover 胶囊时显现；`pointerdown` / `click` 均 `stopPropagation()`；click → 退出应用（复用现有关闭逻辑）；
- 胶囊不提供 provider 切换（Q4 = D）。

### 3.3 出屏钳制（clamp_window_to_work_area）

新增 Rust 内部函数（ipc.rs）：`clamp_window_to_work_area(window, margin)`，margin = 8.0 逻辑像素：

1. `outer_position()`（物理 px）+ `current_monitor()`；
2. 工作区优先取 `Monitor::work_area()`；为 None → 退回 `monitor.size() / position()`；monitor 为 None → 直接返回；
3. margin × `scale_factor()` 转物理 px；`x = clamp(x, wa.x + margin, wa.x + wa.width − w − margin)`，y 同理；若窗口宽/高超过工作区 − 2×margin，钉在 min 侧；
4. 位置有变化才 `set_position()`（避免无谓移动）。

触发点

- `set_window_mode` 两个分支在 `set_size` 之后调用（展开、收起都可能出屏——问题④的主场景就是"边缘收起 → 展开"）；
- 启动钳制（lib.rs setup）：注册窗口事件监听，首次 `WindowEvent::Focused(true)` 时执行钳制并立即 unlisten。理由：`tauri_plugin_window_state`（仅 `StateFlags::POSITION`）恢复位置的时机在插件初始化路径上，`Focused(true)` 是"位置已恢复、窗口已可见"的稳妥信号；
- JS 侧不参与钳制；`@tauri-apps/api` 的 `onMoved` 仅服务 3.4。

### 3.4 靠边淡出（idle fade，仅胶囊）

贴边判定

- `getCurrentWindow().onMoved`（PhysicalPosition）+ `currentMonitor()`；
- 边界基准：优先 JS 侧 `Monitor.workArea`，不可用退回 monitor bounds；
- 窗口外沿距任一边 ≤ **24 逻辑 px**（× scaleFactor 转物理比较）→ near-edge（距离为负即已出屏，同样计入）。

状态机

- 进入：near-edge && 指针不在窗口内 && 该状态持续 **2.5s** → 根节点加 `.is-faded`；
- 退出（任一）：`pointerenter` 窗口 / 移动后不再 near-edge / 开始拖拽 / 切回 dashboard；
- `.is-faded`：`opacity: 0.22; transform: scale(0.9); transition: opacity .35s ease, transform .35s ease;`
- opacity 不影响命中测试：淡出态仍可点击/悬停，指针进入即恢复（这是主要恢复途径，无需额外引导）；
- dashboard 模式永不淡出。

实现位置

- App.svelte 单个 `$effect`：`mode === "compact"` 时注册 `onMoved` + 窗口级 `pointerenter/leave`，清理函数反注册；2.5s 用 `setTimeout`（条件变化时先 clear）。

## 4. 改动面

| 文件 | 改动 |
|------|------|
| src/App.svelte（主） | focus 状态 + localStorage + chips 行；`activeProviderId` → `heatmapTabId` 更名；圆环/热力图联动；compact 模板重写为胶囊（拖拽/点击/✕）；淡出状态机 |
| src-tauri/src/ipc.rs | `set_window_mode` compact 96×136 → 150×44；两分支 `set_size` 后接钳制；新增 `clamp_window_to_work_area` |
| src-tauri/src/lib.rs | setup 中注册首次 `Focused(true)` → 启动钳制 |

- 原则上前端逻辑集中在 App.svelte，不新增文件；若实现中拖拽/淡出逻辑膨胀，允许拆出 `src/lib/windowing.ts`。

## 5. 范围外（本次明确不做）

1. 胶囊上切换 provider（Q4 = D）。
2. settings 中遗留的 `dashboard_x / dashboard_y` 字段清理（window-state 插件 POSITION 接管后已闲置）。
3. 运行中显示器热插拔 / 改分辨率的实时重钳制。
4. 贴边半隐、缩为圆点（Q3 落选方案）。
5. 后端聚合 / 快照接口改造（焦点是纯前端推导）。
6. `.gitignore` 补 `src-tauri/target/`（建议另行处理：当前暂存区混有 target 构建产物）。

## 6. 已知限制与风险

- 显示器热插拔：运行中不重钳制（无 `onMoved` 触发），重启后由启动钳制兜底。
- 启动钳制依赖 `Focused(true)`：若窗口始终不获得焦点则不钳制；用户一旦点击窗口即触发，可接受。
- work_area 缺失时退回显示器全边界，理论上可能压到任务栏（Windows 主平台正常返回 work_area，影响有限）。
- 拖拽中不做钳制：在边缘松手可能残留部分出屏；用户点击胶囊展开时会被展开钳制拉回完整可见（自愈路径成立）。
- 淡出窗口仍响应点击（设计意图），低对比可能让用户误以为无响应——0.22 保留轮廓可辨识。
- 4px / 24px / 2.5s / 0.22 / 0.9 均为基准值，实现后可按手感微调，不需再次评审。
- window-state 插件若调整恢复时机，启动钳制触发点（首次 `Focused(true)`）需复查。

## 7. 验收标准

- [ ] 多 provider：chips 行出现；聚焦单个 provider 后圆环、热力图、胶囊三者同步。
- [ ] 「全部」恢复聚合 min 与内层 tabs 行为。
- [ ] 重启后 focus 记忆保留；localStorage 写入脏 id → 快照到达后回退 `"all"`。
- [ ] 聚焦 MiniMax：热力图区显示占位文案，圆环/胶囊正常。
- [ ] compact 窗口 150×44；点击展开 / 拖动移动 / hover ✕ 关闭三者互不误触。
- [ ] 在屏幕右缘收起再展开，dashboard 完整位于工作区内（≥8px 边距）。
- [ ] 手动将窗口移出屏幕后重启，首次聚焦窗口即被钳回工作区。
- [ ] 胶囊贴边 2.5s 淡出（.22 / .9），指针进入立即恢复；dashboard 从不淡出。
- [ ] `cargo test`（src-tauri 下）与 `npm run build` 通过；`npx tauri build --no-bundle` 通过。
