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
/// 胶囊收起态的宽（与 `set_window_mode` 的 168×56 保持一致）。
///
/// 沿边方向的胶囊尺寸**随边翻转**：左右边的沿边轴是竖直的 → 取高 56；
/// 上下边的沿边轴是水平的 → 取宽 168。搞错这个会把把手与胶囊错开整整
/// 56px（真机表现：贴上边时把手比胶囊偏左）。
pub const PILL_W: f64 = 168.0;

/// 胶囊在「沿边轴」上的尺寸。
pub fn pill_span_for(side: DockSide) -> f64 {
    if side.is_horizontal_edge() {
        PILL_ROW_H
    } else {
        PILL_W
    }
}

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
///
/// 签名只收真正用到的量：沿边坐标就是「窗口在该轴上的起点减去 work_area 原点」，
/// 与窗口尺寸、工作区尺寸都无关。早先为了和 `anchor_rect` 保持参数对称而收了
/// 9 个参数，其中 4 个从未被使用——只会换来永久的 `unused_variables` 警告，
/// 还会误导读代码的人以为它们参与计算。
pub fn anchor_from_rect(
    x: f64,
    y: f64,
    area_x: f64,
    area_y: f64,
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
    // 下边缘收起已禁用（2026-10 用户拍板）：底部横条把手与任务栏的交互异常
    // （胶囊贴底后把手/唤出不可用）。Bottom 仍是合法的持久化枚举值（旧配置
    // 反序列化需要），只是不再作为贴边判定候选——贴底松手一律视为浮动。
    let cands = [
        (DockSide::Left, x - area_x),
        (DockSide::Right, (area_x + area_w) - (x + w)),
        (DockSide::Top, y - area_y),
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

/// 把手窗的物理位置。
///
/// 约定（Task 4 的 CSS 必须严格照此对齐，否则缺陷 2 会以另一种形式复发）：
/// **可见条永远贴在屏幕边缘。** 两种实现，取决于系统允不允许把多余部分推出屏外：
///
/// | 贴边   | 窗口位置                | 可见条在窗口的 | `.peek` 尺寸/对齐          |
/// |--------|-------------------------|----------------|----------------------------|
/// | Left   | 右缘贴 `ax+thickness`    | 右端           | 填满 + `justify-content: flex-end`   |
/// | Right  | 左缘贴 `ax+aw-thickness` | 左端           | 填满 + `justify-content: flex-start` |
/// | Top    | 顶边贴 `ay`             | **上端**       | `height: thickness`（其余透明）      |
/// | Bottom | 顶边贴 `ay+ah-thickness` | **上端**       | `height: thickness`（其余透明）      |
///
/// 左右两边系统允许负坐标，于是把多余宽度推出屏外、`.peek` 填满整个窗口即可。
/// 上下边**不行**：Windows 会把越过上沿的 y 钳回 0（越下沿同理），窗口必然整块
/// 压在屏幕内沿上——真机表现是「一大块黑砖」。所以上下边改为只画 thickness 高的
/// 一条、其余透明。代价：那块透明区仍属 webview，会吞掉该区域的鼠标事件
/// （贴上边时是一条约 136×32 的横向带，正好覆盖在胶囊自身范围内，可接受）。
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
            // 上下边**不能**沿用左右边「把多余部分推出屏外」的思路：Windows
            // 不允许窗口越过显示器上沿，y 会被钳回 0，于是整块 39px 高的窗口
            // 压在屏幕内沿上（真机表现：一大块黑砖）。改为把窗口顶边对齐工作区
            // 边缘，让**窗口顶部那 thickness px** 就是把手条，其余由 CSS 留空。
            let y = area_y;
            let x = (area_x + along + (pill_span - length) / 2.0
                - (actual_w - length).max(0.0) / 2.0)
                .clamp(area_x, (area_x + area_w - actual_w).max(area_x));
            (x, y)
        }
        DockSide::Bottom => {
            // 同上：窗口顶边对齐「工作区底边 − thickness」，顶部 thickness px
            // 即把手条。多出的高度挂在工作区之外（任务栏一侧），不挡屏幕内容。
            let y = area_y + area_h - thickness;
            let x = (area_x + along + (pill_span - length) / 2.0
                - (actual_w - length).max(0.0) / 2.0)
                .clamp(area_x, (area_x + area_w - actual_w).max(area_x));
            (x, y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA_X: f64 = 0.0;
    const AREA_Y: f64 = 0.0;
    const AREA_W: f64 = 1920.0;
    const AREA_H: f64 = 1040.0;

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
            let a = anchor_from_rect(x, y, AREA_X, AREA_Y, side);
            assert_eq!(a.along, 321, "side={side:?}");
            let (x2, y2) = anchor_rect(a, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H);
            assert_eq!((x, y), (x2, y2), "side={side:?}");
        }
    }

    #[test]
    fn anchor_from_rect_stores_along_relative_to_the_work_area() {
        // work_area 原点不在 (0,0)（副屏在左侧）时，along 必须是相对值。
        let a = anchor_from_rect(-1900.0, 300.0, -1920.0, 0.0, DockSide::Left);
        assert_eq!(a, DockAnchor { side: DockSide::Left, along: 300 });
    }

    #[test]
    fn nearest_side_picks_the_closest_edge() {
        assert_eq!(nearest_side(0.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Left));
        assert_eq!(nearest_side(1752.0, 300.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Right));
        assert_eq!(nearest_side(600.0, 0.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), Some(DockSide::Top));
    }

    #[test]
    fn bottom_edge_docking_is_disabled() {
        // 下边缘收起已禁用：贴底松手一律 None（浮动），即使距离比其他边更近。
        assert_eq!(nearest_side(600.0, 984.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, None), None);
        // 底+左角点：Bottom 被排除后由唯一候选 Left 胜出（往下拖也不贴底）。
        assert_eq!(nearest_side(0.0, 984.0, 168.0, 56.0, AREA_X, AREA_Y, AREA_W, AREA_H, 40.0, Some((3.0, 200.0))), Some(DockSide::Left));
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
    fn peek_top_anchors_the_window_top_to_the_work_area_edge() {
        // 回归：早先算 y = ay + thickness - actual_h（= -32），想让 39px 高的
        // 窗口把多余部分推出屏外。但 **Windows 不允许窗口越过上沿**，y 被钳回 0，
        // 整块 39px 就压在屏幕内沿上——用户看到的「一大块黑砖」。
        // 现在改为：窗口顶边贴工作区上沿，CSS 只画顶部 thickness px。
        let (x, y) = peek(DockSide::Top, 600.0, LEN, STRETCH_H);
        assert_eq!(y, AREA_Y);
        assert_eq!(x, 600.0 + (56.0 - LEN) / 2.0); // 沿边轴居中于胶囊
    }

    #[test]
    fn peek_bottom_anchors_the_window_top_just_above_the_work_area_edge() {
        // 同上：窗口顶边贴「工作区底边 − thickness」，顶部 thickness px 即把手条，
        // 多出的高度挂在工作区之外（任务栏一侧），不挡屏幕内容。
        let (x, y) = peek(DockSide::Bottom, 600.0, LEN, STRETCH_H);
        assert_eq!(y, AREA_H - THICK);
        assert_eq!(x, 600.0 + (56.0 - LEN) / 2.0);
    }

    #[test]
    fn peek_top_and_bottom_put_the_bar_at_the_windows_top_edge() {
        // 上下边的把手条都在窗口**顶部**（CSS 只画顶部 thickness px），
        // 所以窗口 y 必须让「窗口顶 + thickness」正好落在工作区边缘内侧。
        let (_, y_top) = peek(DockSide::Top, 400.0, LEN, STRETCH_H);
        assert_eq!(y_top + THICK, AREA_Y + THICK); // 顶边：条紧贴上沿内侧
        let (_, y_bot) = peek(DockSide::Bottom, 400.0, LEN, STRETCH_H);
        assert_eq!(y_bot, AREA_H - THICK);          // 底边：条紧贴下沿内侧
    }

    /// 上下边的把手条必须**完全在屏内**（不能像左右边那样被钳位后整块压在边内）。
    /// 这是「黑砖」缺陷的护栏：只要条的高度等于 thickness，砖就不会出现。
    #[test]
    fn peek_top_and_bottom_bar_is_only_thickness_tall_and_onscreen() {
        for side in [DockSide::Top, DockSide::Bottom] {
            let (x, y) = peek(side, 400.0, LEN, STRETCH_H);
            assert!(x >= 0.0, "side={side:?} 把手横向出屏");
            assert!(y >= AREA_Y, "side={side:?} 把手顶边越过工作区上沿");
            let bar_bottom = y + THICK;
            assert!(
                bar_bottom <= AREA_Y + AREA_H,
                "side={side:?} 把手条越过工作区下沿（会被系统钳位成一整块）"
            );
        }
    }

    #[test]
    fn peek_never_falls_off_the_screen_along_the_edge() {
        // 贴上边、沿边坐标贴近工作区左缘时，夹回屏内。
        let (x, _) = peek(DockSide::Top, -900.0, LEN, STRETCH_H);
        assert!(x >= 0.0);
        let (_, y) = peek(DockSide::Left, -900.0, STRETCH_W, LEN);
        assert!(y >= 0.0);
    }

    /// 四条边的可见条都必须**紧贴屏幕边缘内侧**（不越界、不留缝）。
    /// 可见条在窗口的哪一端随边翻转——这正是 Task 4 CSS 必须对齐的约定：
    /// 左右边条在窗口的横向两端（多余宽度被推出屏外），上下边条在窗口**顶部**。
    #[test]
    fn peek_visible_strip_always_sits_flush_against_the_screen_edge() {
        for side in [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom] {
            let (aw, ah) = if side.is_horizontal_edge() { (STRETCH_W, LEN) } else { (LEN, STRETCH_H) };
            let (x, y) = peek(side, 400.0, aw, ah);
            let strip = match side {
                DockSide::Left => (x + aw - THICK, x + aw),
                DockSide::Right => (x, x + THICK),
                DockSide::Top => (y, y + THICK),
                DockSide::Bottom => (y, y + THICK),
            };
            let edge = match side {
                DockSide::Left | DockSide::Top => AREA_X.max(AREA_Y),
                DockSide::Right => AREA_X + AREA_W,
                DockSide::Bottom => AREA_Y + AREA_H,
            };
            let expected = match side {
                DockSide::Left | DockSide::Top => (edge, edge + THICK),
                DockSide::Right | DockSide::Bottom => (edge - THICK, edge),
            };
            assert_eq!(strip, expected, "side={side:?}");
        }
    }

    #[test]
    fn pill_span_flips_with_the_edge() {
        // 沿边轴：左右边是竖直的（取胶囊高 56），上下边是水平的（取胶囊宽 168）。
        assert_eq!(pill_span_for(DockSide::Left), 56.0);
        assert_eq!(pill_span_for(DockSide::Right), 56.0);
        assert_eq!(pill_span_for(DockSide::Top), 168.0);
        assert_eq!(pill_span_for(DockSide::Bottom), 168.0);
    }

    /// 回归：把手可见条的中心必须与胶囊中心重合。
    /// 早先 `pill_span` 对四条边一律传 56（胶囊的**高**），于是贴上/下边时
    /// 沿边轴是水平的、实际该用胶囊的**宽 168** —— 把手与胶囊横向错开 56px，
    /// 悬停把手弹出的胶囊不在把手上方。
    #[test]
    fn peek_visible_strip_centre_lands_on_the_pill_centre() {
        // 沿边轴：左右边是 Y，上下边是 X。只在**沿边轴**上比中心。
        for side in [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom] {
            let (aw, ah) = if side.is_horizontal_edge() { (STRETCH_W, LEN) } else { (LEN, STRETCH_H) };
            let along = 400.0;
            let (x, y) = peek_rect(
                side, along, pill_span_for(side), THICK, LEN, aw, ah,
                AREA_X, AREA_Y, AREA_W, AREA_H,
            );
            let pill_c = along + pill_span_for(side) / 2.0;
            let strip_c = if side.is_horizontal_edge() {
                // 左右边：条是窗口的左端或右端，跨满整个窗口高度 → 中心在 Y。
                y + ah / 2.0
            } else {
                // 上下边：条是窗口顶部的 thickness，跨满整个窗口宽度 → 中心在 X。
                x + aw / 2.0
            };
            assert!(
                (strip_c - pill_c).abs() < 0.5,
                "side={side:?} 把手中心 {strip_c} != 胶囊中心 {pill_c}"
            );
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
}
