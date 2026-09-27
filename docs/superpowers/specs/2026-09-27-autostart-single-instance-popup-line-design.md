# 设计：开机自启 + 单例模式 + 弹窗上边缘黑线修复

- 日期：2026-09-27
- 分支：feat/logo-pill-edge-peek
- 状态：设计已获用户批准；两项默认值按推荐执行——单例重复启动=聚焦已运行面板，自启默认=关闭

## 目标
1. 开机自启，并做成设置项，用户可开关
2. 单例模式：同时只允许运行一个程序实例，重复启动不打扰/唤起已有实例
3. 修复 trend/tools/settings 弹窗放大后上边缘出现的黑/白线

## 功能 1：开机自启（设置项）

### 方案选择
采用官方插件 tauri-plugin-autostart（Windows 下写 HKCU\Software\Microsoft\Windows\CurrentVersion\Run，仅当前用户）。
备选方案「手写 winreg 注册表」因需自行维护 exe 路径引号、升级换路径等细节被否决。

### 设计
- Cargo.toml：新增依赖 tauri-plugin-autostart = "2"
- lib.rs：builder 注册插件（MacosLauncher::LaunchAgent，参数空）
- settings.rs：Settings 新增 #[serde(default)] autostart: bool；移除遗留字段 autostart_hint_shown（serde 默认忽略未知字段，旧 config.toml 天然兼容）
- 新命令 set_autostart(enabled: bool)：调用 autolaunch().enable()/disable() 并持久化到 config.toml
- save_settings：检测 autostart 字段变化，即时 enable/disable
- 启动自愈（setup 钩子）：settings.autostart == true 且 is_enabled() == false 时重新 enable（用户手动删注册表项后可自动恢复）
- 设置页前端：新增「开机自启」开关行，样式对齐 close_to_tray 等布尔项
- capabilities 无需改动：插件仅在 Rust 侧调用，不走前端 invoke 插件命令

## 功能 2：单例模式

### 方案选择
采用官方插件 tauri-plugin-single-instance。
备选方案「手写 CreateMutex」因无法唤起已运行实例的窗口（只能静默退出）被否决。

### 设计
- Cargo.toml：新增依赖 tauri-plugin-single-instance = "2"
- lib.rs：该插件必须注册在 builder 链最前（官方要求）
- 回调行为：dashboard 的 unminimize + show + set_focus——第二实例启动时回调在第一实例内执行，随后第二实例进程由插件自动退出；面板此前收进托盘也能被唤起

## 优化 3：弹窗上边缘黑/白线

### 根因
trend/tools/settings 三个窗口与 dashboard 相比缺两处处理：
1. dashboard 配置了 shadow(false)；三个弹窗未设置，Tauri 默认 shadow=true，透明无边框窗口会带出 1px 系统描边（即用户所见"外矩形边"）
2. dashboard 在 WindowEvent::Resized 中重设 dwm_corner::disable_corner_artifacts；Win11 会在尺寸变化后重画 DWM 边框/圆角，三个弹窗只在创建时调用一次，放大后边框复活

### 修复
ipc.rs 三个窗口 builder：
- 追加 .shadow(false)
- 追加 on_window_event(Resized) → disable_corner_artifacts（复用 dashboard 现成模式）
peek 把手窗口保持不动（无问题反馈，控制改动面）。

## 影响面与测试
- 代码：src-tauri/Cargo.toml、src-tauri/src/lib.rs、src-tauri/src/ipc.rs、src-tauri/src/settings.rs、设置页前端组件
- 单元测试：settings 序列化往返（autostart 默认 false、旧字段缺失兼容）
- 手动验证：双开实例（第二开退出且面板唤起）、自启开关写/删注册表项、弹窗放大后上边缘无线条