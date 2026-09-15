# 全局焦点 + 迷你胶囊 + 工作区钳制 + 靠边淡出 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让圆环/热力图/胶囊可全局选定来源（含「全部」聚合），把 compact 形态重做为 150×44 可拖拽迷你胶囊（靠边 2.5s 自动淡出），并保证窗口永不悬出屏幕工作区。

**Architecture:** 前端（Svelte 5 runes）承担焦点状态、chips 行、胶囊 UI、拖拽手势与淡出状态机；后端（Rust/Tauri）只做窗口几何——`clamp_rect` 纯函数 + `clamp_window_to_work_area` 应用函数，在切换形态与启动首次聚焦时把窗口钳回 `Monitor::work_area` 之内。

**Tech Stack:** Tauri 2.11.5（vendored，Rust 后端）、Svelte 5 runes + TypeScript、Vite；测试为 Rust `#[test]` + `svelte-check`（无 vitest）

**Spec:** `docs/superpowers/specs/2026-09-15-focus-provider-and-pill-design.md`（commit `ff668a1`）

---

## 已核实的 API 事实（全部对 vendored 源码 / 本仓库源码核实过）

| # | API / 现状 | 事实 | 来源 |
| --- | --- | --- | --- |
| 1 | 主窗口 label | 是 **`"dashboard"`**，不是 `"main"` | `src-tauri/src/ipc.rs:161` |
| 2 | `Monitor::work_area()` | 返回 `&PhysicalRect<i32, u32>`（**引用，非 Option**）；`PhysicalRect { position: PhysicalPosition<i32>, size: PhysicalSize<u32> }` | `tauri-2.11.5/src/window/mod.rs:96`、`tauri-runtime-2.11.3/src/dpi.rs:28` |
| 3 | `WebviewWindow::on_window_event` | 签名 `F: Fn(&WindowEvent) + Send + 'static`，**返回 `()`，没有 unlisten handle** → 一次性逻辑必须用 `AtomicBool::swap` 守卫 | `tauri-2.11.5/src/webview/webview_window.rs:1524` |
| 4 | `outer_position()` / `outer_size()` | `Result<PhysicalPosition<i32>>` / `Result<PhysicalSize<u32>>` | webview_window.rs:1710 / 1724 |
| 5 | `current_monitor()` | `Result<Option<Monitor>>` | webview_window.rs:1812 |
| 6 | `set_position()` | 接受 `Into<Position>`，可传 `PhysicalPosition::new(i32, i32)` | webview_window.rs:2255 |
| 7 | JS `Monitor` | `workArea: { position, size }` 物理像素（类型非空，运行时仍做回退）；`position`/`size`/`scaleFactor` 同步属性 | `node_modules/@tauri-apps/api/window.d.ts:25-69` |
| 8 | JS `onMoved` / `startDragging` | `onMoved(handler) -> Promise<UnlistenFn>`（异步，清理需 disposed 标志防竞态）；`startDragging() -> Promise<void>` | window.d.ts:1232 / 1085 |
| 9 | `UsageSnapshot` 字段 | `provider_id` + **`provider_display_name`**（没有 `provider_name`） | `src/lib/types.ts:37-44` |
| 10 | 热力图 tab class | 字符串插值写法 `class="heatmap__tab {cond ? 'heatmap__tab--active' : ''}"` | `src/App.svelte:243` |
| 11 | `data-tauri-drag-region={false}` | 关闭拖拽的既有模式（Tauri 只检查事件目标元素本身，不看祖先） | `src/App.svelte:200` |
| 12 | `ProgressRing` | props `{ value /*0..1*/, size, stroke, label? }`；`label=""` 渲染无文字圆环 | `src/lib/components/ProgressRing.svelte` |
| 13 | svelte-check | 只在 **error** 时失败；a11y warning 可容忍（现有 `main` 上的 `oncontextmenu` 已产生同类 warning） | 现状 `npm run build` 通过 |

## 环境命令（Windows / PowerShell）

Rust 编译需要 LLVM-MinGW 链接器在 PATH（见 `src-tauri/.cargo/config.toml` 注释），每个 Rust 终端会话先执行一次：

```powershell
$env:PATH = "C:\Users\xiong\AppData\Local\Microsoft\WinGet\Packages\MartinStorsjo.LLVM-MinGW.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\llvm-mingw-20260616-ucrt-x86_64\bin;" + $env:PATH
```

