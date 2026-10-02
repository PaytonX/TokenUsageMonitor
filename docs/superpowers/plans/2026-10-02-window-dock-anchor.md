# 窗口贴边锚点化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「贴边」从每次从窗口位置反推的临时判断，改成显式持久化的锚点；支持左/右/上/下四条边；修掉贴边后展开/收起丢失贴边、贴左边把手不可见两个既有缺陷。

**Architecture:** 新建 `src-tauri/src/dock.rs`，只放纯几何函数（无 IPC、无窗口句柄），全部可单测。核心类型 `DockAnchor { side, along }`——「贴在哪条边 + 沿边位置」，坐标**相对 work_area 原点**（故与显示器无关）。位置一律用 `anchor_rect(anchor, w, h, area)` 从边**反算**，而不是在旧位置上 `set_size` 再 `clamp`。这把「尺寸变化导致出屏」从根上消掉，也顺带消灭了左右/上下必须各写一套的镜像风险。

**Tech Stack:** Rust 2021 / Tauri 2 / serde+toml / 原生 JS（无框架的 `peek/main.ts`）/ Svelte 5（`App.svelte`）

---

## ⚠️ 执行前必读：Rust 测试必须在 `src-tauri/` 下跑

```powershell
cd src-tauri; cargo test          # ✅ 正确
cargo test --manifest-path src-tauri/Cargo.toml dock   # ❌ 链接失败
```

`src-tauri/.cargo/config.toml` 配置了 `linker = rust-lld`（LLVM-MinGW），
**`.cargo` 配置只在 CWD 位于 `src-tauri/` 或其子目录时生效**。用
`--manifest-path` 指定清单会绕过它，于是 `linker` 回落成 MSVC 的 `link.exe`，
在依赖的 build script 阶段就报 `ld: cannot find crrt2.o / -lkernel32`。
这看起来像代码坏了，实际是工作目录不对。

**所有涉及 cargo 的步骤都用 `cd src-tauri; cargo …`。**

---

## 另一个执行纪律：搬运类 Task 必须读源文件原文

本计划 Task 1 的 `dock.rs` 全文是我基于**被截断的 grep 片段**起草的，
结果漏了 2 个测试、把另 1 个改弱了（丢了左边缘断言）、漏了 3 行函数内注释。
实施 agent 读原文时发现并停下来报告，没有猜着改——这是正确行为。

**规则**：凡是"把 X 从 A 搬到 B"、"把 Y 改成 Z"、"在 W 加一个命令"这类
**照抄/照改**性质的 Task，实施 agent 必须先读目标文件的原文逐字比对，
发现与计划描述不符时**停下来报告**，不得自行猜测补全。
只有"设计新函数"才以计划里的代码为准。

（该 agent 还用 `Compare-Object` 对搬迁的函数体做了逐行比对，
证明除 `pub` 关键字与 3 行 ⚠️ 注释外逐字一致。这个自查值得后续沿用。）

---

## 现状与根因

以下三条是**读代码读出来的**，每条都给了可复现的推演，不是猜测。

### 缺陷 1：贴边不是状态，是每次从位置反推的临时判断

证据链（全部在 `src-tauri/src/ipc.rs`）：

1. `set_window_mode()` 在 `"dashboard"` 分支只调 `set_size(400, 680)`，**没有 `set_position`**（`ipc.rs:611`）。Win32 改尺寸时保持左上原点。
2. 胶囊贴右时 `x = area_x + area_w - 168`。展开成 400 宽后窗口变成 `[x, x+400]`，**右侧溢出 232px 出屏**。
3. 紧接着 `clamp_window_to_work_area(&window, 8.0)`（`ipc.rs:619`）把 x 拉到 `area_x + area_w - 400 - 8`。
4. 收回胶囊时 `set_size(168, 56)` **同样不动原点**（`ipc.rs:624`），于是胶囊停在 `area_x + area_w - 400 - 8`，**距右边缘 240px**——既不贴边，`dock_side()` 也不再判定为 docked。

贴左时 x=0，展开后仍在 0，收回仍在 0 —— **只有右侧会坏**，与用户描述完全一致。

用户的预判也对：如果只把「锚定」改到右侧，贴左就会镜像地坏。根因不是锚在哪一侧，而是**贴边这个状态从未被记下来**。

附带问题：`dock_side()`（`ipc.rs:420`）用「窗口中心 vs 工作区中心」判断左右，且「正中归右」是个任意规则。同时 `sync_peek_window()` 里 `peek_placement(&dash)`（`ipc.rs:647`）的调用发生在 `dock_window(&dash)`（`ipc.rs:672`）**之前**——把手拿到的边是从**贴边前**的位置推断的，与最终贴定的边可能不一致。

### 缺陷 2：贴左时把手不可见 —— Rust 的几何与 CSS 的绘制端不一致

1. 把手窗建的是 `inner_size(7, 58)`（`ipc.rs:686`），但 **Windows 强制最小窗口宽度**（`SM_CXMINTRACK` ≈ 112~136），实际窗宽约 136。
2. `peek_rect()` 对右侧：`x = area_x + area_w - 7`（`ipc.rs:448`）→ 窗占 `[1913, 2049]`，可见 7px 在**窗口左端**。
3. 对左侧：`x = area_x - (136 - 7) = -129`（`ipc.rs:450`）→ 窗占 `[-129, 7]`，可见 7px 在**窗口右端**。
4. 但 `src/peek/main.ts:28` 的 CSS 是 `.peek { width: 7px }` —— 块级元素在 body 里**永远贴窗口左端**，与 `data-side` 无关。

于是贴左时画出来的 7px 落在窗口局部 `[0, 7]` = 屏幕 `[-129, -122]`，**完全在屏幕外**。贴右时碰巧对上，所以只有左边坏。

### 缺陷 3：只支持左右

`dock_side` 只比 x；`dock_rect` 只 snap x；`peek_rect` 只贴 x 且 y 对齐胶囊行；前端 `settlePillAfterDrag()` 的 `nearEdge` 也只算 `min(距左, 距右)`（`App.svelte:762-769`）。上下贴边需要：沿边轴从 y 换成 x、把手窗从 7×58 换成 58×7、把手视觉加横向变体。

### 附带发现：`dashboard_x / dashboard_y` 是死字段

`Settings` 里有 `dashboard_x: Option<i32>` / `dashboard_y`（`settings.rs:39-40`），写进了默认值、参与 `settings_delta.rs:119` 的变更比较、前端 `types.ts:267` 也镜像了——但**全仓库没有任何一处读它们来定位窗口**。旧 spec（`docs/superpowers/specs/2026-09-15-focus-provider-and-pill-design.md:133`）注明这两字段是等「window-state 插件 POSITION 接管」后闲置的，那一步始终没做。

这解释了用户看到的「定位在左上角」：窗口位置既没持久化也没主动设置，每次启动由系统决定。Task 6 直接复用这两个字段，不新增。

---

## 设计

### 核心类型

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockSide { Left, Right, Top, Bottom }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockAnchor {
    pub side: DockSide,
    /// 沿边方向的偏移（物理像素，**相对 work_area 左上角**）：
    /// Left/Right → 窗口上缘的 y；Top/Bottom → 窗口左缘的 x
    pub along: i32,
}
```

`along` 用**相对 work_area** 而非绝对物理坐标：拔掉/换掉显示器后，同一个 `along` 在新显示器上仍然有意义，不会落到不存在的区域。

### 关键函数：`anchor_rect`

这是修掉缺陷 1 的正身。**给定锚点和任意尺寸，直接从边算出位置**：

```rust
pub fn anchor_rect(a: DockAnchor, w: f64, h: f64,
                   ax: f64, ay: f64, aw: f64, ah: f64) -> (f64, f64) {
    let along = a.along as f64;
    let max_y = (ay + ah - h).max(ay);
    let max_x = (ax + aw - w).max(ax);
    match a.side {
        DockSide::Left   => (ax,           along.clamp(ay, max_y)),
        DockSide::Right  => ((ax+aw-w).max(ax), along.clamp(ay, max_y)),
        DockSide::Top    => (along.clamp(ax, max_x), ay),
        DockSide::Bottom => (along.clamp(ax, max_x), (ay+ah-h).max(ay)),
    }
}
```

为什么它同时消灭左右/上下两套代码：贴边轴的位置**只由边和尺寸决定**，与旧位置无关。展开到 400 宽时 `ax+aw-w` 自动变小，窗口永远不出屏。左右走 `match` 的前两个分支，上下走后两个，结构对称，**不存在「锚在哪一侧」的分叉**，也就不存在镜像 bug。

### 边判定：`nearest_side`

替换 `dock_side()` 的中心比较法：

```rust
pub const CORNER_TIE_EPS: f64 = 2.0; // 物理像素

