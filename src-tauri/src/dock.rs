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

/// 胶囊应贴靠的水平边。比较窗口中心与工作区中心；正中时归右侧（默认边）。
///
/// ⚠️ 仅供 Task 1 的行为不变迁移使用，Task 2 起由 `nearest_side` 取代。
pub fn dock_side(x: f64, w: f64, area_x: f64, area_w: f64) -> bool {
    x + w / 2.0 >= area_x + area_w / 2.0
}

/// 把窗口横向贴死到最近的水平边缘，纵向保留原位置但夹在工作区内。
/// 把手画在屏幕边缘，胶囊必须紧贴边缘二者才能对齐；纵向是用户自由选择的位置。
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

/// 把手的物理位置：贴死所在边缘，纵向中心对齐胶囊主行（`row_h` 为物理像素）。
///
/// ⚠️ 仅供 Task 1 的行为不变迁移使用，Task 3 起由四边版 `peek_rect` 取代。
pub fn peek_rect(
    dash_y: f64, row_h: f64, is_right: bool,
    visible_w: f64, actual_w: f64,
    area_x: f64, area_y: f64, area_w: f64, area_h: f64,
    peek_h: f64,
) -> (f64, f64) {
    // 系统最小窗口宽度会把把手窗撑到 ~136px。可见的那 7px 必须贴住屏幕边，
    // 多出来的部分一律推到屏幕外——否则贴左边时那一整条透明区域会持续吞掉
    // 桌面上的鼠标事件（点什么都点不到）。
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
}