- Rust 测试（在 `src-tauri/` 下）：`cargo test`（完成后预期 8 个测试：signing 3 + clamp 5）
- 前端检查/构建：`npm run build`（= `svelte-check && vite build`）
- 打包验证：`npx tauri build --no-bundle`

**Path-limited 提交（必须）**：staging 区存在 `src-tauri/target/**` 与用户未提交改动的污染，每次提交都必须路径限定：

```powershell
git add <路径...>
git commit -m "<message>" -- <路径...>
```

## 文件改动地图

| 文件 | 动作 | 任务 |
| --- | --- | --- |
| `src-tauri/src/ipc.rs` | 修改：`clamp_rect` 纯函数 + 5 个单元测试（TDD）；`clamp_window_to_work_area`；重写 `set_window_mode`（compact → 150×44，两分支后钳制） | 1, 2 |
| `src-tauri/src/lib.rs` | 修改：`setup` 内 `app.manage(state);`（line 131）之后新增 dashboard 首次聚焦钳制（AtomicBool 一次性守卫） | 3 |
| `src/App.svelte` | 修改：focus 状态 + chips 行（T4）；`activeProviderId` → `heatmapTabId` + 圆环/热力图联动（T5）；胶囊 + 手势 + 淡出（T6） | 4, 5, 6 |

---

### Task 1: `clamp_rect` 纯函数（TDD）

**Files:**
- Modify: `src-tauri/src/ipc.rs`（实现插在 `set_window_mode` 之前；测试追加到文件末尾）

- [ ] **Step 1: 写失败测试** — 追加到 `src-tauri/src/ipc.rs` 文件末尾：

```rust
#[cfg(test)]
mod clamp_tests {
    use super::clamp_rect;

    #[test]
    fn keeps_inside_when_already_visible() {
        let (x, y) = clamp_rect(100.0, 100.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (100.0, 100.0));
    }

    #[test]
    fn pulls_off_screen_window_back_into_view() {
        let (x, y) = clamp_rect(1800.0, 950.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (1552.0, 432.0));
    }

    #[test]
    fn keeps_margin_on_every_edge() {
        let (x, y) = clamp_rect(0.0, 0.0, 360.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (8.0, 8.0));
    }

    #[test]
    fn pins_when_window_wider_than_work_area() {
        let (x, y) = clamp_rect(500.0, 0.0, 2000.0, 600.0, 0.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (8.0, 8.0));
    }

    #[test]
    fn respects_work_area_origin() {
        let (x, y) = clamp_rect(-1920.0, -50.0, 360.0, 600.0, -1920.0, 0.0, 1920.0, 1040.0, 8.0);
        assert_eq!((x, y), (-1912.0, 8.0));
    }
}
```

- [ ] **Step 2: 运行确认失败（红）**

```powershell
$env:PATH = "C:\Users\xiong\AppData\Local\Microsoft\WinGet\Packages\MartinStorsjo.LLVM-MinGW.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\llvm-mingw-20260616-ucrt-x86_64\bin;" + $env:PATH
cd src-tauri
cargo test clamp
```

Expected: 编译失败，`cannot find function clamp_rect in this scope`

- [ ] **Step 3: 写最小实现** — 在 `set_window_mode`（约 line 156）之前插入：

```rust
fn clamp_rect(
    x: f64, y: f64, w: f64, h: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
    margin: f64,
) -> (f64, f64) {
    let max_x = (area_x + area_w - w - margin).max(area_x + margin);
    let max_y = (area_y + area_h - h - margin).max(area_y + margin);
    (x.clamp(area_x + margin, max_x), y.clamp(area_y + margin, max_y))
}
```

- [ ] **Step 4: 运行确认通过（绿）**

```powershell
cargo test clamp
```

Expected: `test result: ok. 5 passed; 0 failed`

- [ ] **Step 5: 提交**

```powershell
cd ..
git add src-tauri/src/ipc.rs
git commit -m "feat: clamp_rect 工作区钳制纯函数与单元测试" -- src-tauri/src/ipc.rs
```

---

### Task 2: `clamp_window_to_work_area` + 重写 `set_window_mode`（150×44）

**Files:**
- Modify: `src-tauri/src/ipc.rs:11`（imports）、`src-tauri/src/ipc.rs:156-182`（`set_window_mode` 整体替换）