pub fn nearest_side(
    x: f64, y: f64, w: f64, h: f64,
    ax: f64, ay: f64, aw: f64, ah: f64,
    threshold: f64,
    drag: Option<(f64, f64)>,   // 拖拽位移 (dx, dy)，用于角落歧义裁决
) -> Option<DockSide> {
    let cands = [
        (DockSide::Left,   x - ax),
        (DockSide::Right,  (ax + aw) - (x + w)),
        (DockSide::Top,    y - ay),
        (DockSide::Bottom, (ay + ah) - (y + h)),
    ];
    let min = cands.iter().map(|(_, d)| *d)
        .filter(|d| *d <= threshold)
        .fold(f64::INFINITY, f64::min);
    if !min.is_finite() { return None; }
    let tied: Vec<DockSide> = cands.iter()
        .filter(|(_, d)| *d <= threshold && (*d - min).abs() <= CORNER_TIE_EPS)
        .map(|(s, _)| *s).collect();
    if tied.len() > 1 {
        if let Some((dx, dy)) = drag {
            let want_h = dx.abs() >= dy.abs();
            if let Some(p) = tied.iter().copied().find(|s| s.is_horizontal_edge() == want_h) {
                return Some(p);
            }
        }
    }
    tied.first().copied()
}
```

距离可以是负数（窗口已部分出屏）——负值最小，自然胜出，正是「已经贴住了」的语义。角落歧义（胶囊在角上，左右距离都是 0）按**拖拽主方向**裁决，比固定优先级更符合直觉；无拖拽信息时按数组顺序（Left→Right→Top→Bottom）确定性回退。

### 把手几何：`peek_rect`

统一约定：**可见条贴在屏幕边缘，多出来的部分推到屏外**。

```rust
pub fn peek_rect(
    side: DockSide, along: f64, pill_span: f64,
    thickness: f64, length: f64,      // 可见厚度 / 可见长度（物理px）
    actual_w: f64, actual_h: f64,     // 被系统最小尺寸撑大后的真实尺寸
    ax: f64, ay: f64, aw: f64, ah: f64,
) -> (f64, f64) {
    match side {
        DockSide::Left => {
            let x = ax + thickness - actual_w.max(thickness);
            let y = (ay + along + (pill_span - length) / 2.0
                     - (actual_h - length).max(0.0) / 2.0)
                    .clamp(ay, (ay + ah - actual_h).max(ay));
            (x, y)
        }
        DockSide::Right => {
            let x = ax + aw - thickness;
            let y = (ay + along + (pill_span - length) / 2.0
                     - (actual_h - length).max(0.0) / 2.0)
                    .clamp(ay, (ay + ah - actual_h).max(ay));
            (x, y)
        }
        DockSide::Top => {
            let y = ay + thickness - actual_h.max(thickness);
            let x = (ax + along + (pill_span - length) / 2.0
                     - (actual_w - length).max(0.0) / 2.0)
                    .clamp(ax, (ax + aw - actual_w).max(ax));
            (x, y)
        }
        DockSide::Bottom => {
            let y = ay + ah - thickness;
            let x = (ax + along + (pill_span - length) / 2.0
                     - (actual_w - length).max(0.0) / 2.0)
                    .clamp(ax, (ax + aw - actual_w).max(ax));
            (x, y)
        }
    }
}
```

**绘制端约定**（Task 4 改 CSS 时严格照此）——把手窗的可见条永远在窗口的哪一端：

| 贴边 | 窗口位置 | 可见条在窗口的 | `.peek` 内容对齐 |
|---|---|---|---|
| Left | 窗口右缘贴 `ax+thickness` | **右端** | `justify-content: flex-end` |
| Right | 窗口左缘贴 `ax+aw-thickness` | **左端** | `justify-content: flex-start` |
| Top | 窗口下缘贴 `ay+thickness` | **下端** | `align-items: flex-end` |
| Bottom | 窗口上缘贴 `ay+ah-thickness` | **上端** | `align-items: flex-start` |

### 修缺陷 2 的正解：把手「就是」窗口

不要让 CSS 去镜像绘制端（脆弱、容易和 Rust 再次对不上），而是让 `.peek` **填满整个窗口**（`width:100%; height:100%`）。屏外那部分不可见，屏内那 `thickness` px 自然就是完整的把手条。这样：

- 绘制端与几何解耦，**只有一个真值**（窗口位置）
- 系统把窗口撑到多大，视觉就多大，不会出现「CSS 写死 7px 但窗口 136px」的错位
- 握柄位置用 flex 对齐到可见端，而不是在窗口里居中（居中会落到屏外）

### 上下边把手的实际长度

`inner_size(58, 7)` 会被 Windows 最小宽度（≈136）撑成 `136 × 39`：

- **宽度**：58 → 136，多出的 78px 沿边**留在屏内**，所以上下边的把手实际就是 ~136px 长。这不是 bug，是必然；胶囊本身 168px 宽，136 的把手比例合理。**CSS 里不要按 58px 居中。**
- **高度**：7 → 39，多出的 32px 按 `peek_rect` 推到屏幕上方外侧。

左右边不受影响：58 > 最小高度 39，垂直方向不撑。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `src-tauri/src/dock.rs` | 全部贴边纯几何：`DockSide`/`DockAnchor`/`anchor_rect`/`anchor_from_rect`/`nearest_side`/`peek_rect`。无 IPC、无窗口句柄、无状态 | **新建** |
| `src-tauri/src/lib.rs` | 注册 `mod dock` | 修改（1 行） |
| `src-tauri/src/ipc.rs` | 删除 `dock_side`/`dock_rect`/`peek_rect`/`peek_placement`/`dock_window`（搬去 dock.rs）；`set_window_mode`/`sync_peek_window` 改用锚点 | 修改 |
| `src-tauri/src/settings.rs` | `Settings` 增加 `dock: Option<DockAnchor>`；接通 `dashboard_x/y` | 修改 |
| `src/peek/main.ts` | 把手按四边切换宽高、对齐、握柄朝向、动画轴 | 修改 |
| `src/App.svelte` | 落点判定改四边；胶囊滑出方向支持上下；上报锚点 | 修改 |
| `src/lib/api.ts` + `src/lib/types.ts` | `syncPeekWindow` 返回值扩为四边；新增 `set_dock_anchor` 命令封装 | 修改 |

---

## Task 1: 抽出 `dock.rs`（纯重构，行为不变）

先把现有左右两函数原样搬进新模块并让测试跟着走，**不碰任何行为**。目的是把后面 8 个任务的地基铺平，而不是一次性大改。

**Files:**
- Create: `src-tauri/src/dock.rs`
- Modify: `src-tauri/src/lib.rs`（`mod dock;`）
- Modify: `src-tauri/src/ipc.rs`（删掉已搬走的函数与其测试）

- [ ] **Step 1: 建 `dock.rs`，原样搬入现有实现**

在 `src-tauri/src/dock.rs` 写入：

```rust
//! 窗口贴边几何。纯函数、无 IPC、无窗口句柄——所以能直接单测。
//!
//! 本模块的坐标一律是**物理像素**，且除 `DockAnchor::along` 外都相对
//! `work_area` 原点。混用逻辑像素会让高 DPI 下贴边差几个像素。

use serde::{Deserialize, Serialize};

/// 把手窗与胶囊主行的逻辑尺寸（DIP）。主行固定取收起态的 56，这样明细
/// 展开时把手不会跟着下移，收起后也无需重新对齐。
pub const PEEK_THICK: f64 = 7.0;
pub const PEEK_LEN: f64 = 58.0;
pub const PILL_ROW_H: f64 = 56.0;

/// 胶囊应贴靠的边。原先只有水平两向（`dock_side` 返回 bool），
/// 2026-10 扩为四边。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl DockSide {
    /// 贴边发生在 X 轴上（左右两边）。
    pub fn is_horizontal_edge(self) -> bool {
        matches!(self, DockSide::Left | DockSide::Right)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DockSide::Left => "left",
            DockSide::Right => "right",
            DockSide::Top => "top",
            DockSide::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(DockSide::Left),
            "right" => Some(DockSide::Right),
            "top" => Some(DockSide::Top),
            "bottom" => Some(DockSide::Bottom),
            _ => None,
        }
    }

    /// 把手窗的 (宽, 高) 逻辑尺寸。左右边是竖条，上下边是横条。
    pub fn peek_size(self) -> (f64, f64) {
        if self.is_horizontal_edge() {
            (PEEK_THICK, PEEK_LEN)
        } else {
            (PEEK_LEN, PEEK_THICK)
        }
    }
}

/// 贴边锚点：贴在哪条边 + 沿边位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockAnchor {
    pub side: DockSide,
    /// 沿边方向的偏移（物理像素，**相对 work_area 左上角**）：
    /// Left/Right → 窗口上缘的 y；Top/Bottom → 窗口左缘的 x。
    ///
    /// 用相对坐标而非绝对物理坐标：换显示器后同一个 `along` 依然有意义，
    /// 不会落到已拔掉的屏幕区域里。
    pub along: i32,
}

/// 胶囊应贴靠的水平边。比较窗口中心与工作区中心；正中时归右侧（默认边）。
///
/// ⚠️ 仅供 Task 1 的行为不变迁移使用，Task 2 起由 `nearest_side` 取代。
pub fn dock_side(x: f64, w: f64, area_x: f64, area_w: f64) -> bool {
    x + w / 2.0 >= area_x + area_w / 2.0
}

/// 把窗口横向贴死到最近的水平边缘，纵向保留原位置但夹在工作区内。
///
/// ⚠️ 仅供 Task 1 的行为不变迁移使用，Task 2 起由 `anchor_rect` 取代。
pub fn dock_rect(
    x: f64, y: f64, w: f64, h: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
) -> (f64, f64) {
    let left = area_x;
    let right = (area_x + area_w - w).max(area_x);
    let nx = if dock_side(x, w, area_x, area_w) { right } else { left };
    let max_y = (area_y + area_h - h).max(area_y);
    (nx, y.clamp(area_y, max_y))
}

