//! 平台差异收口（调研文档 §〇「共享核心 + 两套外壳」）。
//!
//! 外壳级行为差异（主入口形态、关闭语义、窗口装饰、菜单栏能力）都声明成
//! 数据，调用方按 [`profile`] 分支；业务代码里不允许再散落
//! `cfg(target_os)` 语义判断——cfg 只应该出现在「IO 边界」（路径、FFI、
//! 系统调用）上。新增平台（Linux）时在这里补一份 profile 即可。

/// 主入口形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryEntry {
    /// Windows：托盘图标左键唤出悬浮面板。
    Tray,
    /// macOS：菜单栏常驻（LSUIElement）+ 状态项 popover。
    MenuBar,
}

/// 外壳能力画像。全部是编译期不变量，运行期不会改变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformProfile {
    /// 平台标识（`std::env::consts::OS`：`"windows"` / `"macos"`）。
    /// 前端经建窗初始化脚本注入（`window.__TUM_PLATFORM__`），用于
    /// 快捷键文案（⌘ vs Ctrl）与浅色令牌开关等纯展示分支。
    pub id: &'static str,
    /// 主入口形态。
    pub primary_entry: PrimaryEntry,
    /// 关闭窗口是否强制「隐藏」：macOS 原生语义是 ⌘W = 隐藏、⌘Q 才退出，
    /// 设置里的 `close_to_tray` 不参与判定；Windows 仍由设置决定。
    pub close_hides_forced: bool,
    /// 是否保留 Dock / 任务栏入口。macOS = false（LSUIElement）。
    pub dock_icon: bool,
    /// 菜单栏图标是否直显百分比（`TrayIcon::set_title`，macOS 专属能力）。
    pub menu_bar_title: bool,
    /// 二级窗口（设置/趋势/工具）是否用系统标题栏（Overlay + 红绿灯）。
    pub native_titlebar: bool,
    /// 悬浮面板是否常驻所有虚拟桌面 / Space（macOS 专属）。
    pub visible_on_all_workspaces: bool,
    /// 快捷键修饰键的显示文案。
    pub shortcut_mod: &'static str,
}

/// 当前平台的画像。
#[cfg(target_os = "macos")]
pub fn profile() -> PlatformProfile {
    PlatformProfile {
        id: std::env::consts::OS,
        primary_entry: PrimaryEntry::MenuBar,
        close_hides_forced: true,
        dock_icon: false,
        menu_bar_title: true,
        native_titlebar: true,
        visible_on_all_workspaces: true,
        shortcut_mod: "⌘",
    }
}

/// 当前平台的画像（macOS 以外的全部桌面目标；现阶段即 Windows）。
#[cfg(not(target_os = "macos"))]
pub fn profile() -> PlatformProfile {
    PlatformProfile {
        id: std::env::consts::OS,
        primary_entry: PrimaryEntry::Tray,
        close_hides_forced: false,
        dock_icon: true,
        menu_bar_title: false,
        native_titlebar: false,
        visible_on_all_workspaces: false,
        shortcut_mod: "Ctrl",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// id 必须与 Rust 的平台常量一致：前端浅色令牌、快捷键文案都按它分支，
    /// 拼错会让 macOS 拿到 Windows 外壳（或反过来），而 Windows 上测不出来。
    #[test]
    fn id_matches_target_os() {
        assert_eq!(profile().id, std::env::consts::OS);
    }

    /// 关闭语义与主入口形态必须配套：菜单栏常驻平台关窗即隐藏（永不退出），
    /// 托盘平台维持「由设置决定」的既有行为。
    #[test]
    fn close_semantics_follow_primary_entry() {
        assert_eq!(
            profile().close_hides_forced,
            profile().primary_entry == PrimaryEntry::MenuBar
        );
    }

    /// 菜单栏百分比直显是 macOS 专属能力（TrayIcon::set_title 平台限制），
    /// 其它平台必须关闭，否则运行时会拿到一次无害但无效的调用。
    #[test]
    fn menu_bar_title_only_on_macos() {
        assert_eq!(profile().menu_bar_title, cfg!(target_os = "macos"));
    }
}