- [ ] **Step 1: 扩展 line 11 的 use 语句** 为：

```rust
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WebviewWindowBuilder};
```

（`Emitter` 供 `force_refresh`、`WebviewWindowBuilder` 供 `open_settings` 使用，保留不动。）

- [ ] **Step 2: 在 `clamp_rect` 之后加入应用函数：**

```rust
pub fn clamp_window_to_work_area(window: &WebviewWindow, margin_logical: f64) -> bool {
    let Ok(pos) = window.outer_position() else { return false; };
    let Ok(size) = window.outer_size() else { return false; };
    let Ok(Some(monitor)) = window.current_monitor() else { return false; };
    let area = monitor.work_area();
    let margin = margin_logical * f64::from(monitor.scale_factor());
    let (nx, ny) = clamp_rect(
        f64::from(pos.x),
        f64::from(pos.y),
        f64::from(size.width),
        f64::from(size.height),
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
        margin,
    );
    let changed = nx != f64::from(pos.x) || ny != f64::from(pos.y);
    if changed {
        let _ = window.set_position(PhysicalPosition::new(
            nx.round() as i32,
            ny.round() as i32,
        ));
    }
    changed
}
```

- [ ] **Step 3: 整体替换 `set_window_mode`（约 line 156-182）为：**

```rust
#[tauri::command]
pub async fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    use tauri::LogicalSize;

    let window = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    match mode.as_str() {
        "dashboard" => {
            window
                .set_size(LogicalSize::new(360u32, 600u32))
                .map_err(|e| e.to_string())?;
        }
        "compact" => {
            // Mini pill: single row 150x44 (ring + name + divider + dot + close).
            window
                .set_size(LogicalSize::new(150u32, 44u32))
                .map_err(|e| e.to_string())?;
        }
        other => return Err(format!("unknown window mode: {other}")),
    }

    clamp_window_to_work_area(&window, 8.0);
    Ok(())
}
```

- [ ] **Step 4: 全量测试 + 编译验证**

```powershell
cd src-tauri
cargo test
```

Expected: `test result: ok. 8 passed; 0 failed`（signing 3 + clamp 5；新代码编译通过本身即是验证）

- [ ] **Step 5: 提交**

```powershell
cd ..
git add src-tauri/src/ipc.rs
git commit -m "feat: 工作区钳制应用与胶囊窗口尺寸 150x44" -- src-tauri/src/ipc.rs
```

---

### Task 3: 启动时首次聚焦钳制（lib.rs）

**Files:**
- Modify: `src-tauri/src/lib.rs:131`（`app.manage(state);` 之后、调度器 spawn 之前）

- [ ] **Step 1: 在 setup 闭包内 `app.manage(state);`（line 131）之后插入：**

注意：`on_window_event` 没有 unlisten handle（已核实事实 #3），用 `AtomicBool::swap` 保证只钳制一次；window label 是 `"dashboard"`（事实 #1）。

```rust
        // One-shot startup clamp: the first time the dashboard gains focus,
        // pull it back inside the monitor's work area. on_window_event has no
        // unlisten handle, so guard with an AtomicBool swap.
        let dash = app
            .get_webview_window("dashboard")
            .expect("dashboard window must exist");
        let startup_clamped = std::sync::atomic::AtomicBool::new(false);
        let dash_for_clamp = dash.clone();
        dash.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Focused(true))
                && !startup_clamped.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                ipc::clamp_window_to_work_area(&dash_for_clamp, 8.0);
            }
        });
```

- [ ] **Step 2: 编译检查**

```powershell
cd src-tauri
cargo check
```

Expected: `Finished ...` 无 error

- [ ] **Step 3: 提交**

```powershell
cd ..
git add src-tauri/src/lib.rs
git commit -m "feat: 启动时首次聚焦钳制窗口位置" -- src-tauri/src/lib.rs
```

---

### Task 4: 全局焦点状态 + 来源 chips 行（App.svelte）

**Files:**
- Modify: `src/App.svelte:29`（常量与状态）、`src/App.svelte:133`（effect 之后）、`src/App.svelte:218-220`（模板 chips 行）、`src/App.svelte:452`（CSS）

- [ ] **Step 1: 在 `let activeProviderId = $state<string | null>(null);`（line 29）之后插入：**