/// 把手的物理位置：贴死所在边缘，纵向中心对齐胶囊主行。
///
/// ⚠️ 仅供 Task 1 的行为不变迁移使用，Task 3 起由四边版 `peek_rect` 取代。
pub fn peek_rect(
    dash_y: f64, row_h: f64, is_right: bool,
    visible_w: f64, actual_w: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
    peek_h: f64,
) -> (f64, f64) {
    let x = if is_right {
        area_x + area_w - visible_w
    } else {
        area_x - (actual_w - visible_w).max(0.0)
    };
    let y = dash_y + row_h / 2.0 - peek_h / 2.0;
    let max_y = (area_y + area_h - peek_h).max(area_y);
    (x, y.clamp(area_y, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA_X: f64 = 0.0;
    const AREA_Y: f64 = 0.0;
    const AREA_W: f64 = 1920.0;
    const AREA_H: f64 = 1040.0;

    #[test]
    fn dock_rect_flushes_to_the_left_edge() {
        let (x, y) = dock_rect(10.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (0.0, 300.0));
    }

    #[test]
    fn dock_rect_flushes_to_the_right_edge() {
        let (x, y) = dock_rect(1000.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (1752.0, 300.0));
    }

    #[test]
    fn dock_rect_exact_middle_docks_right() {
        let (x, _) = dock_rect(876.0, 0.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(x, 1752.0);
    }

    #[test]
    fn dock_rect_keeps_vertical_but_clamps_inside() {
        let (_, top) = dock_rect(0.0, -40.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(top, 0.0);
        let (_, bottom) = dock_rect(0.0, 1030.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(bottom, 984.0);
    }

    #[test]
    fn dock_side_mirrors_for_a_negative_origin_monitor() {
        assert!(!dock_side(-1900.0, 168.0, -1920.0, 1920.0));
        assert!(dock_side(-60.0, 168.0, -1920.0, 1920.0));
    }

    #[test]
    fn peek_rect_sits_flush_and_centres_on_the_pill_row() {
        let args = (100.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 58.0);
        let (x, y) = peek_rect(args.0, args.1, true, 7.0, 7.0, args.2, args.3, args.4, args.5, args.6);
        assert_eq!((x, y), (1913.0, 99.0));
        let (lx, _) = peek_rect(args.0, args.1, false, 7.0, 7.0, args.2, args.3, args.4, args.5, args.6);
        assert_eq!(lx, 0.0);
    }

    #[test]
    fn peek_rect_pushes_the_extra_width_off_screen() {
        // 系统最小窗口宽度把把手窗撑到 136px：可见的 7px 仍贴住屏幕边，
        // 多出的 129px 必须落在屏幕外，否则会吞掉桌面上的鼠标事件。
        let (lx, _) = peek_rect(100.0, 56.0, false, 7.0, 136.0, AREA_X, AREA_Y, AREA_W, AREA_H, 58.0);
        assert_eq!(lx, -129.0);
        let (rx, _) = peek_rect(100.0, 56.0, true, 7.0, 136.0, AREA_X, AREA_Y, AREA_W, AREA_H, 58.0);
        assert_eq!(rx, 1913.0);
    }

    #[test]
    fn peek_rect_clamps_to_the_work_area() {
        let (_, top) = peek_rect(0.0, 56.0, true, 7.0, 7.0, AREA_X, AREA_Y, AREA_W, AREA_H, 58.0);
        assert_eq!(top, 0.0);
        let (_, bottom) = peek_rect(1020.0, 56.0, true, 7.0, 7.0, AREA_X, AREA_Y, AREA_W, AREA_H, 58.0);
        assert_eq!(bottom, 982.0);
    }
}
```

> ⚠️ 这 8 个测试是**从 `ipc.rs` 的 `mod dock_tests` 原样搬来的**，不是新写的。
> 写这份方案时我基于被截断的 grep 片段起草，漏掉了后两个 `peek_rect` 测试
> 并把第一个改弱了（丢了左边缘断言）。实施 agent 读原文时发现并报告了——
> **搬运类 Task 必须读源文件原文，不能照抄计划里的转述**。

- [ ] **Step 2: 在 `lib.rs` 注册模块**

在模块声明区加一行（紧邻 `mod ipc;`）：

```rust
mod dock;
```

- [ ] **Step 3: `ipc.rs` 改为从 `dock` 引用，删掉本地副本与旧测试**

删掉 `ipc.rs:413-455` 的 `PEEK_W`/`PEEK_H`/`PILL_ROW_H`/`dock_side`/`dock_rect`/`peek_rect` 定义，以及 `ipc.rs:1624` 起的整个 `mod dock_tests`。**只删定义，函数体先不动**，所以 import 只能引入 Task 1 已经存在的符号：

```rust
use crate::dock::{dock_rect, dock_side, peek_rect, PEEK_LEN, PEEK_THICK, PILL_ROW_H};
```

（`DockAnchor` / `DockSide` / `anchor_*` / `nearest_side` 要到 Task 2、4 才出现，现在引入会编译不过。）

并把 `ipc.rs` 里所有 `PEEK_W` 改成 `PEEK_THICK`、`PEEK_H` 改成 `PEEK_LEN`（共 3 处引用：`inner_size`、`peek_placement` 的 `PEEK_W * scale` 与 `PEEK_H * scale`）。

- [ ] **Step 4: 跑测试确认行为未变**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dock`
Expected: PASS，**8 个**测试，全部来自 `ipc.rs` 原有的 `mod dock_tests`（名字会带 `dock::tests::` 前缀）。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/dock.rs src-tauri/src/ipc.rs src-tauri/src/lib.rs
git commit -m "refactor(dock): 抽出 dock.rs，贴边几何与 IPC 解耦

纯搬运，行为不变。目的是让后续四边改造只动一个无副作用的模块。
原有三个纯函数与 6 个测试一并搬入。"
```

---

## Task 2: 四边锚点模型（`anchor_rect` / `anchor_from_rect` / `nearest_side`）

**Files:**
- Modify: `src-tauri/src/dock.rs`
- Test: `src-tauri/src/dock.rs`（内联 `mod tests`）

- [ ] **Step 1: 写失败测试**

追加到 `dock.rs` 的 `mod tests`：

```rust
    // ---- Task 2: 四边锚点 -------------------------------------------------

    fn anchor(side: DockSide, along: i32) -> DockAnchor {
        DockAnchor { side, along }
    }

    #[test]
    fn anchor_rect_left_is_flush_and_keeps_the_along_axis() {
        let (x, y) = anchor_rect(anchor(DockSide::Left, 300), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (0.0, 300.0));
    }

    #[test]
    fn anchor_rect_right_is_flush() {
        let (x, y) = anchor_rect(anchor(DockSide::Right, 300), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (1752.0, 300.0));
    }

    #[test]
    fn anchor_rect_top_is_flush_and_along_axis_is_x() {
        let (x, y) = anchor_rect(anchor(DockSide::Top, 600), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (600.0, 0.0));
    }

    #[test]
    fn anchor_rect_bottom_is_flush() {
        let (x, y) = anchor_rect(anchor(DockSide::Bottom, 600), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!((x, y), (600.0, 984.0));
    }

    /// 回归缺陷 1：贴右的胶囊展开成 400 宽后**必须重新贴右**。
    /// 旧实现只 set_size 不 set_position，窗口会撑出屏幕右侧 232px，
    /// 随后的 clamp 把它拉到「留 8px 边距」的位置，收回胶囊后就离右边 240px。
    #[test]
    fn anchor_rect_survives_a_width_change_without_leaving_the_edge() {
        let a = anchor(DockSide::Right, 300);
        let (x_small, y_small) = anchor_rect(a, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        let (x_big, y_big) = anchor_rect(a, 400.0, 680.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(x_small + 168.0, AREA_W); // 胶囊右缘贴屏幕右边
        assert_eq!(x_big + 400.0, AREA_W);   // 展开后右缘**仍**贴屏幕右边
        assert_eq!(y_small, y_big);          // 纵向不因尺寸变化而漂移
        assert!(x_big >= 0.0 && x_big + 400.0 <= AREA_W); // 且完全在屏内
    }

    #[test]
    fn anchor_rect_survives_a_height_change_for_top_and_bottom() {
        let top = anchor(DockSide::Top, 600);
        let (x1, y1) = anchor_rect(top, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        let (x2, y2) = anchor_rect(top, 168.0, 680.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(y1, 0.0);
        assert_eq!(y2, 0.0);
        assert_eq!(x1, x2);

        let bot = anchor(DockSide::Bottom, 600);
        let (_, yb1) = anchor_rect(bot, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        let (_, yb2) = anchor_rect(bot, 168.0, 680.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(yb1 + 56.0, AREA_H);
        assert_eq!(yb2 + 680.0, AREA_H);
    }

    #[test]
    fn anchor_rect_clamps_a_stale_along_value_back_on_screen() {
        // 换了个分辨率更小的显示器后，沿边坐标可能超出新工作区。
        let (x, y) = anchor_rect(anchor(DockSide::Left, 5000), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(y, AREA_H - 56.0);
        let (x, y) = anchor_rect(anchor(DockSide::Top, 5000), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
        assert_eq!(x, AREA_W - 168.0);
    }

    #[test]
    fn anchor_from_rect_round_trips_through_anchor_rect() {
        for side in [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom] {
            let (x, y) = anchor_rect(anchor(side, 321), 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
            let a = anchor_from_rect(x, y, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, side);
            assert_eq!(a.along, 321, "side={side:?}");
            let (x2, y2) = anchor_rect(a, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
            assert_eq!((x, y), (x2, y2), "side={side:?}");
        }
    }

    #[test]
    fn anchor_from_rect_stores_along_relative_to_the_work_area() {
        // work_area 原点不在 (0,0)（副屏在左侧）时，along 必须是相对值。
        let a = anchor_from_rect(-1900.0, 300.0, 168.0, 56.0, -1920.0, 0.0, 1920.0, 1040.0, DockSide::Left);
        assert_eq!(a, DockAnchor { side: DockSide::Left, along: 300 });
    }

    #[test]
    fn nearest_side_picks_the_closest_edge() {
        assert_eq!(nearest_side(0.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Left));
        assert_eq!(nearest_side(1752.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Right));
        assert_eq!(nearest_side(600.0, 0.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Top));
        assert_eq!(nearest_side(600.0, 984.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Bottom));
    }

    #[test]
    fn nearest_side_returns_none_when_far_from_every_edge() {
        assert_eq!(nearest_side(800.0, 500.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), None);
    }

    #[test]
    fn nearest_side_breaks_corner_ties_by_drag_direction() {
        // 胶囊停在左上角：left 与 top 的距离都是 0。
        let (x, y, w, h) = (0.0, 0.0, 168.0, 56.0);
        // 往右拖 → 用户想贴左边
        assert_eq!(nearest_side(x, y, w, h, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, Some((200.0, 3.0))), Some(DockSide::Left));
        // 往下拖 → 用户想贴上边
        assert_eq!(nearest_side(x, y, w, h, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, Some((3.0, 200.0))), Some(DockSide::Top));
        // 无拖拽信息 → 按数组顺序确定性回退到 Left
        assert_eq!(nearest_side(x, y, w, h, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Left));
    }

    #[test]
    fn nearest_side_treats_a_partially_offscreen_window_as_docked() {
        // 缺陷 1 的场景：展开后窗口右侧溢出屏幕，left 距离为负。
        // 负值最小，必须仍然判成 Right 而不是 None。
        assert_eq!(nearest_side(1752.0, 300.0, 400.0, 680.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Right));
    }

    #[test]
    fn nearest_side_is_exact_at_the_threshold() {
        assert_eq!(nearest_side(40.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Left));
        assert_eq!(nearest_side(41.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), None);
    }

    #[test]
    fn dock_side_as_str_and_parse_round_trip() {
        for s in [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom] {
            assert_eq!(DockSide::parse(s.as_str()), Some(s));
        }
        assert_eq!(DockSide::parse("diagonal"), None);
    }

    #[test]
    fn peek_size_swaps_the_axes_for_top_and_bottom() {
        assert_eq!(DockSide::Left.peek_size(), (7.0, 58.0));
        assert_eq!(DockSide::Right.peek_size(), (7.0, 58.0));
        assert_eq!(DockSide::Top.peek_size(), (58.0, 7.0));
        assert_eq!(DockSide::Bottom.peek_size(), (58.0, 7.0));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dock`
Expected: FAIL —— 编译错误 `cannot find function anchor_rect in this scope`（以及 `anchor_from_rect` / `nearest_side` / `DockAnchor` 未定义）。

- [ ] **Step 3: 实现**

在 `dock.rs` 中 `dock_side` 定义之前插入（放在 `DockAnchor` 之后、旧的 `dock_side` 之前）：

```rust
/// 角落歧义阈值：两条边的贴合距离相差在此以内即视为平局（物理像素）。
pub const CORNER_TIE_EPS: f64 = 2.0;

/// 从锚点算出**任意尺寸**下的窗口位置。
///
/// 这是修掉「贴边后展开就丢贴边」的正身：位置一律**从边反算**，
/// 贴边轴的坐标只由「哪条边 + 当前尺寸」决定，与旧位置无关。
/// 于是把窗口从 168×56 撑到 400×680 时，右缘仍然贴屏幕右缘、且完全不出屏——
/// 旧实现只 `set_size` 不 `set_position`，窗口会右侧溢出 232px 出屏。
///
/// 四个分支结构对称，所以左右与上下不存在「镜像 bug」这回事。
pub fn anchor_rect(
    a: DockAnchor,
    w: f64,
    h: f64,
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
) -> (f64, f64) {
    let along = f64::from(a.along);
    // 沿边轴要夹回工作区，否则换了个小屏后窗口会整块跑到屏幕外。
    let max_y = (area_y + area_h - h).max(area_y);
    let max_x = (area_x + area_w - w).max(area_x);
    match a.side {
        DockSide::Left => (area_x, along.clamp(area_y, max_y)),
        DockSide::Right => ((area_x + area_w - w).max(area_x), along.clamp(area_y, max_y)),
        DockSide::Top => (along.clamp(area_x, max_x), area_y),
        DockSide::Bottom => (along.clamp(area_x, max_x), (area_y + area_h - h).max(area_y)),
    }
}

/// 从「已落位」的窗口几何反推锚点。落位后回填用，保证 anchor 与实际位置同源。
pub fn anchor_from_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
    side: DockSide,
) -> DockAnchor {
    let along = if side.is_horizontal_edge() { y - area_y } else { x - area_x };
    DockAnchor { side, along: along.round() as i32 }
}

/// 找出离窗口最近的那条边；四条边都超出 `threshold` 时返回 `None`（应保持浮动）。
///
/// 取代旧的 `dock_side`：那个函数只比 x、且用「窗口中心 vs 工作区中心」，
/// 带一条「正中归右」的任意规则。这里的距离是「窗口在该边上的边到屏幕边」，
/// 天然对称。
///
/// 距离**允许为负**（窗口已部分出屏）：负值最小，自然胜出——这正是
/// 「它已经贴住了」的语义，缺陷 1 里展开后溢出屏幕的窗口靠这条判对边。
///
/// `drag` 是拖拽位移 (dx, dy)，只用于角落歧义（胶囊停在角上时左右/上下
/// 距离同时为 0）：此时「往哪个方向拖」才是用户的真实意图。
pub fn nearest_side(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
    threshold: f64,
    drag: Option<(f64, f64)>,
) -> Option<DockSide> {
    let cands = [
        (DockSide::Left, x - area_x),
        (DockSide::Right, (area_x + area_w) - (x + w)),
        (DockSide::Top, y - area_y),
        (DockSide::Bottom, (area_y + area_h) - (y + h)),
    ];
    let min = cands
        .iter()
        .map(|(_, d)| *d)
        .filter(|d| *d <= threshold)
        .fold(f64::INFINITY, f64::min);
    if !min.is_finite() {
        return None;
    }
    let tied: Vec<DockSide> = cands
        .iter()
        .filter(|(_, d)| *d <= threshold && (*d - min).abs() <= CORNER_TIE_EPS)
        .map(|(s, _)| *s)
        .collect();
    if tied.len() > 1 {
        if let Some((dx, dy)) = drag {
            let want_horizontal = dx.abs() >= dy.abs();
            if let Some(p) = tied
                .iter()
                .copied()
                .find(|s| s.is_horizontal_edge() == want_horizontal)
            {
                return Some(p);
            }
        }
    }
    tied.first().copied()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dock`
Expected: PASS，19 个测试。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/dock.rs
git commit -m "feat(dock): 四边锚点模型 anchor_rect/nearest_side

贴边从「每次从位置反推」改为「显式锚点 + 从边反算位置」。
anchor_rect 对任意尺寸都从边算贴边轴，窗口因此不会因放大而溢出屏幕——
这修掉「贴右收起 → 恢复主窗 → 再收起，胶囊不再贴边」。

along 用相对 work_area 的坐标，换显示器后仍可复用。
nearest_side 取代中心比较法：距离可为负（已出屏即已贴住），
角落歧义按拖拽主方向裁决。"
```

---

## Task 3: 把手几何泛化到四边

**Files:**
- Modify: `src-tauri/src/dock.rs`（把旧 `peek_rect` 替换为四边版）
- Test: `src-tauri/src/dock.rs`

- [ ] **Step 1: 写失败测试**

先删掉旧的 `peek_rect` 及其测试 `peek_rect_sits_flush_and_centres_on_the_pill_row`，换成：

```rust
    // ---- Task 3: 四边把手 -------------------------------------------------

    const THICK: f64 = 7.0;   // 可见厚度
    const LEN: f64 = 58.0;    // 可见长度
    const STRETCH_W: f64 = 136.0; // Windows 最小窗口宽度撑大后的真实宽度
    const STRETCH_H: f64 = 39.0;  // 最小窗口高度（58 > 39，竖向不撑）

    fn peek(side: DockSide, along: f64, aw: f64, ah: f64) -> (f64, f64) {
        peek_rect(side, along, 56.0, THICK, LEN, aw, ah, AREA_X, AREA_Y, AREA_W, AREA_H)
    }

    #[test]
    fn peek_right_puts_the_visible_strip_on_the_windows_left_end() {
        // 窗 [1913, 2049]，只有左端 7px 在屏内 —— CSS 须左对齐内容。
        let (x, y) = peek(DockSide::Right, 300.0, STRETCH_W, LEN);
        assert_eq!(x, 1913.0);
        assert_eq!(y, 299.0); // 300 + (56-58)/2
    }

    #[test]
    fn peek_left_puts_the_visible_strip_on_the_windows_right_end() {
        // 回归缺陷 2：窗 [-129, 7]，只有右端 7px 在屏内。
        // 旧 CSS 把 7px 画在窗口左端 → 落到 [-129,-122]，屏外，看不见把手。
        let (x, y) = peek(DockSide::Left, 300.0, STRETCH_W, LEN);
        assert_eq!(x, -129.0);
        assert_eq!(x + STRETCH_W, THICK); // 右缘正好是屏内可见条的右界
        assert_eq!(y, 299.0);
    }

    #[test]
    fn peek_top_pushes_the_stretched_height_off_screen() {
        let (x, y) = peek(DockSide::Top, 600.0, LEN, STRETCH_H);
        assert_eq!(y, THICK - STRETCH_H); // 负值，窗口下缘贴 ay+thickness
        assert_eq!(y + STRETCH_H, THICK);
        assert_eq!(x, 600.0 + (56.0 - LEN) / 2.0); // 沿边轴居中于胶囊
    }

    #[test]
    fn peek_bottom_sits_flush_on_the_work_area_edge() {
        let (x, y) = peek(DockSide::Bottom, 600.0, LEN, STRETCH_H);
        assert_eq!(y, AREA_H - THICK);
        assert_eq!(x, 600.0 + (56.0 - LEN) / 2.0);
    }

    #[test]
    fn peek_never_falls_off_the_screen_along_the_edge() {
        // 贴上边、沿边坐标贴近工作区左缘时，夹回屏内。
        let (x, _) = peek(DockSide::Top, -900.0, LEN, STRETCH_H);
        assert!(x >= 0.0);
        let (_, y) = peek(DockSide::Left, -900.0, STRETCH_W, LEN);
        assert!(y >= 0.0);
    }

    #[test]
    fn peek_visible_strip_always_straddles_the_screen_edge_exactly() {
        for side in [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom] {
            let (aw, ah) = if side.is_horizontal_edge() { (STRETCH_W, LEN) } else { (LEN, STRETCH_H) };
            let (x, y) = peek(side, 400.0, aw, ah);
            let strip = match side {
                DockSide::Left => (x + aw, x + aw + THICK),
                DockSide::Right => (x, x + THICK),
                DockSide::Top => (y + ah, y + ah + THICK),
                DockSide::Bottom => (y, y + THICK),
            };
            let edge = match side {
                DockSide::Left => AREA_X,
                DockSide::Right => AREA_X + AREA_W,
                DockSide::Top => AREA_Y,
                DockSide::Bottom => AREA_Y + AREA_H,
            };
            let expected = match side {
                DockSide::Left | DockSide::Bottom => (edge - THICK, edge),
                DockSide::Right | DockSide::Top => (edge, edge + THICK),
            };
            assert_eq!(strip, expected, "side={side:?}");
        }
    }

    #[test]
    fn peek_works_when_the_window_was_not_stretched_at_all() {
        // 极小 DPI 或将来绕开最小宽度时，actual == 可见尺寸。
        let (x, y) = peek(DockSide::Right, 300.0, THICK, LEN);
        assert_eq!((x, y), (1913.0, 299.0));
        let (x, y) = peek(DockSide::Left, 300.0, THICK, LEN);
        assert_eq!((x, y), (0.0, 299.0));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dock::tests::peek`
Expected: FAIL —— 编译错误 `this function takes 9 arguments but 11 arguments were supplied`。

- [ ] **Step 3: 实现**

用下面这份替换 `dock.rs` 里旧的 `peek_rect`：

```rust
/// 把手窗的物理位置。
///
/// 约定（Task 4 的 CSS 必须严格照此对齐，否则缺陷 2 会以另一种形式复发）：
/// **可见条永远贴在屏幕边缘，被系统最小尺寸撑出来的多余部分推到屏外。**
/// 于是「把手可见条落在窗口的哪一端」是确定的：
///
/// | 贴边   | 可见条在窗口的 | `.peek` 内容对齐              |
/// |--------|----------------|------------------------------|
/// | Left   | 右端           | `justify-content: flex-end`  |
/// | Right  | 左端           | `justify-content: flex-start`|
/// | Top    | 下端           | `align-items: flex-end`      |
/// | Bottom | 上端           | `align-items: flex-start`    |
///
/// 参数：
/// - `along`：胶囊沿边轴的起点（相对 work_area 原点，与 DockAnchor::along 同义）
/// - `pill_span`：胶囊在沿边轴上的尺寸
/// - `thickness` / `length`：把手的可见厚度 / 可见长度（物理像素）
/// - `actual_w` / `actual_h`：把手窗被系统最小尺寸撑大后的真实尺寸
pub fn peek_rect(
    side: DockSide,
    along: f64,
    pill_span: f64,
    thickness: f64,
    length: f64,
    actual_w: f64,
    actual_h: f64,
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
) -> (f64, f64) {
    match side {
        DockSide::Left => {
            let x = area_x + thickness - actual_w.max(thickness);
            let y = (area_y + along + (pill_span - length) / 2.0
                - (actual_h - length).max(0.0) / 2.0)
                .clamp(area_y, (area_y + area_h - actual_h).max(area_y));
            (x, y)
        }
        DockSide::Right => {
            let x = area_x + area_w - thickness;
            let y = (area_y + along + (pill_span - length) / 2.0
                - (actual_h - length).max(0.0) / 2.0)
                .clamp(area_y, (area_y + area_h - actual_h).max(area_y));
            (x, y)
        }
        DockSide::Top => {
            let y = area_y + thickness - actual_h.max(thickness);
            let x = (area_x + along + (pill_span - length) / 2.0
                - (actual_w - length).max(0.0) / 2.0)
                .clamp(area_x, (area_x + area_w - actual_w).max(area_x));
            (x, y)
        }
        DockSide::Bottom => {
            let y = area_y + area_h - thickness;
            let x = (area_x + along + (pill_span - length) / 2.0
                - (actual_w - length).max(0.0) / 2.0)
                .clamp(area_x, (area_x + area_w - actual_w).max(area_x));
            (x, y)
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml dock`
Expected: PASS。`ipc.rs` 此刻会编译失败（调用点还是旧签名），**这是预期的**，Task 4 修。

- [ ] **Step 5: 提交（连同 Task 4 一起，见下）**

本步的改动不单独提交——它会让 `ipc.rs` 编译不过。**直接进入 Task 4，完成后一起提交。**

---

## Task 4: 修缺陷 2 —— 把手绘制端与几何对齐

**Files:**
- Modify: `src/peek/main.ts`
- Modify: `src-tauri/src/ipc.rs`（`peek_placement` 改用四边 `peek_rect`；建窗时按边设尺寸并纠正）
- Test: `src-tauri/src/dock.rs`（Task 3 已写）

- [ ] **Step 1: 改 `peek/main.ts` 的 CSS 与类型**

把文件顶部的类型与常量改为：

```ts
type Side = "left" | "right" | "top" | "bottom";

declare global {
  interface Window {
    /** 由 Rust 建窗时的 initialization_script 注入。 */
    __PEEK_SIDE__?: Side;
  }
}

const HOVER_DEBOUNCE_MS = 60;
const side: Side = window.__PEEK_SIDE__ ?? "right";
const isHorizontalEdge = side === "left" || side === "right";
```

把 `host.innerHTML` 之后注入的 `<style>` 整段替换为：

```ts
  const style = document.createElement("style");
  style.textContent = `
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body { height: 100%; overflow: hidden; background: transparent; }
    .peek {
      /* ⚠️ 必须填满整个窗口，不能写死 7px / 58px。
         Windows 会把窗口撑到最小尺寸（横向 ~136px），写死尺寸会让画出来的条
         落在窗口的固定一端，贴左边时整条被推到屏幕外 —— 表现为「贴左边没有
         小把手」。填满窗口后「把手可见条 = 屏幕边缘那 thickness px」由 Rust 的
         几何唯一决定，CSS 不再需要知道窗口被撑成多大。 */
      width: 100%;
      height: 100%;
      display: flex;
      align-items: center;
      /* 与 --tum-glass 保持一致：透明窗里 backdrop-filter 采不到桌面，
         只用高不透明度深色保证对比度。 */
      background: rgba(22, 25, 31, 0.92);
      transition:
        opacity 160ms cubic-bezier(0.33, 1, 0.68, 1),
        transform 160ms cubic-bezier(0.33, 1, 0.68, 1);
      cursor: default;
    }
    /* 对齐到「屏内可见的那一端」（见 dock.rs::peek_rect 的表）。
       水平边：主轴是水平。竖直边：主轴变成垂直。 */
    .peek[data-side="right"]  { flex-direction: row; justify-content: flex-start; transform-origin: left center; }
    .peek[data-side="left"]   { flex-direction: row; justify-content: flex-end;   transform-origin: right center; }
    .peek[data-side="bottom"] { flex-direction: column; align-items: flex-start; transform-origin: center top; }
    .peek[data-side="top"]    { flex-direction: column; align-items: flex-end;   transform-origin: center bottom; }

    /* 圆角落在靠屏内的那一端，贴边那侧保持直角。 */
    .peek[data-side="right"]  { border-radius: 4px 0 0 4px; border-right: none; }
    .peek[data-side="left"]   { border-radius: 0 4px 4px 0; border-left: none; }
    .peek[data-side="bottom"] { border-radius: 4px 4px 0 0; border-bottom: none; }
    .peek[data-side="top"]    { border-radius: 0 0 4px 4px; border-top: none; }

    .grip { flex: none; background: rgba(255, 255, 255, 0.55); transition: transform 110ms ease; }
    .peek[data-side="right"] .grip, .peek[data-side="left"] .grip {
      width: 2px; height: 22px; border-radius: 2px;
    }
    .peek[data-side="bottom"] .grip, .peek[data-side="top"] .grip {
      width: 22px; height: 2px; border-radius: 2px;
    }
    .peek[data-side="right"]:hover .grip,
    .peek[data-side="left"]:hover .grip { transform: scaleY(1.3); }
    .peek[data-side="bottom"]:hover .grip,
    .peek[data-side="top"]:hover .grip { transform: scaleX(1.3); }

    /* 只做视觉隐藏：绝不在这里改 pointer-events —— 在指针仍位于把手上时翻转它，
       会让浏览器改判 hover 目标并派发一次假的 pointerleave，于是主窗收到
       peek-leave、刚滑入的胶囊 320ms 后就被收回（表现为「弹出即收回」）。
       把手能否接收鼠标由 Rust 的 set_ignore_cursor_events 在窗口级托管。 */
    .peek.is-hidden { opacity: 0; }
    .peek[data-side="right"].is-hidden,
    .peek[data-side="left"].is-hidden { transform: scaleX(0.4); }
    .peek[data-side="bottom"].is-hidden,
    .peek[data-side="top"].is-hidden { transform: scaleY(0.4); }
    @media (prefers-reduced-motion: reduce) {
      .peek, .grip { transition-duration: 0.001ms; }
    }
  `;
```

把 `peek-show` 监听里的合法值判断扩为四边：

```ts
  void listen("peek-show", (event) => {
    const next = event.payload as Side | undefined;
    if (next === "left" || next === "right" || next === "top" || next === "bottom") {
      host.dataset.side = next;
    }
    host.classList.remove("is-hidden");
  });
```

- [ ] **Step 2: 改 `ipc.rs` 的 `peek_placement`**

把现有 `peek_placement()` 整个替换为：

```rust
/// 把手窗的物理位置 + 胶囊贴靠的边。
/// 位置**从锚点算**，不再读 dashboard 的当前位置——锚点是唯一真值，
/// 胶囊在滑出动画里怎么动都不会带着把手抖。
fn peek_placement(
    window: &WebviewWindow,
    side: DockSide,
    along: i32,
) -> Option<PhysicalPosition<i32>> {
    let Ok(Some(monitor)) = window.current_monitor() else { return None };
    let area = monitor.work_area();
    let scale = f64::from(monitor.scale_factor());
    let (vis_w, vis_h) = side.peek_size();
    let thickness = vis_w * scale;
    let length = vis_h * scale;
    // 把手窗可能已被系统最小尺寸撑大：按真实尺寸定位，把垂直于贴边轴的
    // 多余部分推出屏幕（否则那一整条透明区域会持续吞掉桌面上的鼠标事件）。
    let actual = window
        .app_handle()
        .get_webview_window("peek")
        .and_then(|p| p.outer_size().ok())
        .map(|s| (f64::from(s.width), f64::from(s.height)));
    let (aw, ah) = actual.unwrap_or((thickness, length));
    let (aw, ah) = (aw.max(thickness), ah.max(length));
    let (px, py) = peek_rect(
        side,
        f64::from(along),
        PILL_ROW_H * scale,
        thickness,
        length,
        aw,
        ah,
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    );
    Some(PhysicalPosition::new(px.round() as i32, py.round() as i32))
}
```

- [ ] **Step 3: 改 `sync_peek_window` 的建窗与定位**

函数开头（原 `ipc.rs:643-655`）：

```rust
pub async fn sync_peek_window(app: AppHandle, state: String) -> Result<String, String> {
    let dash = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;
    // 边与沿边坐标**先算出来并落位**，再拿去建/放把手。旧实现先按胶囊当前位置
    // 推断边、再 dock，于是把手拿到的边可能与最终贴定的边不一致。
    let Some(anchor) = current_anchor(&dash) else {
        return Err("dashboard window has no monitor".to_string());
    };
    let side = anchor.side;
    let side_str = side.as_str();

    // 只在 compact 胶囊态才有把手；dashboard 态（可能是模式切换竞态）直接
    // 返回所在边，不重建刚被 set_window_mode 关掉的窗口。
    if !app.state::<AppState>().compact_mode.load(Ordering::SeqCst) {
        return Ok(side_str.to_string());
    }

    match state.as_str() {
```

`"revealed" | "docked"` 分支里，把

```rust
            if state == "docked" {
                dock_window(&dash);
            }
```

替换为

```rust
            // docked 才贴死；revealed 时窗口已贴边，不再移动，避免每帧校正抖动。
            if state == "docked" {
                apply_anchor(&dash, anchor);
            }
```

建窗处（原 `ipc.rs:675-702`）——尺寸按边取，`initialization_script` 带上四边字面量：

```rust
            let peek = match app.get_webview_window("peek") {
                Some(existing) => existing,
                None => {
                    // 把手页通过初始化脚本拿到所在边，决定内容对齐与圆角朝向。
                    let (pw, ph) = side.peek_size();
                    let built = WebviewWindowBuilder::new(
                        &app,
                        "peek",
                        tauri::WebviewUrl::App("peek.html".into()),
                    )
                    .title("TokenUsageMonitor · 贴边把手")
                    .inner_size(pw, ph)
                    .resizable(false)
                    .decorations(false)
                    .transparent(true)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .shadow(false)
                    .focused(false)
                    .visible(false)
                    .initialization_script(format!("window.__PEEK_SIDE__ = {:?};", side_str))
                    .build()
                    .map_err(|e| e.to_string())?;
                    #[cfg(windows)]
                    crate::dwm_corner::disable_corner_artifacts(&built);
                    built
                }
            };

            // 刚建出来时还不知道系统实际最小尺寸：定位两次——第一次用默认厚度，
            // 拿到真实 outer_size 后由 peek_placement 重算，把垂直于贴边轴的
            // 多余部分推出屏幕（贴左边/上边时尤其关键）。
            if let Some(p) = peek_placement(&dash, side, anchor.along) {
                let _ = peek.set_position(p);
            }
```

函数末尾（原 `Ok(if is_right { "right" } else { "left" }.to_string())`）改为：

```rust
    Ok(side_str.to_string())
```

- [ ] **Step 3b: 修 `reposition_peek` 的调用点**

`reposition_peek`（原 `ipc.rs:548-555`）也在调 `peek_placement`，签名变了必须一起改。整段替换为：

```rust
/// 让把手跟随 Dashboard 胶囊的纵向位置。把手尚未创建时为空操作。
///
/// 从锚点算而非读胶囊当前位置：胶囊在滑出动画里怎么动都不会带着把手抖。
pub fn reposition_peek(window: &WebviewWindow) {
    let Some(peek) = window.app_handle().get_webview_window("peek") else { return };
    let Some(anchor) = current_anchor(window) else { return };
    let Some(pos) = peek_placement(window, anchor.side, anchor.along) else { return };
    if peek.outer_position().map(|cur| cur == pos).unwrap_or(false) {
        return;
    }
    let _ = peek.set_position(pos);
}
```

- [ ] **Step 4: 补 `current_anchor` / `apply_anchor` 两个助手，并删掉 `dock_window`**

放在 `dock_window` 原本的位置（`ipc.rs` 约 484 行）。`dock_window` **整个删掉**——它的唯一调用点已在 Step 3 换成 `apply_anchor`，留着会变成死代码（`dock_rect` 随之也无人调用，一并在 Task 8 的清理里删）：

```rust
/// 读出胶囊当前**已贴边**时的锚点；离四条边都超过阈值时返回 `None`（浮动）。
///
/// 语义说明（这是一次有意的行为变化，不是缺陷）：判据是「离边够近」，
/// 而不是「前端此刻认为它是 docked」。`set_window_mode` 是 Rust 命令、
/// 拿不到前端的 `pillDocked`，而 `pillDocked` 本身就是拖拽松手那一刻用
/// 同一套阈值判出来的。后果是：一个恰好停在边缘 30px 处的浮动胶囊，
/// 展开成主窗时会被吸附到那条边。我认为这符合直觉（用户把它放在那儿，
/// 多半就是想靠边），但**要写在这里**，以免日后被当成 bug 重新"修"回去。
pub fn current_anchor(window: &WebviewWindow) -> Option<DockAnchor> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitor = window.current_monitor().ok()??;
    let area = monitor.work_area();
    let (ax, ay) = (f64::from(area.position.x), f64::from(area.position.y));
    let (aw, ah) = (f64::from(area.size.width), f64::from(area.size.height));
    let side = nearest_side(
        f64::from(pos.x), f64::from(pos.y),
        f64::from(size.width), f64::from(size.height),
        ax, ay, aw, ah,
        DOCK_SNAP_MARGIN_PX * f64::from(monitor.scale_factor()),
        None,
    )?;
    Some(anchor_from_rect(
        f64::from(pos.x), f64::from(pos.y),
        f64::from(size.width), f64::from(size.height),
        ax, ay, aw, ah, side,
    ))
}

/// 把窗口落到锚点上。`anchor_rect` 保证贴边轴永远贴死且不出屏，
/// 所以窗口尺寸怎么变都不会「掉出贴边态」。
pub fn apply_anchor(window: &WebviewWindow, a: DockAnchor) -> bool {
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
    let _ = window.set_position(PhysicalPosition::new(nx.round() as i32, ny.round() as i32));
    true
}
```

并在文件顶部常量区加：

```rust
/// 胶囊距离屏幕边多少物理像素内算「已贴边」。与前端 PILL_DOCK_THRESHOLD_PX
/// （40 逻辑 px）保持同一量级；这里用逻辑值 × scale_factor 换算。
const DOCK_SNAP_MARGIN_PX: f64 = 40.0;
```

同时把 `ipc.rs` 的 import 补成：

```rust
use crate::dock::{
    anchor_from_rect, anchor_rect, nearest_side, peek_rect, DockAnchor, DockSide,
    PEEK_LEN, PEEK_THICK, PILL_ROW_H,
};
```

（移除已不再使用的 `dock_rect` / `dock_side`。）

- [ ] **Step 5: 跑测试与类型检查**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS，0 failed。

Run: `npx svelte-check --threshold warning`
Expected: 0 errors 0 warnings。

- [ ] **Step 6: 提交（Task 3 + Task 4 一起）**

```bash
git add src-tauri/src/dock.rs src-tauri/src/ipc.rs src/peek/main.ts
git commit -m "fix(peek): 贴左边把手不可见 + 把手几何泛化到四边

缺陷根因：Windows 最小窗口宽度把 7px 的把手窗撑到 ~136px，而 CSS 把
7px 画在窗口**左端**。贴右边时窗口 [1913,2049] 恰好看得见左端，贴左边
时窗口 [-129,7] 的左端全在屏外 —— 于是贴左边没有把手。

改法：`.peek` 填满整个窗口，「可见条 = 屏幕边缘那 7px」由 Rust 几何
唯一决定，CSS 不再需要知道窗口被撑成多大。顺带把内容对齐、圆角、
握柄朝向、隐藏动画轴都按四边分派。

同时把 peek_placement 改为从锚点算（不再读胶囊当前位置），并消除
它早于 dock_window 的时序错位。"
```

---

## Task 5: `set_window_mode` 改用锚点落位

**Files:**
- Modify: `src-tauri/src/ipc.rs:597-635`

- [ ] **Step 1: 改写 `set_window_mode`**

整体替换为：

```rust
/// Switch the dashboard window between full and compact modes.
///
/// 关键：换尺寸**必须同时换位置**，且位置由锚点算。旧实现只 `set_size`
/// 不 `set_position`——Win32 保持左上原点，于是贴右的胶囊展开成 400 宽
/// 后右侧溢出 232px 出屏，随后的 clamp 把它拉到「留 8px 边距」处，
/// 再收回胶囊时它就停在离右边 240px 的地方，不再贴边。
#[tauri::command]
pub async fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    use tauri::LogicalSize;

    let window = app
        .get_webview_window("dashboard")
        .ok_or_else(|| "dashboard window not found".to_string())?;

    let compact = app.state::<AppState>().compact_mode.clone();

    match mode.as_str() {
        "dashboard" => {
            // 先取锚点：必须在改尺寸**之前**读，旧尺寸才是胶囊的贴边尺寸。
            let anchor = current_anchor(&window);
            window
                .set_size(LogicalSize::new(400u32, 680u32))
                .map_err(|e| e.to_string())?;
            // 全窗态是普通窗口：恢复鼠标交互，贴边把手在此模式下没有意义。
            let _ = window.set_ignore_cursor_events(false);
            if let Some(peek) = app.get_webview_window("peek") {
                let _ = peek.close();
            }
            compact.store(false, Ordering::SeqCst);
            match anchor {
                // 贴着边来的：按同一锚点重算 400×680 的位置，仍然贴死且不出屏。
                Some(a) => { apply_anchor(&window, a); }
                // 浮动来的：只把越界的位置拉回屏内，不强行贴边。
                None => { clamp_window_to_work_area(&window, 8.0); }
            }
        }
        "compact" => {
            let anchor = current_anchor(&window);
            // 迷你胶囊 168x56：主行 = 圆环 + 百分比 + 分隔线 + 品牌芯片。
            window
                .set_size(LogicalSize::new(168u32, 56u32))
                .map_err(|e| e.to_string())?;
            compact.store(true, Ordering::SeqCst);
            // 常态浮动：胶囊停在原处，只把越界的位置拉回屏内 —— 不再强制贴边；
            // 只有用户把它拖到屏幕边缘松手，才由 sync_peek_window("docked") 贴死。
            match anchor {
                Some(a) => { apply_anchor(&window, a); }
                None => { clamp_window_to_work_area(&window, 8.0); }
            }
        }
        other => return Err(format!("unknown window mode: {other}")),
    }

    Ok(())
}
```

- [ ] **Step 2: 跑测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS（Task 4 已让全量编译通过）。

- [ ] **Step 3: 真机手测（这一条不能只靠单测）**

```bash
npm run tauri:dev
```

按顺序验：

1. 把胶囊拖到**右边**边缘松手 → 应贴死右缘，把手出现在右侧。
2. 点胶囊展开成主窗 → 主窗右缘应仍贴屏幕右缘，**完全不出屏**（旧版会右侧溢出 232px）。
3. 再收回胶囊 → 胶囊应回到右缘 168×56 贴死，**不是**漂在屏幕中间偏右。
4. 重复一次，确认来回多次不漂移。
5. 再对**左边**做一遍 1~4，把手应出现在左侧且可见。
6. 上下边留到 Task 7 之后验。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/ipc.rs
git commit -m "fix(window): 换模式时按锚点重算位置，修掉贴边往返后掉出贴边态

set_window_mode 原本只 set_size 不 set_position。Win32 保持左上原点，
贴右的胶囊展开成 400 宽后右侧溢出屏幕，随后的 clamp 把它拉到离右缘
400+8px 处；再收回 168 宽时原点不动，胶囊就停在离右缘 240px 的地方。

改成：改尺寸前先读锚点，改完用 anchor_rect 重算位置——贴边轴由边和
尺寸决定，与旧位置无关，所以任何尺寸都贴死且不出屏。"
```

---

## Task 6: 锚点持久化 + 接通 `dashboard_x/y`

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`（启动时按持久化锚点落位）
- Modify: `src-tauri/src/ipc.rs`（落位后回写 settings）

- [ ] **Step 1: `Settings` 增加 `dock` 字段**

在 `settings.rs` 的 `Settings` 里，`compact_mode` 之后加：

```rust
    /// 胶囊上次贴边的锚点。None = 上次是浮动。
    /// 存下来是为了重启后回到同一条边、同一个位置——否则每次启动都靠系统
    /// 随手放，用户看到的现象就是「窗口老在左上角」。
    #[serde(default)]
    pub dock: Option<crate::dock::DockAnchor>,
```

在 `Settings::default()`（或 `SettingsStore` 的初始构造）里加：

```rust
            dock: None,
```

- [ ] **Step 2: 写一个默认值回归测试**

在 `settings.rs` 的既有 `mod tests` 里追加：

```rust
    #[test]
    fn a_config_without_the_dock_field_deserialises_to_none() {
        // 老配置里根本没有 dock 这行；serde(default) 必须给出 None 而不是报错，
        // 否则升级即读不出配置，用户会以为设置被清空了。
        let raw = "\
enabled_providers = []
poll_interval_seconds = 30
compact_mode = true
autostart_hint_shown = false
";
        let s: Settings = toml::from_str(raw).expect("old config must still parse");
        assert_eq!(s.dock, None);
    }

    #[test]
    fn a_config_with_a_dock_field_round_trips() {
        use crate::dock::{DockAnchor, DockSide};
        let a = DockAnchor { side: DockSide::Bottom, along: 640 };
        let s = Settings { dock: Some(a), ..Settings::default() };
        let text = toml::to_string(&s).expect("serialize");
        let back: Settings = toml::from_str(&text).expect("deserialize");
        assert_eq!(back.dock, Some(a));
    }
```

若该测试模块顶部没有 `use`，按现有惯例补 `use super::*;`。

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings`
Expected: PASS。

- [ ] **Step 4: 启动时按锚点落位**

在 `lib.rs` 现有那段「compact_mode_now 时先定尺寸再显示」之后（`let _ = dash.show();` 之前）插入：

```rust
            // 贴边状态是持久的：上次贴在哪儿，这次就回到哪儿。
            let saved_anchor = app.state::<AppState>().settings.get_anchor_now();
            if let Some(a) = saved_anchor {
                let _ = dash.set_size(tauri::LogicalSize::new(
                    if app.state::<AppState>().compact_mode.load(Ordering::SeqCst) { 168u32 } else { 400u32 },
                    if app.state::<AppState>().compact_mode.load(Ordering::SeqCst) { 56u32 } else { 680u32 },
                ));
                ipc::apply_anchor(&dash, a);
            }
```

并在 `SettingsStore` 上加同步读方法（紧邻 `compact_mode_now`）：

```rust
    /// Sync, non-async read of `dock`. Same rationale as `compact_mode_now`.
    pub fn dock_now(&self) -> Option<crate::dock::DockAnchor> {
        self.cache.try_read().ok().and_then(|s| s.dock)
    }
```

- [ ] **Step 5: 落位后回写 settings**

在 `ipc.rs` 的 `apply_anchor` 成功落位之后加回写（`AppState.settings` 是 `Arc<SettingsStore>`，`save` 是 async）：

```rust
    if let Some(state) = window.app_handle().try_state::<AppState>() {
        let settings = state.settings.clone();
        let next = settings.get().await.clone();
        if next.dock != Some(a) {
            let mut updated = next;
            updated.dock = Some(a);
            let _ = settings.save(updated).await;
        }
    }
```

- [ ] **Step 6: 接通 `dashboard_x / dashboard_y`**

这两个字段目前是死的。在 `set_window_mode` 的两个分支末尾（`Ok(())` 之前）补上浮动位置回写：

```rust
    // 浮动位置也要持久化：这两个字段从项目第一天起就只写不读（见
    // docs/superpowers/specs/2026-09-15-focus-provider-and-pill-design.md:133
    // 「等 window-state 插件接管」——那步始终没做）。这里顺手接通。
    if let (Ok(pos), Some(state)) = (window.outer_position(), app.try_state::<AppState>()) {
        let settings = state.settings.clone();
        let next = settings.get().await.clone();
        if next.dashboard_x != Some(pos.x) || next.dashboard_y != Some(pos.y) {
            let mut updated = next;
            updated.dashboard_x = Some(pos.x);
            updated.dashboard_y = Some(pos.y);
            let _ = settings.save(updated).await;
        }
    }
```

并在 `lib.rs` 启动块里，紧接上面的锚点落位之后：

```rust
            if saved_anchor.is_none() {
                // 浮动态：用持久化的自由位置，没有则由系统决定。
                if let (Some(x), Some(y)) = (
                    app.state::<AppState>().settings.get_xy_now(),
                ) {
                    let _ = dash.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }
```

配 `SettingsStore::get_xy_now`：

```rust
    /// Sync, non-async read of the persisted free-floating position.
    pub fn get_xy_now(&self) -> (Option<i32>, Option<i32>) {
        self.cache
            .try_read()
            .map(|s| (s.dashboard_x, s.dashboard_y))
            .unwrap_or((None, None))
    }
```

- [ ] **Step 7: 跑测试与构建**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS。

Run: `npm run tauri:build`
Expected: 两个安装包都产出（这一步顺带验证 `beforeBuildCommand` 的 svelte-check 闸门）。

- [ ] **Step 8: 提交**

```bash
git add src-tauri/src/settings.rs src-tauri/src/lib.rs src-tauri/src/ipc.rs
git commit -m "feat(dock): 贴边锚点持久化，接通死字段 dashboard_x/y

Settings.dashboard_x/y 从项目第一天起只写不读（旧 spec 注明是等
window-state 插件接管后闲置，那步始终没做）——这就是用户看到「窗口老在
左上角」的原因：位置既没持久化也没主动设置，每次启动由系统决定。

现在 dock 锚点与自由位置都回写、启动时都读回。"
```

---

## Task 7: 前端四边落点判定与胶囊滑出方向

**Files:**
- Modify: `src/App.svelte`（`settlePillAfterDrag` + `pillSide` + 胶囊滑出 CSS）
- Modify: `src/lib/types.ts`（`side` 类型扩为四边）
- Modify: `src/lib/dev-mock.ts`（`syncPeekWindow` 的桩返回四边）

- [ ] **Step 1: 改 `settlePillAfterDrag` 的落点判定**

先在 `onPillPointerDown` 里记住拖拽起点（若尚未有该字段则新增）：

```ts
  let pillDragOrigin: { x: number; y: number } | null = null;
```

在 `onPillPointerDown` 起始处、`pillDragStart = { ... }` 之后加：

```ts
    try {
      const p = await getCurrentWindow().outerPosition();
      pillDragOrigin = { x: p.x, y: p.y };
    } catch {
      pillDragOrigin = null;
    }
```

（该函数已是 async；若不是，先改成 async 并在调用处加 `void`。）

然后把 `settlePillAfterDrag` 里从 `try {` 到函数末尾 `}` 的整段替换为：

```ts
    // 落点判定统一交给 Rust 的 nearest_side（四条边 + 角落按拖拽主方向裁决）。
    // 前端只负责把「拖了多远」报上去——自己再算一遍必然和 Rust 的阈值/坐标
    // 空间对不上，这正是旧实现只有 min(距左, 距右) 的原因。
    let side: "left" | "right" | "top" | "bottom" | null = null;
    try {
      const win = getCurrentWindow();
      const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
      const drag = pillDragOrigin
        ? ([pos.x - pillDragOrigin.x, pos.y - pillDragOrigin.y] as [number, number])
        : null;
      side = (await dockSideOf(pos.x, pos.y, size.width, size.height, drag)) as typeof side;
    } catch {
      side = null; // 读不到窗口几何：按浮动处理，至少保证胶囊可交互
    }
    pillDragOrigin = null;

    if (side) {
      // 贴边：先置收起态并同步 docked —— Rust 立刻贴死边缘 + 建把手 + 主窗穿透，
      // 随后进入正常收起链路滑出，因此胶囊滑出时窗口已经贴边。
      pillDocked = true;
      pillRevealed = false;
      void syncPeek().then(() => dockPill());
    } else {
      // 浮动：层保持可见、主窗接管鼠标，把手窗口由 Rust 销毁。
      pillDocked = false;
      pillRevealed = true;
      pillCollapsing = false;
      void syncPeek();
    }
```

`currentMonitor`、`PILL_DOCK_THRESHOLD_PX` 若因此不再被引用，一并删掉（`npx svelte-check` 会指出未使用的局部变量）。`dockSideOf` 是新增的 API 封装（Step 3 定义），需在文件顶部的 import 列表里加进去。

- [ ] **Step 2: 在 `ipc.rs` 加 `dock_side_of` 命令**

```rust
/// 前端拖拽松手时问一次「这次该贴哪条边」。判定逻辑只有一份（在 dock.rs 的
  `nearest_side`），前端不再自己算——旧实现前端算 min(距左,距右)、Rust 算
  中心比较，两套口径迟早对不上。
#[tauri::command]
pub async fn dock_side_of(
    window: tauri::Window,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    drag_x: Option<f64>,
    drag_y: Option<f64>,
) -> Result<Option<String>, String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor".to_string())?;
    let area = monitor.work_area();
    let drag = match (drag_x, drag_y) {
        (Some(dx), Some(dy)) => Some((dx, dy)),
        _ => None,
    };
    Ok(nearest_side(
        x, y, w, h,
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
        DOCK_SNAP_MARGIN_PX * f64::from(monitor.scale_factor()),
        drag,
    )
    .map(|s| s.as_str().to_string()))
}
```

在 `lib.rs` 的 `invoke_handler` 里注册：

```rust
            ipc::dock_side_of,
```

- [ ] **Step 3: 在 `src/lib/api.ts` 加封装**

```ts
/** 拖拽松手时问 Rust 该贴哪条边；null = 不贴边（保持浮动）。 */
export async function dockSideOf(
  x: number,
  y: number,
  w: number,
  h: number,
  drag: [number, number] | null,
): Promise<string | null> {
  return invoke<string | null>("dock_side_of", {
    x,
    y,
    w,
    h,
    dragX: drag?.[0] ?? null,
    dragY: drag?.[1] ?? null,
  });
}
```

- [ ] **Step 4: 胶囊滑出方向支持上下**

`App.svelte` 里把 `pillSide` 的类型从左右扩为四边，并把胶囊层的 docked 类名按边派发：

```ts
  let pillSide = $state<"left" | "right" | "top" | "bottom">("right");
```

模板里胶囊层：

```svelte
<div class="pill-layer" class:is-docked={pillDocked} data-side={pillSide}>
```

对应 CSS（在 `App.svelte` 的 `<style>` 里找到现有 `.pill-layer.is-docked` 规则，改为）：

```css
  .pill-layer.is-docked[data-side="right"]  { transform: translateX(100%); }
  .pill-layer.is-docked[data-side="left"]   { transform: translateX(-100%); }
  .pill-layer.is-docked[data-side="bottom"] { transform: translateY(100%); }
  .pill-layer.is-docked[data-side="top"]    { transform: translateY(-100%); }
```

（保留原有的 `transition: transform 200ms` 与 `PILL_SLIDE_MS = 200` 同步。）

- [ ] **Step 5: 同步 `types.ts` / `dev-mock.ts`**

`src/lib/types.ts` 里若存在贴边相关的联合类型，扩为四边；`dev-mock.ts` 的 `sync_peek_window` 桩改为：

```ts
    case "sync_peek_window":
      // 开发期桩：跟随入参回边，够预览用；真机走 Rust 的 nearest_side。
      return args.state === "floating" ? "right" : (args.__side ?? "right");
```

并把 `get_dock_side_of` 桩加上：

```ts
    case "dock_side_of": {
      const near = 40;
      const cands: Array<[string, number]> = [
        ["left", args.x - 0],
        ["right", 1920 - (args.x + args.w)],
        ["top", args.y - 0],
        ["bottom", 1040 - (args.y + args.h)],
      ];
      const min = Math.min(...cands.map(([, d]) => d));
      return min <= near
        ? cands.find(([, d]) => d === min)![0]
        : null;
    }
```

- [ ] **Step 6: 跑闸门**

Run: `npm run test`
Expected: PASS（若 `dock_side_of` 桩签名与 api.ts 不一致会在这里暴露）。

Run: `npx svelte-check --threshold warning`
Expected: 0 errors 0 warnings。

- [ ] **Step 7: 提交**

```bash
git add src/App.svelte src/lib/api.ts src/lib/types.ts src/lib/dev-mock.ts src-tauri/src/ipc.rs src-tauri/src/lib.rs
git commit -m "feat(dock): 落点判定统一到 Rust，支持上下边贴边

新增 dock_side_of 命令：前端拖拽松手时把窗口几何和拖拽位移报给 Rust，
由 nearest_side 判边。前端不再自己算 min(距左,距右)——两套口径迟早
对不上，这是缺陷 1 长期存在的另一面。

胶囊滑出方向扩为四边。"
```

---

## Task 8: 四边真机验收

**Files:** 无代码改动（除非验收发现问题，按发现补 Task）

- [ ] **Step 1: 起真机**

```bash
npm run tauri:dev
```

- [ ] **Step 2: 四边逐一验收**

对 **左 / 右 / 上 / 下** 四条边各做一遍，每一遍都要走完 5 步：

1. 把胶囊拖到该边松手 → 胶囊贴死该边，**把手出现在该边且肉眼可见**
2. 点胶囊展开成主窗 → 主窗该边仍贴死、**完全不出屏**（上下边时是上/下缘不出屏，左右边时是左/右缘）
3. 收回胶囊 → 回到该边贴死，位置与第 1 步一致
4. 把主窗展开/收起**来回三次**，确认不漂移
5. 杀掉进程重启 → 胶囊仍在上次那条边的同一位置

- [ ] **Step 3: 角落歧义验收**

把胶囊拖到屏幕左上角（同时距左边和上边 < 40px）松手：

- 往右拖动后松手 → 应贴**左**边
- 往下拖动后松手 → 应贴**上**边

- [ ] **Step 4: 浮动回归验收**

把胶囊拖到屏幕**正中**松手 → 应保持浮动（不起把手、不贴边）。这是 `nearest_side` 返回 `None` 的路径。

- [ ] **Step 5: 记录结果**

把四条边的实测（贴边是否精确、把手是否可见、往返是否漂移、重启是否保持）写进 `docs/superpowers/plans/2026-10-02-window-dock-anchor.md` 末尾的「验收记录」小节。

---

## 验收标准

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 全绿，且 `dock::tests` 不少于 24 个
- [ ] `npm run test` 全绿
- [ ] `npx svelte-check --threshold warning` 0 errors 0 warnings
- [ ] `npm run tauri:build` 产出 msi + nsis
- [ ] 四边贴边：贴边精确（0px 间隙）、把手可见、往返不漂移、重启保持
- [ ] 缺陷 2 复现路径已消除：贴**左**边时把手可见
- [ ] 缺陷 1 复现路径已消除：贴**右**边 → 展开 → 收回，胶囊仍贴右
- [ ] 浮动路径未回归：屏幕正中松手保持浮动

## 验收记录（2026-10-02 真机实测，单屏 1920×1080，工作区 1920×1032）

| 项 | 结果 |
|---|---|
| 贴右 → 展开 → 收回，往返 3 次 | 每次 gap=0，**零漂移**（缺陷 1 消除） |
| 贴左边把手 | **可见**（缺陷 2 消除），往返 2 次 gap=0 |
| 贴上边 | gapT=0，把手 136×39 横向条，**把手与胶囊中心偏差 0** |
| 贴边写盘 | `config.toml` 写入 `[dock] side="right" along=35` |
| 重启读回（贴边） | 胶囊回 `x=1752, right=1920`（右缘正好），纵向 = work_area 顶 + along |
| 拖到中央 → 浮动 | `[dock]` 被清掉，`dashboard_x/y` 更新为 1486/722 |
| 重启读回（浮动） | 胶囊回 (1486, 722)，四边距离 266/259，不贴边 |
| 拖到屏幕正中 | 保持浮动（`nearest_side` 返回 `None`） |

### 实施中额外发现并修掉的三个缺陷

原计划只预见两个（缺陷 1、缺陷 2），真机又暴露三个，都已修（commit `f26d7eb`）：

1. **上下边把手变成一大块黑砖**。`peek_rect` 想把系统最小高度撑出的多余
   部分推出屏外（Top 算 `y = -32`），但 **Windows 不允许窗口越过上沿**，
   y 被钳回 0，整块 39px 压在屏幕内沿。改为只画顶部 `thickness` px、
   其余透明。
2. **启动恢复到贴边态时把手不见了**。`App.svelte` 硬编码
   `pillDocked = false`——那是 dock 持久化**之前**的老前提。于是
   `syncPeekWindow("floating")` 把刚恢复的贴边态拆掉：窗口卡在贴边位置
   但把手被销毁。改为启动时问一次 Rust 当前落点贴没贴边。
3. **把手与胶囊横向错开 56px**。`pill_span` 对四条边一律传胶囊的**高**
   56；上下边沿边轴是水平的，该用**宽** 168。新增 `pill_span_for(side)`
   并加测试断言「把手可见条中心必须落在胶囊中心上」。

另修 `peek/main.ts` 的事件竞态：Rust 建窗后立刻 `emit("peek-show")`，但
webview 挂上监听之前事件会被静默丢弃。加了**指针感知**的兜底自显——
收到过 peek-show 就清掉，指针已经进过把手就不自显。

## 已知限制（有意不做）

1. **多显示器热插拔的实时重钳制**：拔掉显示器后，持久化的 `along` 会在下次落位时被 `anchor_rect` 的 clamp 拉回新工作区，可能贴到同一侧的边缘而不是原来那条边。实时纠正需要监听 `MonitorRemoved` 事件重算四边，属于独立任务。
2. **上下边把手的可见条只有 7px，窗口仍是 39px 高**：系统最小高度躲不掉，那多出的 32px 是**透明**的，但仍属 webview，会吞掉该区域的鼠标事件——贴上边时是一条约 136×32 的横向带（正好覆盖在胶囊自身范围内，可接受），贴下边时挂在工作区之外（任务栏一侧）。这是「不让系统钳位成一整块」与「不吞点击」之间的取舍。
3. **角标拖拽方向依赖前端上报**：`drag` 位移由前端计算后传给 Rust。若前端漏报（异常路径），角落歧义会回退到「水平边优先」的确定性默认值，而不是猜。
4. **下边贴边的落位偶发不贴死**（实测 gapB=94px）：四边几何与 `pill_span` 已修正且测试覆盖，但真机上下边拖拽的落位时序仍偶发不贴死。**用户 2026-10-02 决定不追**。上/左/右三边均已验证通过。