```ts
  const PROVIDER_COLORS: Record<string, string> = {
    minimax: "#ff5c5c",
    deepseek: "#4d6bfe",
    volcengine: "#12b76a",
  };
  const FOCUS_FALLBACK_COLOR = "#8a8f98";
  const FOCUS_KEY = "tum.focus";

  // Global focus: "all" (aggregate min) or one provider_id. Drives the
  // header ring, the chips row and the heatmap panel.
  let focus = $state(localStorage.getItem(FOCUS_KEY) ?? "all");
```

- [ ] **Step 2: 在 fallback `$effect`（line 127-133）之后插入两个 effect：**

```ts
  // Persist focus across restarts.
  $effect(() => {
    localStorage.setItem(FOCUS_KEY, focus);
  });

  // If the focused provider disappears (disabled in Settings), fall back to
  // "all". Skipped while snapshots is still empty (startup race).
  $effect(() => {
    if (snapshots.length === 0) return;
    if (focus !== "all" && !snapshots.some((s) => s.provider_id === focus)) {
      focus = "all";
    }
  });
```

- [ ] **Step 3: 在 `</header>`（line 218）与 `<section class="shell__cards">`（line 220）之间插入 chips 行：**

```svelte
    {#if snapshots.length > 0}
      <div class="focus-row" data-tauri-drag-region={false}>
        <button
          class="heatmap__tab"
          class:heatmap__tab--active={focus === "all"}
          onclick={() => (focus = "all")}
        >全部</button>
        {#each snapshots as snap (snap.provider_id)}
          <button
            class="heatmap__tab"
            class:heatmap__tab--active={focus === snap.provider_id}
            onclick={() => (focus = snap.provider_id)}
          >
            <span
              class="focus-dot"
              style:background={PROVIDER_COLORS[snap.provider_id] ?? FOCUS_FALLBACK_COLOR}
            ></span>
            {snap.provider_display_name}
          </button>
        {/each}
      </div>
    {/if}
```

- [ ] **Step 4: 在 `.heatmap__tab--active` 规则（line 448-452）之后插入 CSS：**

```css
  .focus-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .focus-row .heatmap__tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .focus-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }
```

- [ ] **Step 5: 验证**

```powershell
npm run build
```

Expected: svelte-check 0 errors、vite build 成功

- [ ] **Step 6: 提交**

```powershell
git add src/App.svelte
git commit -m "feat: 全局焦点状态与来源切换 chips" -- src/App.svelte
```

---

### Task 5: 圆环/热力图联动全局焦点（`activeProviderId` → `heatmapTabId`）

**Files:**
- Modify: `src/App.svelte`（全文 8 处重命名：lines 29、46、47、57、129、131、243、244；派生替换 lines 102-105；ring 值替换 lines 199、212；热力图区整体替换 lines 236-254；CSS 新增 `.heatmap__empty`）

- [ ] **Step 1: 全文把 `activeProviderId` 重命名为 `heatmapTabId`**（共 8 处：29 声明、46/47 onMount 初始化、57 onUsageUpdated、129/131 fallback effect、243/244 热力图 tabs）。重命名后 fallback effect（原 lines 127-133）应为：

```ts
  // Ensure heatmapTabId always points at a snapshot that exists &
  // (preferably) has heatmap data, so the heatmap tab is never orphaned.
  $effect(() => {
    if (snapshots.length === 0) return;
    const stillExists = snapshots.some((s) => s.provider_id === heatmapTabId);
    if (!stillExists) {
      heatmapTabId = snapshotsWithHeatmap[0]?.provider_id ?? snapshots[0].provider_id;
    }
  });
```

- [ ] **Step 2: 将 `activeSnapshot` 派生（lines 102-105）整体替换为：**

```ts
  // Snapshot behind the global focus ("all" → aggregate; one provider → it).
  let focusedSnapshot = $derived(
    focus === "all"
      ? null
      : (snapshots.find((s) => s.provider_id === focus) ?? null),
  );
  let ringPercent = $derived(
    focusedSnapshot ? remainingPercent(focusedSnapshot) : aggregateRemaining,
  );
  let ringLabel = $derived(
    focusedSnapshot
      ? `${Math.round(remainingPercent(focusedSnapshot) * 100)}%`
      : aggregateLabel,
  );

  // The active snapshot whose heatmap is shown in the bottom panel.
  let activeSnapshot = $derived(
    focusedSnapshot ??
    snapshots.find((s) => s.provider_id === heatmapTabId) ??
    snapshots[0] ??
    null,
  );
```

- [ ] **Step 3: 两处 ProgressRing 的 value/label 都改为 `ringPercent`/`ringLabel`：**

compact 处（line 199，size 56/stroke 4 保持不变）：

```svelte
      <ProgressRing value={ringPercent} label={ringLabel} size={56} stroke={4} />
```

头部处（line 212，size 28/stroke 3 保持不变）：

```svelte
        <ProgressRing value={ringPercent} label={ringLabel} size={28} stroke={3} />
```

- [ ] **Step 4: 热力图区（lines 236-254）整体替换为：**

```svelte
    {#if snapshots.length > 0 && (hasHeatmap || focus !== "all")}
      <section class="shell__heatmap" data-tauri-drag-region={false}>
        <div class="heatmap__head">
          <span class="heatmap__title">日历热力图</span>
          {#if focus === "all"}
            <div class="heatmap__tabs">
              {#each snapshotsWithHeatmap as snap (snap.provider_id)}
                <button
                  class="heatmap__tab {snap.provider_id === heatmapTabId ? 'heatmap__tab--active' : ''}"
                  onclick={() => (heatmapTabId = snap.provider_id)}
                  title={snap.provider_display_name}
                >{snap.provider_display_name}</button>
              {/each}
            </div>
          {/if}
        </div>
        {#if activeSnapshot && activeSnapshot.heatmap}
          <HeatmapGrid cells={activeSnapshot.heatmap} unit={heatmapUnit} />
        {:else if focus !== "all"}
          <div class="heatmap__empty">该来源暂无热力图数据</div>
        {/if}
      </section>
    {/if}
```

- [ ] **Step 5: 在 `.heatmap__tab--active` 规则（Task 4 已在其后插入 focus-row CSS）之后追加：**

```css
  .heatmap__empty {
    padding: var(--tum-space-4) 0;
    text-align: center;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
  }
```

- [ ] **Step 6: 验证 + 提交**

```powershell
npm run build
```

Expected: 0 errors

```powershell
git add src/App.svelte
git commit -m "feat: 圆环与热力图联动全局焦点" -- src/App.svelte
```

---

### Task 6: 迷你胶囊 + 拖拽手势 + 靠边淡出（App.svelte）

**Files:**
- Modify: `src/App.svelte`（script：常量/派生/手势函数/`toggleMode`/onMoved effect；模板 lines 197-204 整体替换；CSS：lines 285-286 注释更新、lines 490-545 块整体替换）

- [ ] **Step 1: 在 Task 4 插入的 `let focus = $state(...)` 之后加入胶囊常量：**

```ts
  const PILL_DRAG_THRESHOLD_PX = 4;
  const PILL_FADE_DELAY_MS = 2500;
  const PILL_EDGE_THRESHOLD_PX = 24;
```

- [ ] **Step 2: 在 Task 4 的两个 `$effect`（focus 持久化 + 校验回退）之后、`let timeLabel` 之前，加入胶囊派生、淡出状态机与拖拽手势：**

```ts
  // Mini pill (compact mode) — focused provider color/name.
  const focusedColor = $derived(
    focusedSnapshot
      ? (PROVIDER_COLORS[focusedSnapshot.provider_id] ?? FOCUS_FALLBACK_COLOR)
      : FOCUS_FALLBACK_COLOR,
  );
  const focusedName = $derived(
    focusedSnapshot ? focusedSnapshot.provider_display_name : "全部",
  );

  // Pill fade machine (spec §3.4): fade to 22% after 2.5s while parked near
  // a screen edge in compact mode; pointer enter / leaving the edge /
  // dashboard mode restores full opacity.
  let pillHovered = $state(false);
  let pillFaded = $state(false);
  let pillFadeTimer: ReturnType<typeof setTimeout> | null = null;
  let pillDragStart: { x: number; y: number } | null = null;
  let pillDidDrag = false;

  function clearPillFadeTimer() {
    if (pillFadeTimer !== null) {
      clearTimeout(pillFadeTimer);
      pillFadeTimer = null;
    }
  }

  async function pillNearEdge(): Promise<boolean> {
    const win = getCurrentWindow();
    const [pos, size, monitor] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      win.currentMonitor(),
    ]);
    if (!monitor) return false;
    const threshold = PILL_EDGE_THRESHOLD_PX * monitor.scaleFactor;
    const area = monitor.workArea ?? { position: monitor.position, size: monitor.size };
    const left = pos.x - area.position.x;
    const top = pos.y - area.position.y;
    const right = area.position.x + area.size.width - (pos.x + size.width);
    const bottom = area.position.y + area.size.height - (pos.y + size.height);
    return Math.min(left, top, right, bottom) < threshold;
  }

  async function refreshPillFade() {
    clearPillFadeTimer();
    if (mode !== "compact" || pillHovered) {
      pillFaded = false;
      return;
    }
    if (!(await pillNearEdge())) {
      pillFaded = false;
      return;
    }
    pillFadeTimer = setTimeout(() => {
      pillFaded = !pillHovered;
    }, PILL_FADE_DELAY_MS);
  }

  function onPillPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    pillDragStart = { x: event.clientX, y: event.clientY };
    pillDidDrag = false;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function onPillPointerMove(event: PointerEvent) {
    if (!pillDragStart) return;
    const dx = event.clientX - pillDragStart.x;
    const dy = event.clientY - pillDragStart.y;
    if (!pillDidDrag && Math.hypot(dx, dy) > PILL_DRAG_THRESHOLD_PX) {
      pillDidDrag = true;
      clearPillFadeTimer();
      pillFaded = false;
      void getCurrentWindow().startDragging();
    }
  }

  function onPillPointerUp() {
    const didDrag = pillDidDrag;
    pillDragStart = null;
    pillDidDrag = false;
    if (!didDrag) {
      void toggleMode();
    } else {
      void refreshPillFade();
    }
  }
```

- [ ] **Step 3: 将 `toggleMode`（lines 161-165）整体替换为：**

```ts
  async function toggleMode() {
    const next: Mode = mode === "dashboard" ? "compact" : "dashboard";
    await setWindowMode(next);
    mode = next;
    pillHovered = false;
    pillFaded = false;
    clearPillFadeTimer();
    if (next === "compact") {
      void refreshPillFade();
    }
  }
```

- [ ] **Step 4: 紧跟 Step 2 的函数块之后加入 onMoved 监听 effect：**

`onMoved` 异步 resolve（事实 #8）；用 disposed 标志保证 teardown 早于 resolve 时也能正确 unlisten。

```ts
  // Window moved (OS drag or external) → re-evaluate the pill fade state.
  $effect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void win
      .onMoved(() => {
        void refreshPillFade();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  });
```

- [ ] **Step 5: 将 compact 模板块（lines 197-204）整体替换为：**

```svelte
  {#if mode === "compact"}
    <div
      class="pill"
      class:is-faded={pillFaded}
      onpointerenter={() => {
        pillHovered = true;
        clearPillFadeTimer();
        pillFaded = false;
      }}
      onpointerleave={() => {
        pillHovered = false;
        void refreshPillFade();
      }}
      onpointerdown={onPillPointerDown}
      onpointermove={onPillPointerMove}
      onpointerup={onPillPointerUp}
      oncontextmenu={(e) => e.preventDefault()}
    >
      <div class="pill__ring">
        <ProgressRing value={ringPercent} label="" size={24} stroke={3} />
        <span class="pill__percent">{ringLabel}</span>
      </div>
      <span class="pill__divider"></span>
      <span class="pill__dot" style:background={focusedColor}></span>
      <span class="pill__name">{focusedName}</span>
      <button
        class="pill__close"
        title="关闭应用"
        onpointerdown={(e) => e.stopPropagation()}
        onclick={(e) => {
          e.stopPropagation();
          void closeApp();
        }}
      >✕</button>
    </div>
  {:else}
```

（胶囊不加 `data-tauri-drag-region` —— 拖拽由 pointer 手势 + `startDragging()` 实现；✕ 的 `stopPropagation` 防止触发胶囊的点击/拖拽。）

- [ ] **Step 6: 更新 `.shell--compact` 的过期注释（lines 285-286）为：**

```css
  /* In compact mode, remove the shell's padding and gap so the mini pill
     gets the full window area (150x44). */
```

- [ ] **Step 7: 将 compact CSS 块（lines 490-545：注释 + `.compact` / `.compact__actions` / `.compact__btn` 及其 hover 变体）整体替换为：**

```css
  /* Mini pill (compact) — single row 150x44 (see ipc::set_window_mode
     "compact"). Background matches the old compact__btn fill. */
  .pill {
    height: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: 22px;
    background: rgba(10, 14, 26, 0.9);
    border: 1px solid var(--tum-border);
    transition: opacity 0.35s ease, transform 0.35s ease;
    cursor: default;
    overflow: hidden;
  }

  .pill.is-faded {
    opacity: 0.22;
    transform: scale(0.9);
  }

  .pill__ring {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .pill__percent {
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font-mono);
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
  }

  .pill__divider {
    width: 1px;
    height: 20px;
    background: var(--tum-border-strong);
    flex-shrink: 0;
  }

  .pill__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .pill__name {
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font);
    color: var(--tum-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .pill__close {
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity 0.2s ease, background 0.2s ease, color 0.2s ease;
  }

  .pill:hover .pill__close {
    opacity: 1;
  }

  .pill__close:hover {
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
  }
```

- [ ] **Step 8: 验证 + 提交**

```powershell
npm run build
```

Expected: svelte-check 0 errors（pill div 上的 pointer 事件可能产生 a11y warning，可容忍——见已核实事实 #13）、vite build 成功

```powershell
git add src/App.svelte
git commit -m "feat: 迷你胶囊形态、拖拽手势与靠边淡出" -- src/App.svelte
```

---

### Task 7: 全量验证 + 验收清单（无提交）

**Files:** 无新改动（如发现问题，回到对应任务修复后重跑本任务）

- [ ] **Step 1: Rust 全量测试**

```powershell
$env:PATH = "C:\Users\xiong\AppData\Local\Microsoft\WinGet\Packages\MartinStorsjo.LLVM-MinGW.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\llvm-mingw-20260616-ucrt-x86_64\bin;" + $env:PATH
cd src-tauri
cargo test
```

Expected: `test result: ok. 8 passed; 0 failed`（signing 3 + clamp 5）

- [ ] **Step 2: 前端构建**

```powershell
cd ..
npm run build
```

Expected: svelte-check 0 errors、vite build 成功

- [ ] **Step 3: 打包验证**

```powershell
npx tauri build --no-bundle
```

Expected: 编译成功产出 exe（不打包安装器）

- [ ] **Step 4: 手动验收（spec §7 九条；`npx tauri dev` 或运行 Step 3 的 exe）：**

1. 多 provider 时 chips 行出现；点 chip 后圆环、热力图、胶囊三处同步显示该来源
2. 「全部」chip 恢复聚合 min 圆环与内层热力图 tabs
3. 重启应用：focus 记忆保留；聚焦的 provider 被禁用后回退 "all"
4. 聚焦无热力图数据的来源（如 MiniMax）：热力图区显示「该来源暂无热力图数据」
5. compact 为 150×44 单行胶囊；点击展开、拖动移动、hover 出 ✕ 三个动作互不误触
6. 贴右缘收起后再展开 dashboard：窗口完整位于工作区内（边缘 ≥8px）
7. 手动把窗口拖出屏幕后重启：首次聚焦时窗口被钳回可视区
8. 胶囊贴边停留 2.5s 后淡出（透明度 0.22、缩放 0.9）；指针进入立即恢复；dashboard 模式从不淡出
9. 以上全部通过且 Step 1-3 全绿

（无人值守执行时：本步标记为「需人工复核」，Step 1-3 照常执行并汇报结果。）

---

## 自审记录

- **Spec 覆盖**：§3.1 全局焦点（chips/持久化/校验回退） → Task 4/5；§3.2 胶囊 150×44/手势/✕ → Task 2 + Task 6；§3.3 工作区钳制（纯函数/应用/切换时机/启动时机） → Task 1/2/3；§3.4 淡出状态机 → Task 6；§7 九条验收 → Task 7
- **占位符扫描**：无 TBD/TODO/「略」/「类似 Task N」；所有代码步骤均为完整可粘贴代码
- **类型一致性**：`clamp_rect(… f64 …) -> (f64, f64)`；`clamp_window_to_work_area(&WebviewWindow, f64) -> bool`；`PhysicalPosition::new(i32, i32)`（`round() as i32`）；`focus: string`；`heatmapTabId: string | null`；`ringPercent: number`（0..1）、`ringLabel: string`；`focusedSnapshot: UsageSnapshot | null`；`pillNearEdge(): Promise<boolean>`；`refreshPillFade(): Promise<void>`；字段名统一 `provider_id` / `provider_display_name`
