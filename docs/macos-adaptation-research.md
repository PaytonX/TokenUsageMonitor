# macOS 适配调研（2026-11）

> 结论先行：**架构上可行，但不是「加个 target 就能跑」**。目标形态不是把 Windows
> 体验原样复制到 Mac，而是**共享核心 + 两套平台外壳**：Provider/账本/汇率/路由/
> 扫描/贴边几何这些核心逻辑一套代码；入口、窗口装饰、关闭语义、材质、快捷键、
> 菜单按各自平台的原生约定做（详见 §〇）。
>
> 在这个前提下，工作分三类：
>
> 1. **编译/打包级硬阻塞（5 项）**——不修就产不出可用的 `.app`；
> 2. **静默失效（约 10 项）**——凭据、本地扫描、拖拽结算、Dock 图标等会坏而**不报错**；
> 3. **平台契约差异（4 项）**——首次点击被吞、关窗≠退出、无边框窗口拖边、
>    "屏幕边缘"在 macOS 上的含义（菜单栏/Dock 占位）。
>
> 好消息：macOS 的原生外壳恰好能把最不确定的"悬停把手"移出关键路径——菜单栏
> 作为主入口后，贴边胶囊可以退为可选模式，不必在第一天就赌 WKWebView 的 hover。
>
> 本文的判定分三档：
>
> - **已核实**：读了本仓库代码或锁定的依赖源码（tauri 2.12 / tao 0.37.1 /
>   tauri-runtime-wry 2.12 / wry 0.57 / keyring 3.6.3），结论确定。
> - **待真机**：源码上无法定论，必须在真实 Mac 上跑一次（见 §八）。
> - **设计选择**：不是对错问题，需要拍板（见 §九）。

---

## 〇、平台策略：一套核心 + 两套外壳

这是整个方案的顶层约束，后面所有取舍都从这里推导。

**1. 核心不感知平台。** Provider 抓取、账本、汇率、TokenRouter、本地扫描、贴边
几何（`dock.rs` 纯函数）共享一套；平台差异只允许出现在"IO 边界"——路径、FFI、
窗口/托盘/菜单。

**2. 外壳按平台原生，不追求像素级一致。**
Windows 保持现在的"悬浮胶囊 + 托盘 + 贴边"（它已经贴近 Win11 的挂件心智）；
macOS 走"菜单栏常驻 + 原生窗口"，不把 Windows 的交互习惯硬搬过去。

**3. 共享视觉语言，平台化材质与排版。** 圆环、账户色、卡片密度、玻璃面板的
品牌感保持一致；但材质（macOS 原生 vibrancy vs Windows 深色 alpha）、字体
（SF Pro/PingFang vs MiSans）、阴影/圆角/密度按平台 token 走。

**4. 能力差异显式化。** 用 `PlatformProfile`（Rust 生成、前端读取）把"这个平台
有没有菜单栏 / 贴边 / 悬停把手 / 关窗即退出"变成数据，而不是散落各处的 `cfg` 和
`if os === "macos"`。这样新增 Linux 时只需要再加一份 profile。

### 能力矩阵

| 能力 | Windows（保持现状） | macOS（原生方案） |
|---|---|---|
| 主入口 | 托盘左键 → 显示面板 | **菜单栏图标**（左键弹面板/菜单），无 Dock 图标 |
| 任务栏/Dock | `skipTaskbar`（不进任务栏） | `LSUIElement`（不进 Dock、不进 Cmd+Tab） |
| 悬浮面板 | 400×680 置顶，贴边胶囊 | 保留为可选"桌面挂件"，默认不抢焦点 |
| 贴边胶囊 + 把手 | 四边贴边 + 悬停唤出 | **默认关闭**，作为可选"贴边模式"（P3 真机验证通过后再开） |
| 悬停判定 | WebView hover | 宿主侧全局光标轮询（不依赖窗口活跃/聚焦） |
| 关闭语义 | `close_to_tray` 开关；关最后一窗可退出 | **⌘W/关闭 = 隐藏到菜单栏**（原生），⌘Q 才退出，永不因关窗退出 |
| 二级窗口 | 无边框自绘 + 自绘 resize | **原生标题栏**（Overlay + 红绿灯 + hiddenTitle），可拖边改大小 |
| 材质 | 深色 alpha（现状） | 原生 `NSVisualEffectView`（推荐；先出原型；尊重"降低透明度"→ 回退不透明） |
| 字体 | MiSans VF | `-apple-system`（SF Pro + PingFang SC）为推荐；与 MiSans 做 A/B 后定 |
| 快捷键 | 无 | ⌘, 设置、⌘W 隐藏、⌘Q 退出、⌘R 刷新、Esc 关浮层 |
| 右键 | 无菜单 | 原生上下文菜单（显示/刷新/设置/退出） |
| 自启 | HKCU Run 注册表 | LaunchAgent（现状）；可升级 `SMAppService`（macOS 13+，系统"登录项"里更原生） |
| 通知 | 系统通知 | 通知中心；首次启用时请求权限，被拒时引导去系统设置 |

**关键排序结论**：把 macOS 的"菜单栏主入口"放在 P1，而不是最后做。它一旦成立，
贴边/hover 从"核心交互"降级为"可选增强"——最需要真机验证的那部分就不再阻塞
可用版本。

---

## 一、先看结论：分期与工作量

| 阶段 | 内容 | 目标 | 估算（1 人，熟代码，有 1 台 Mac） |
|---|---|---|---|
| **P0 跑起来** | 工具链、keyring、透明窗、打包、open_url | `tauri build` 产出能双击打开的 `.app`，凭据能存 | 1–2 人日 |
| **P1 macOS 外壳** | LSUIElement、**菜单栏 popover（一步到位）**、应用菜单/⌘ 快捷键、关闭语义、不抢焦点、原生标题栏、首次点击、**浅色令牌** | Mac 用户按原生习惯用起来 | 3–4 人日 |
| **P2 功能对齐** | 本地扫描路径、拖拽结算、通知权限、hostname、DPI 切换、后台节流 | 数据与交互在 Mac 上真的可用 | 3–4 人日 |
| **P3 原生质感** | 原生 vibrancy（按原型）、菜单栏百分比直显、贴边模式原生化、阴影/圆角 token、跨 Space | 用起来"像 Mac 软件"，而不是网页套壳 | 2–3 人日 |
| **P4 分发与迁移** | **未签名 DMG**（含 Gatekeeper 说明）、universal、CI 矩阵、Windows→macOS 迁移文档 | 用户按 README 绕过 Gatekeeper 即可用；你自己能无缝切机器 | 1–2 人日 |

合计约 **10–14 人日**（popover 一步到位 + 浅色令牌使 P1 变重，公证从 P4 移除使 P4 变轻）。

**Windows 侧本轮不改功能**：所有外壳改动都用 `cfg` / `PlatformProfile` 隔离，
Windows 只做回归验证（§八 A 组里与平台无关的部分）。

**建议的推进顺序**：P0 一次做完 → 真机跑 §八 的精简清单（只需验证"窗口/凭据/
通知"这几项）→ P1 外壳 → P2 → P3（vibrancy/字体等视觉改动先出原型）→ P4。
贴边模式的真机验证可以放到 P3 之前，失败也不影响主线。

---

## 二、兼容性审计（逐项）

### 2.1 窗口与贴边（项目的核心体验）

| 能力 | 当前实现 | macOS 实际行为 | 判定 |
|---|---|---|---|
| 透明无边框窗 | `tauri.conf.json` `transparent: true` | **必须同时开 `app.macOSPrivateApi: true`**（config schema 明确写了 macOS 透明窗依赖 `macos-private-api` 特性），否则窗口是不透明黑底 | **P0** |
| 窗口置顶 | `alwaysOnTop: true` | 映射到 `NSFloatingWindowLevel`，可用；但默认只出现在当前 Space，且不会盖住全屏 App | 可用，建议加 `visibleOnAllWorkspaces` |
| 不进任务栏 | `skipTaskbar: true` | **macOS 上是空实现**（`tauri-runtime-wry` 对 `target_os="macos"` 的 `skip_taskbar` 直接 `self` 返回）。Dock 里会出现图标 | **P1（已定：LSUIElement）** |
| 工作区几何 | `monitor.work_area()` | macOS 侧 = `NSScreen.visibleFrame`，已正确换算成"左上原点"并排除**菜单栏与 Dock**。贴边几何可直接用 | ✅ 已核实，无需改 |
| 贴边锚点持久化 | `DockAnchor { side, along }`，`along` 相对 work_area | 坐标系一致（物理像素 + 相对原点），换显示器不会落到已拔掉的屏幕 | ✅ 设计正确 |
| 四边贴边 | `dock.rs::nearest_side` | bottom 已禁用；top 会贴在**菜单栏下方**（不是屏幕物理上沿）。语义与 Windows 略有差异，但不影响正确性 | 可用，§九 有个小决策 |
| 贴边滑动动画 | 后台线程每 8ms `set_position` | 可用；macOS 的 `setFrameOrigin` 会派发到主线程，125Hz 的跨线程窗口移动**可能**有掉帧（Windows 也可能，但 Mac 的合成器对帧对齐更敏感） | **待真机** |
| 把手窗 7×58 | 建窗 + 最小尺寸兜底 + `peek_rect` 溢出屏外 | macOS 没有最小跟踪尺寸，`min_inner_size` 兜底无害。贴边模式在 macOS 默认关闭（§〇），开启时建议把手加宽到 10–12pt（7pt 低于 HIG 的命中尺寸） | 可选模式，P3 |
| Windows 11 圆角/DWM | `dwm_corner` FFI | `#[cfg(windows)]`，macOS 不编译；无边框窗口本身没有圆角蒙版，不需要替代品 | ✅ 无需改 |
| 无边框窗口改大小 | `resizable: true`（settings/trend/tools） | Tauri 的 `undecorated_resizing` 只编译在 Windows/Linux，`tauri-runtime-wry` 没有 macOS 分支 → 无边框窗大概率没有拖边抓手。**macOS 直接改用原生标题栏**（Overlay + 红绿灯，§四 P1-6），resize 由系统提供；Windows 保持无边框现状 | **P1（方案已定）** |

### 2.2 交互（最需要真机验证的部分）

| 能力 | 当前实现 | macOS 风险 | 判定 |
|---|---|---|---|
| 胶囊拖拽 | 前端 `pointermove` 超过阈值后 `getCurrentWindow().startDragging()`（`src/App.svelte:1172-1181`） | macOS 的 `performWindowDragWithEvent:` 要求调用时鼠标仍处于按下；IPC 往返后是否仍满足需实测。失败表现：按住拖动没反应 | **待真机** |
| 拖拽结束判定 | Rust 侧 120ms 轮询"窗口停稳 + `lbutton_down()`"（`src-tauri/src/lib.rs:886-918`） | `lbutton_down()` 在非 Windows 恒 `false`（`lib.rs:189-192`）。于是用户**按住不动超过 150ms 就会被判成松手**，中途触发贴边结算，把窗口拽回边缘 | **P2（确定会坏）** |
| 悬停把手唤出 | 把手页 `pointerenter`（`src/peek/main.ts:126`），CSS `:hover` | 非活跃 App 的窗口通常收不到 `mouseMoved`，WKWebView 的 hover 很可能不触发。表现：**贴边收起后，鼠标划过把手胶囊不出来**（主窗此时是穿透的，等于面板不可达）。托盘"显示"仍能救回（`lib.rs:258-285`）。→ 若 macOS 主入口走菜单栏、贴边默认关闭，这条就不在关键路径上；开启贴边模式前必须原生化（§五.1） | **P3（可选模式的门槛）** |
| 首次点击 | 默认 `acceptFirstMouse: false` | 点击一个非活跃窗口的任务栏/胶囊，第一次点击只会激活 App，**不会传给 webview** → 用户要点两下才能展开 | **P1（确定）** |
| 把手窗抢焦点 | 建窗 `.focused(false)` | 不会主动抢焦点；但点击/悬停激活时仍可能把前台 App 切走。开启贴边模式时再加 `.focusable(false)` | P3（可选） |
| 鼠标穿透 | `set_ignore_cursor_events` | macOS 支持（`setIgnoresMouseEvents`），行为一致 | ✅ |
| 右键菜单 | 前端 `oncontextmenu` 全部 prevent | Windows 保持现状；macOS 原生习惯是右键弹上下文菜单，应改为 `Menu::popup`（P1-8） | **P1（macOS）** |

### 2.3 系统集成

| 能力 | 当前实现 | macOS 实际行为 | 判定 |
|---|---|---|---|
| 凭据存储 | `keyring = { features = ["windows-native"] }`（`src-tauri/Cargo.toml:50`） | keyring 3.x 在 macOS 上**只在启用 `apple-native` 时才有 Keychain store**，否则 `Entry::new` 拿不到 store。表现：保存 Key 失败 / 读取永远为空 → 所有 Provider 都 NotConfigured | **P0** |
| 开机自启 | `MacosLauncher::LaunchAgent`（`lib.rs:323-326`）+ 启动自愈 `enable()` | 写 `~/Library/LaunchAgents/<identifier>.plist`，指向 `.app` 内可执行文件；`.app` 搬家后自愈逻辑会重写。比 Windows 注册表更干净 | ✅ 基本可用 |
| 单实例 | `tauri-plugin-single-instance` | macOS 有实现（Unix socket），可用；且 LaunchServices 本身也会复用同一份 `.app` | ✅ |
| 数据目录 | `app_data_dir()`（`lib.rs:338-348`） | `~/Library/Application Support/com.tokenmonitor.app`；SQLite、`config.toml`、窗口状态都落在那里，无需改 | ✅ |
| 系统通知 | `notify::deliver` 直接 `builder().show()`（`src-tauri/src/notify.rs:184`） | 桌面端 `request_permission()` 是 no-op（恒 Granted）；实际走后端 `mac-notification-sys`。**未签名的开发构建可能完全不显示**；正式包需要 bundle id + 签名 | **待真机（P2）** |
| 打开浏览器 | `open_url` 只在 Windows 实现，其它平台直接 `Err`（`src-tauri/src/app_meta.rs:86-103`） | 更新提示 / 关于页里的 GitHub 链接点了没反应 | **P0（一行改）** |
| 托盘图标 | 用 App 默认彩色图标（`lib.rs:695-715`） | 菜单栏图标应是 **template image**（单色、18–22pt），否则在深浅色菜单栏下都难看/模糊；`TrayIconBuilder::icon_as_template` 可用 | **P1（macOS 外壳）** |
| 托盘左键 | `show_menu_on_left_click(false)` + 左键显示面板 | macOS 原生约定是左键弹菜单/面板、右键同一套菜单；"左键直接甩出悬浮窗"是 Windows 习惯 | **P1（按平台分流）** |
| 菜单栏文案 | 无 | `TrayIcon::set_title` **仅 macOS/部分 Linux 支持**（Windows 不支持）。可以把当前最高占用率直接显示在菜单栏 | P3（原生质感） |
| 关闭到托盘 | 拦截 `CloseRequested` → hide（受 `close_to_tray` 控制） | **macOS 原生语义是"关窗=隐藏，应用常驻菜单栏"，不该提供"关窗即退出"**。`close_to_tray=false` 在 Mac 上应被忽略；退出只走 ⌘Q/菜单栏"退出"。实现上仍要拦 `CloseRequested`→hide，并显式处理 `RunEvent::ExitRequested`（隐藏的 `peek` 窗会让窗口表非空，默认退出路径不可靠） | **P1（外壳语义）** |
| 多端同步 hub | 监听 `0.0.0.0:{port}`（`src-tauri/src/hub.rs:79`） | macOS 会弹"是否允许接受传入网络连接"防火墙提示；拒绝后 hub 不可达 | P2（提示/文档） |
| mDNS 发现 | `mdns-sd` | macOS 14+ 需要**本地网络权限**；Info.plist 需要 `NSLocalNetworkUsageDescription`（以及可选的 `NSBonjourServices`）。未声明时系统可能直接拒绝组播 | **P2** |
| 设备名 | `COMPUTERNAME` → `HOSTNAME` → `"unknown"`（`hub.rs:283-294`） | GUI App 从 Finder 启动时 **`HOSTNAME` 通常没有导出** → 设备页显示 "unknown" | P2 |

### 2.4 本地工具扫描（`local/*`）——macOS 上会整体归零

| 扫描器 | 当前路径假设 | macOS 结果 |
|---|---|---|
| 通用 home 解析 | `windows_candidates()` 只读 `USERPROFILE`（`src-tauri/src/local/roots.rs:96-110`） | macOS **没有 `USERPROFILE`** → 默认 home 候选为空。只有用户手填 `tool_data_roots` 或精确覆盖才有数据 |
| Claude Code | `~/.claude/projects` | 路径本身跨平台，修好 home 解析即可 |
| Codex | `~/.codex/sessions` | 同上 |
| ZCode | `~/.zcode/cli/db` | 同上 |
| MiniMax Code | `~/.minimax/...` | 同上 |
| OpenCode | `~/.local/share/opencode/opencode.db` | 同上 |
| Cherry Studio | `%APPDATA%` / `%LOCALAPPDATA%`（`local/cache.rs:280-292`） | macOS 应为 `~/Library/Application Support/CherryStudio/Data/cherrystudio.sqlite` |
| WSL | `wsl -l -q`、`\\wsl$` | 非 Windows 空实现（`local/wsl.rs:26,77`），无害 |

> 这不是"少一个工具"的问题：`local::scan_all` 是面板上工具页/模型页的数据源，
> home 解析为空会让这些页在 macOS 上**永远是空的**，而 UI 不会有任何报错。

### 2.5 前端 / 视觉

| 项 | 现状 | macOS 影响 |
|---|---|---|
| 字体 | MiSans VF 已随包提供（`src/styles/fonts/misans/`，woff2 子集） | 不会缺字，但"像原生"应换 `-apple-system`（SF Pro + PingFang SC）；MiSans 可留给数字/品牌区。建议做平台字体 token，先比对再定 |
| 等宽字体 | `"Cascadia Code", "Cascadia Mono", MiSans VF, Consolas, monospace` | macOS 没有 Cascadia/Consolas → 落到 `monospace` = **Courier New**，数字表格观感明显变差。建议加 `"SF Mono", ui-monospace, Menlo` |
| `backdrop-filter` | 已带 `-webkit-` 前缀，且 `tokens.css` 已注明"透明窗采不到桌面" | ✅ 不会更糟；但 macOS 可以借原生 vibrancy 真正做到"模糊桌面"（§5.3） |
| 滚动条 | `::-webkit-scrollbar` + `scrollbar-width: thin` | `scrollbar-width` 在旧 WebKit 不生效，但有 `::-webkit-` 兜底，可用 |
| 首次点击 | 见 §2.2 | 需 `acceptFirstMouse` |
| 设置窗尺寸 | 820×780（`ipc.rs:1445`） | 13" MacBook 默认分辨率（如 1440×900 逻辑）下，780 高 + 菜单栏 + Dock **可能超出工作区**；`center()` 不保证夹回屏内。建议加 `preventOverflow: true` 或按 `work_area` 夹取 |
| 快捷键 | 无自定义全局快捷键；菜单用 Tauri 默认 macOS 菜单（`enable_macos_default_menu` 默认 true） | ✅ Cmd+C/V 可用 |
| **WebView 引擎** | Windows = WebView2（Chromium）；macOS = **系统 WKWebView（Safari 内核）** | 不是"换个壳"，是换引擎。本项目用到的现代 CSS 有硬性系统版本门槛，见 §2.6 |
| 文案 | 多处硬编码 Windows：`settings.accounts.hint` / `about.feat3` / `about.credKeyring` 写 DPAPI；自启提示写"写入注册表"；数据根占位符 `D:\Lab\.agentdata` | 需要按平台分支（`zh-CN.ts:142,357,381,195`；`Settings.svelte:2082`） |

### 2.6 WebView 引擎差异（直接决定最低系统版本）

Windows 用 WebView2（Chromium），macOS 用**系统 WKWebView**——Safari 内核，版本随
macOS 走。项目里用到的现代 CSS 与所需系统版本：

| 特性 | 用在哪 | Safari 版本 | 对应 macOS | 低于该版本的表现 |
|---|---|---|---|---|
| `color-mix()` | Settings 的状态色/边框/阴影（`Settings.svelte:3053,3137,3915,4019,4104-4106`） | 16.2 | **13.1** | 声明整条失效 → 状态色描边/底色直接消失（不报错，只是"没颜色"） |
| `:has()` | 卡片焦点环（`ProviderCard.svelte:388`）、全局焦点兜底说明（`tokens.css:187`） | 15.4 | 12.3 | 键盘焦点环不出现（可访问性退化） |
| `@container` | 日历区响应式（`CalendarSection.svelte:471`） | 16.0 | 13.0 | 规则被忽略，保持默认布局（通常不致命） |
| `aspect-ratio` | 热力图格子（`HeatmapGrid.svelte:554`） | 15.0 | 12.0 | 格子高度塌陷 |
| `inset` | 多处绝对定位 | 14.1 | 11.3 | 定位失效 |

结论：**原生不降级的最低版本是 macOS 13.1**。若要把下限压到 12.x/11.x，需要：

- 给 `color-mix()` 补 `rgba()` 回退（机械但量大，约 10 处）；
- 接受 `:has()` 焦点环在旧系统缺失（或用 JS 加 class 兜底）；
- 给 `aspect-ratio` 补 padding-top 兜底。

对一个 2026 年新发的开发者工具，推荐直接 `minimumSystemVersion: "13.1"`（覆盖
2017 年后的绝大多数 Mac，且 Apple Silicon 全系满足），比维护一套回退便宜得多。

### 2.7 构建与分发

| 项 | 现状 | macOS 结果 |
|---|---|---|
| 工具链 | `rust-toolchain.toml` 钉 `stable-x86_64-pc-windows-gnu` + `targets = ["x86_64-pc-windows-gnu"]` | **macOS 上无法使用**：这是 Windows 宿主工具链。macOS 必须在命令前覆盖 `RUSTUP_TOOLCHAIN`（CI 里已经在 Windows 上这么做了，见 `ci.yml` 注释） | **P0** |
| `.cargo/config.toml` | 只配了 `[target.x86_64-pc-windows-gnu]` + `[build] target-dir` | 目标专属配置不影响 macOS；`target-dir` 无碍 | ✅ |
| `build.rs` | Windows 条件分支（MSVC delayload / GNU manifest） | `tauri_build::build()` 通用；Windows 分支不执行 | ✅ |
| bundle targets | `["msi", "nsis"]` | macOS 上没有这两个 target，产不出安装包 | **P0** |
| bundle resources | `vendor/WebView2Loader.dll` | 会被原样塞进 `Contents/Resources`（无意义但通常不致命）；应移到 Windows 专属配置 |
| 图标 | `icons/icon.icns` 已存在 | ✅ 可打包；但外形是否满足 macOS 图标规范（圆角方块 + 留白）需要看一眼 |
| CI | `windows-latest` × 2 + tag 发布 | 没有 macOS 构建、没有公证；`macos-latest` 是 arm64，做 universal 要显式 `--target universal-apple-darwin` |
| 最低系统 | 未设，Tauri 默认 10.13 | **CSS 要求 ≥ 13.1**（见 §2.6）；若压到 12.x/11.x 需要补 `color-mix` / `:has` / `aspect-ratio` 回退 |
| 签名/公证 | 无 | 未签名 DMG 下载后 Gatekeeper 会拦（macOS 15 起"右键打开"也被收进系统设置），必须签名 + 公证才谈得上"用户直接能用" |

---

## 三、P0 清单（不做就没有可用的 macOS 版本）

1. **工具链**：macOS 构建前覆盖 `RUSTUP_TOOLCHAIN`（例如 `stable-aarch64-apple-darwin`）。
   最小改动：新增 `npm run tauri:build:mac` 脚本，内部 `cross-env RUSTUP_TOOLCHAIN=stable npm run tauri:build`；
   CI 的 macOS job 直接设 env。**不要**把 `rust-toolchain.toml` 改成 `stable`——那会
   改变 Windows 贡献者的默认工具链（本项目刻意选 GNU 以避开 MSVC）。
2. **keyring**：`features = ["windows-native", "apple-native"]`。两个特性互不干扰
   （依赖都在 `[target.'cfg(...)']` 下，跨平台编译安全）。
3. **透明窗**：`app.macOSPrivateApi: true`。Tauri CLI 在 build 时会校验/同步
   `Cargo.toml` 里 `tauri` 的 features（`tauri-build::manifest::check` +
   `AppConfig::features()`），所以**只改配置、别手改 features**；反过来说，裸
   `cargo build` 会因 feature 不匹配而报错，正式构建必须走 `npm run tauri:build`。
   注意：用了私有 API 就**不能上 Mac App Store**（本项目走直发，无影响）。
4. **打包配置**：把 Windows 专属项从基础配置里拆出去，用 Tauri 的平台配置文件合并：
   - `tauri.conf.json`：保留 `productName` / `identifier` / `build` / `app.windows` / 图标；
   - `tauri.windows.conf.json`：`bundle.targets: ["msi","nsis"]`、`resources`（WebView2Loader）、
     `bundle.windows.webviewInstallMode`；
   - `tauri.macos.conf.json`：`bundle.targets: ["app","dmg"]`、
     `bundle.macOS.minimumSystemVersion: "13.1"`、`infoPlist`、可选 `dmg` 布局。
   文件名与合并顺序已核实（`tauri-utils` 的 `parse.rs`：`tauri.macos.conf.json`）。
5. **open_url**：macOS 用 `/usr/bin/open <url>`，Linux 用 `xdg-open`，Windows 保持
   `explorer`。注意保持现有 HTTPS + 主机白名单校验（`is_allowed_open_url`）。

---

## 四、P1/P2/P3 工作清单

### P1 — macOS 外壳（先做，它决定交互面）

1. **无 Dock 图标 + 菜单栏常驻**：`LSUIElement=1`（Info.plist）。比在 setup 里调
   `set_activation_policy(Accessory)` 更干净（启动不闪 Dock）；两者选一，别同时用。
2. **菜单栏入口**：模板图标（`icon_as_template(true)`，18–22pt 单色）；左键先弹
   轻量菜单（P3 再升级成 popover 面板）；右键菜单含 显示面板/刷新/设置/退出。
   macOS 上不要沿用 Windows 的"左键直接甩出悬浮窗"。
3. **应用菜单 + 快捷键**：自定义 macOS 菜单（App/Edit/Window/Help），加
   `Settings…`（⌘,）、`Hide`（⌘H）、`Quit`（⌘Q）、`Close`（⌘W → hide）、刷新（⌘R）。
   Tauri 默认菜单只解决 Cmd+C/V；`Settings…` 与 ⌘, 需要自己建。
4. **关闭语义**：macOS 忽略 `close_to_tray`，`CloseRequested` 恒 hide；退出只由
   ⌘Q / 菜单项触发（`app.exit(0)`）。不要沿用"关最后一窗即退出"的 Windows 语义。
5. **启动不抢焦点**：`focus: false`；`show_dashboard` 只在用户主动点菜单栏/托盘时
   `set_focus`（macOS 的 `set_focus` 会 `activateIgnoringOtherApps`，开机自启时很扰人）。
6. **二级窗口原生化**：settings/trend/tools 用 `decorations: true` +
   `titleBarStyle: "Overlay"` + `hiddenTitle: true` + `trafficLightPosition`；
   内容顶部留红绿灯高度。顺带解决拖边 resize，无需自绘抓手。
7. **首次点击**：dashboard / peek 加 `acceptFirstMouse: true`。
8. **右键菜单**：胶囊/面板右键弹原生菜单（`Menu::popup`），不要再 preventDefault
   之后什么也不给。
9. **Windows 回归**：以上都用 `cfg` / `PlatformProfile` 隔离；`acceptFirstMouse`
   这类配置在 Windows 上无副作用，但菜单/关闭语义必须保持原状。

### P2 — 功能对齐（数据与交互真的可用）

1. **home 解析**：`windows_candidates` → `home_candidates`（`dirs::home_dir()`）；
   否则工具页/模型页在 macOS 上恒空（这是最容易漏的一项）。
2. **Cherry Studio**：追加 `~/Library/Application Support/CherryStudio/Data/...`。
3. **拖拽结算**：`lbutton_down` 的 macOS 实现（`CGEventSourceButtonState`）。
4. **通知权限**：首次开启阈值提醒时请求；被拒时给"打开系统设置"的入口
   （桌面端插件的 `request_permission` 是 no-op，需要按 macOS 实际行为处理）。
5. **hostname**：`gethostname` / `scutil --get ComputerName`，设备页不再 unknown。
6. **DPI 切换**：`WindowEvent::ScaleFactorChanged` → 重新贴边/定位把手。
7. **设置窗尺寸**：`preventOverflow` 或按 `work_area` 夹取（13" 屏会顶到 Dock）。
8. **后台节流**：`backgroundThrottling: "disabled"`（macOS 14+ 生效）。
9. **mDNS / hub**：Info.plist 的 `NSLocalNetworkUsageDescription`；hub 走 LAN 时
   提示防火墙弹窗。

### P3 — 原生质感 / 可选增强

1. **原生 vibrancy**：先出原型（面板不透明度要降），并尊重"降低透明度"回退不透明。
2. **菜单栏百分比标题**：`TrayIcon::set_title("42%")`；进阶用 `TrayIcon::rect()`
   把面板定位到图标下方，做成 popover 观感。
3. **贴边模式原生化**：宿主光标轮询（§五.1），真机验证通过后再默认开启。
4. **字体/阴影 token 平台化**：macOS 用 `-apple-system`（SF Pro/PingFang SC）与
   原生 shadow；Windows 保持 MiSans。
5. **跨 Space + 外观**：`visibleOnAllWorkspaces`；是否跟随系统浅/深色（决策项）。

---

## 五、macOS 上体验更好的方案（不是"对齐 Windows"，而是"更像 Mac"）

### 5.1 悬停判定原生化（只服务"可选贴边模式"）

> 若采纳 §〇 的菜单栏优先策略，贴边胶囊在 macOS 上默认关闭，这一节就是**开启贴边
> 模式的前置条件**，而不是首发阻塞项。它同时也是 Windows 侧的健壮性升级。

现状靠"把手窗 + CSS hover"，本质是在赌"非活跃窗口也能收到鼠标移动"。macOS 上
这个假设大概率不成立。把判定移到宿主侧（轮询全局光标 + 几何命中）之后：

- 不依赖窗口是否 key、是否活跃、是否穿透；
- 把手窗可以永远 `ignore_cursor_events(true)`，只做渲染，三态切换的竞态消失；
- 托盘"显示"、拖拽结算、贴边收起共用同一套坐标（`anchor_rect` / `peek_rect` 已有），
  不会再出现"前端以为在把手上、宿主以为在浮动"的分歧。

代价：一个 60–80ms 的 tokio 轮询（每 tick 一次 FFI + 几次浮点比较），可忽略；
且 macOS 的 `CGEventSourceButtonState` / `CGEventGetLocation` 都不需要任何权限。

### 5.2 菜单栏常驻（macOS 的"主入口"）

macOS 用户的直觉是"常驻工具 = 菜单栏图标"，而不是 Dock 里的一个窗口 App。
既然目标是"贴合各平台原生体验"，macOS 的主入口就应该放在菜单栏，Windows 继续
用托盘 + 悬浮胶囊。

**形态建议（从轻到重）**：

1. **菜单栏图标 + 左键菜单**（P1，最小可用）：菜单项 = 显示面板 / 立即刷新 /
   设置… / 退出。零新窗口，最稳。
2. **菜单栏图标 + 左键弹小面板**（P3，最原生）：用 `TrayIcon::rect()` 拿到图标
   位置，把 168×56 的胶囊内容（圆环 + 百分比 + 账户色）摆到图标正下方，点外面
   或 Esc 收起。这是 macOS 监控类 App 的标准形态（iStat Menus / Stats 都是）。
   实现要点：无边框 + 无阴影 + `alwaysOnTop` + 监听 `Focused(false)` 自动隐藏；
   Tauri 没有 NSPanel popover，但这个尺寸的普通窗口足够像。
3. **百分比直显**：`TrayIcon::set_title(Some("42%"))` 是 macOS 专属能力，把最
   紧张的窗口占用率直接放在菜单栏——零额外 UI，比悬浮胶囊更"一眼可见"。

**配套的原生细节**：

- **无 Dock 图标**：`LSUIElement=1`（Info.plist）优先；`set_activation_policy`
  （Accessory）作为运行时备选，二者选一。
- **模板图标**：`icon_as_template(true)` + 单色 18–22pt 图，深浅色菜单栏自动适配；
  彩色 App 图标直接用会在深色菜单栏上发灰、在白底上过艳。
- **左右键分工**：左键弹面板/菜单，右键弹同一套菜单（macOS 用户不习惯"右键才是
  菜单"）。Windows 保持现状（左键显示面板）。
- **应用菜单**：激活时显示 App/Edit/Window/Help；`Settings…` ⌘,、`Close` ⌘W
  （hide）、`Quit` ⌘Q。这比"托盘里藏退出"更符合 Mac 习惯。

### 5.3 原生 vibrancy（真正的"毛玻璃"）

Windows 侧 CSS `backdrop-filter` 采不到桌面（`tokens.css` 已有注释），所以现在是
"高不透明度深色 + 圆角"。macOS 的 `NSVisualEffectView` 可以真的模糊桌面：

```json
"windowEffects": {
  "effects": ["hudWindow"],
  "state": "active",
  "radius": 16
}
```

前提是窗口 `transparent: true` + `macOSPrivateApi`，并且 WebView 背景透明。
要真正看见效果，面板底色不透明度要从 0.92 降到 0.45–0.6 量级——**这是一次视觉
改版**，按仓库规矩必须先出交互原型（HTML mockup）确认，不能直接改 token。
建议作为 P3 的独立开关项，默认先关。

### 5.4 跨 Space 常驻

`visibleOnAllWorkspaces: true`（macOS 支持，Windows 不支持）。桌面挂件跟着用户
切 Space 走，符合"常驻看板"的心智。注意：它**不能**盖住别人的全屏 App
（floating level 不够），这一点需要写进文档，避免被当 bug。

### 5.5 方案对比：继续 Tauri，还是原生 SwiftUI 菜单栏 App？

调研"新平台方案"绕不开这个问题，先说结论：**继续 Tauri，不重写**。

| 维度 | 继续 Tauri（推荐） | 原生 SwiftUI/AppKit 壳 |
|---|---|---|
| 复用 | Provider 抓取、账本、汇率、路由代理、本地扫描 ≈ 2.4 万行 Rust 直接复用；前端 Svelte 复用 | 要么用 UniFFI 把 Rust 核心包成库（可行，但要维护 FFI 层），要么全部重写 |
| 平台差异面 | 只有窗口/托盘/路径/凭据这几百行需要 `cfg` | 全部 UI + 交互重写，且与 Windows 版永久分叉 |
| 能耗/体积 | WKWebView 常驻（每个窗口一个 WebView 进程），比原生高 | 明显更省 |
| 原生能力 | vibrancy/菜单栏/托盘都能拿到，但 popover、NSPanel 这类要自己补 | 全部原生，popover/菜单栏/通知/登录项都是系统 API |
| 维护成本 | 一套代码，两个 target | 两套代码，两个发布节奏 |

只有当 macOS 版要做成"完全不同的产品形态"（比如纯菜单栏 popover、没有悬浮窗）时，
原生壳才值得考虑；即便如此，也可以先走 Tauri + 少量 macOS 专属窗口，把验证成本
压到最低。**本项目现阶段不建议原生重写。**

### 5.6 其他小而值的点

- **启动不抢焦点**：`focus: false`。开机自启时 macOS 的焦点切换比 Windows 明显得多；
  现在 `setup` 里 `dash.show()` 会让面板抢到前台。
- **等宽字体**：`--tum-font-mono` 加 `"SF Mono", ui-monospace, Menlo`，否则数字退到
  Courier New。
- **把手加宽**：7pt 的可见条在 Retina 上仍然很细，建议 macOS 上 10–12pt（几何函数
  已参数化，改 `PEEK_THICK` 的平台分支即可）。
- **`prefers-reduced-motion`**：已经处理，macOS 的"减弱动态效果"会正确生效。
- **"降低透明度"**：macOS 辅助功能里可以要求减少透明效果。用了 vibrancy 之后必须
  监听并回退到不透明底色，否则在开启该选项的用户机器上会变得不可读。
- **Windows 侧不退化**：这次的原则是"各自原生"，不是"向 Mac 看齐"。Windows 的
  无边框胶囊、托盘、DWM 处理都保持现状；将来若做材质（Win11 Mica/Acrylic），
  与 macOS vibrancy 共用同一个 `material` token 抽象即可，现在不必动。

---

## 六、代码改造清单（按文件）

> **状态（2026-11-03）**：下表 P0 全部与 P1 的 Rust/配置/能力清单部分已落地
> （含浅色令牌、popover 页面、CI/Release 的 macOS job）；真机验收清单见 §八。
> i18n 的平台分支文案（Keychain vs DPAPI）与组件级浅色精修留待真机阶段，
> 见文末「遗留」。

| 文件 | 改动 |
|---|---|
| `src-tauri/Cargo.toml` | keyring 加 `apple-native` |
| `src-tauri/tauri.conf.json` | 加 `app.macOSPrivateApi: true`；dashboard 加 `acceptFirstMouse`、`focus: false`、`backgroundThrottling: "disabled"`（macOS 14+）；拆出平台专属 bundle 配置 |
| `src-tauri/tauri.windows.conf.json`（新） | `bundle.targets` / `resources` / `webviewInstallMode`；Windows 窗口行为不变 |
| `src-tauri/tauri.macos.conf.json`（新） | `bundle.targets: ["app","dmg"]`、`minimumSystemVersion: "13.1"`、`infoPlist`（`LSUIElement`、`NSLocalNetworkUsageDescription`）、DMG 布局；`visibleOnAllWorkspaces` |
| `src-tauri/src/platform/`（新） | `mod.rs` 定义 `PlatformProfile`；`windows.rs` / `macos.rs` 实现 `lbutton_down` / `open_url` / `app_support_dirs` / `hostname` / `apply_window_chrome` |
| `src-tauri/src/lib.rs` | 按 profile 构建托盘/菜单栏（模板图标、标题、左右键）；macOS 关闭语义；启动不抢焦点；`show_dashboard` 聚焦策略 |
| `src-tauri/src/menu.rs`（新，或并入 lib.rs） | macOS 应用菜单：About / Settings… ⌘, / Hide ⌘H / Close ⌘W / Quit ⌘Q；胶囊右键菜单（`Menu::popup`） |
| `src-tauri/src/ipc.rs` | `build_peek_window` 加 `.accept_first_mouse(true).focusable(false)`；三个子窗的 macOS 原生标题栏/尺寸策略；`ScaleFactorChanged` 重新定位 |
| `src-tauri/src/dock.rs` | `PEEK_THICK` 的平台分支（贴边模式开启时）；几何算法不动 |
| `src-tauri/src/local/roots.rs` | `windows_candidates` → `home_candidates`（`dirs::home_dir()`），保留新鲜度裁决 |
| `src-tauri/src/local/cache.rs` | Cherry Studio 的 macOS 路径；`%APPDATA%` 收敛到平台模块 |
| `src-tauri/src/local/wsl.rs` | 无需改（非 Windows 空实现） |
| `src-tauri/src/app_meta.rs` | `open_url` 的 macOS/Linux 实现；`get_app_meta` 暴露 `platform` |
| `src-tauri/src/hub.rs` | hostname 用 `gethostname`；bind 提示 |
| `src-tauri/src/notify.rs` | 结构不变；macOS 补权限状态检查/引导 |
| `src/lib/platform.ts`（新） | 前端读取 `PlatformProfile`：快捷键文案（⌘ vs Ctrl）、菜单栏/贴边开关、字体/材质 token 选择 |
| `src/lib/i18n/{zh-CN,en}.ts` | 凭据（Keychain vs DPAPI）、自启（登录项 vs 注册表）、数据根占位符按平台分支 |
| `src/styles/tokens.css` | 平台 token：字体栈（macOS `-apple-system` + PingFang SC）、阴影、mono 栈补 `SF Mono`/`ui-monospace`/`Menlo` |
| `.github/workflows/ci.yml` | 加 `macos-latest` 的 Rust 测试 job（`RUSTUP_TOOLCHAIN=stable`） |
| `.github/workflows/release.yml` | 加 macOS 构建 + 公证 + 上传 DMG；注意 `rust-toolchain.toml` 覆盖 |
| `README*.md` / `docs/` | 平台徽章、安装说明、Gatekeeper 说明、迁移指南 |

平台差异建议收口成一个**数据**而不是散落判断：

```rust
// platform/mod.rs（示意）
pub struct PlatformProfile {
    pub id: &'static str,              // "windows" | "macos"
    pub primary_entry: PrimaryEntry,   // Tray | MenuBar
    pub close_hides: bool,             // macOS: true（恒隐藏）；Windows: 读设置
    pub dock_icon: bool,               // macOS: false（LSUIElement）
    pub edge_dock_default: bool,       // Windows: true；macOS: false（可选增强）
    pub native_titlebar: bool,         // macOS: true
    pub menu_bar_title: bool,          // macOS: true
    pub shortcut_mod: &'static str,    // "Ctrl" | "⌘"
}
```

前端拿到这份 profile 后只做"渲染哪种外壳"，不再自己嗅探 OS；Rust 侧也只有
`platform::{windows,macos}` 两个实现文件，后续加 Linux 只多一份。

## 七、分发与本机迁移

### 7.1 产物

- `bundle.targets: ["app", "dmg"]`；DMG 里放 `.app` + `/Applications` 软链。
- **universal binary**：`tauri build --target universal-apple-darwin`
  （同时装 `aarch64-apple-darwin` 与 `x86_64-apple-darwin`）。
  只发 arm64 会漏掉 Intel Mac；只发 x86_64 则 Apple Silicon 走 Rosetta（能用但慢）。
- `minimumSystemVersion: "13.1"`（由 §2.6 的 CSS 特性矩阵决定）。若坚持支持
  12.x/11.x，需要先补 `color-mix()` / `:has()` 的回退，再单独评估。

### 7.2 签名与公证

需要 Apple Developer Program（$99/年）。Tauri 的流程：

1. `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` / `APPLE_SIGNING_IDENTITY`
   导出 Developer ID Application 证书；
2. `bundle.macOS.hardenedRuntime: true`（公证硬性要求）；
3. 公证：`APPLE_ID` + `APPLE_PASSWORD`（App 专用密码）+ `APPLE_TEAM_ID`，
   或 `APPLE_API_KEY` / `APPLE_API_ISSUER` / `APPLE_API_KEY_PATH`；
4. Tauri 会完成 sign → notarize → staple（在 CI 上要留足超时）。

不需要额外的 entitlements（不沙盒、不加载第三方 dylib、WKWebView 是系统框架）。
**不要**开 App Sandbox——扫描 `~/.claude` 这类点目录会被沙盒直接拒绝。

### 7.3 没有证书时的退路

- 发未签名 DMG，README 写明 `xattr -dr com.apple.quarantine /Applications/TokenUsageMonitor.app`
  或"系统设置 → 隐私与安全性 → 仍要打开"；
- 或只提供源码构建（`npm ci && npm run tauri:build`，本机自签）；
- Homebrew Cask 也需要公证才能免警告，不是绕过方案。

**建议**：既然项目已经发 Windows 安装包，macOS 若要走"下载即用"，公证这 $99
基本绕不开；这属于产品决策，不是技术决策。

### 7.4 CI

```yaml
jobs:
  macos:
    runs-on: macos-latest          # arm64
    env:
      RUSTUP_TOOLCHAIN: stable
    steps:
      - checkout / setup-node / npm ci
      - npm test && npx svelte-check
      - cd src-tauri && cargo test
      - npm run tauri:build -- --target universal-apple-darwin
```

注意：`macos-latest` 只有 arm64，`universal-apple-darwin` 需要额外
`rustup target add x86_64-apple-darwin`；公证 secrets 只在 tag 发布时注入。

### 7.5 本机从 Windows 迁移到 macOS

> 逐步操作手册（Mac 基础操作、工具链安装、数据迁移、验收清单与排错）见
> [`docs/macos-migration-sop.md`](macos-migration-sop.md)。本节只保留数据层面的结论。

既然是"换机器继续用"，这些数据/配置的迁移路径要提前明确：

| 数据 | 位置（Windows） | 位置（macOS） | 能否直接搬 |
|---|---|---|---|
| 设置 | `%APPDATA%\com.tokenmonitor.app\config.toml` | `~/Library/Application Support/com.tokenmonitor.app/config.toml` | 可以，但要清理平台相关字段（见下） |
| 用量账本 | 同目录 `token_usage_monitor.db` | 同目录 | **可以**（SQLite + epoch 时间戳，跨平台无差异）。复制前先退出应用；有 `-wal`/`-shm` 就一起拷，或先在应用内 checkpoint |
| 凭据 | Windows 凭据管理器（DPAPI） | 登录钥匙串（Keychain） | **不能**。DPAPI 与 Keychain 不互通，必须重新录入 API Key |
| 本地工具日志 | `~\.claude` / `~\.codex` / `~/.zcode` / `~/.minimax` / `~/.local/share/opencode` | 同相对路径 | 可以（拷到 Mac 的 `~` 下）；Cherry Studio 改到 `~/Library/Application Support/CherryStudio/Data` |
| 窗口位置 | `config.toml` 的 `dock` / `dashboard_x/y` | 同字段 | **不要搬**：Windows 物理坐标在 Mac 上无意义 |

**`config.toml` 搬过去后必须处理的字段**：

- `tool_data_roots` / 每工具覆盖：`D:\...` 之类的 Windows 绝对路径在 macOS 上无效，
  清空（空数组 = 只用默认 home + 新声明的 macOS 路径）；
- `dock` / `dashboard_x/y`：清空，让首次启动重新夹取/吸附；
- `autostart = true`：可保留，首次启动的 `enable()` 会按新 `.app` 路径重写 LaunchAgent；
- `proxy_url`：若指向本机代理（如 `127.0.0.1:7890`），确认 Mac 上端口一致；
- `hub_base` / `hub_token`：跨设备同步可以继续用，但要确认 Mac 能解析/访问那个地址。

**开发环境（第一天）**：

```bash
xcode-select --install
brew install rustup && rustup-init     # 或按 rustup 官网的方式安装
npm ci
RUSTUP_TOOLCHAIN=stable npm run tauri:dev   # 绕过 windows-gnu 的 rust-toolchain.toml
```

**首次启动验收**：重录凭据 → 清空无效数据根 → 工具页有数 → 设自启 → 通知权限 →
关窗仍在菜单栏 → ⌘Q 才退出。

---

## 八、必须真机验证的清单（源码无法定论）

按"是否阻塞首发"分两组。P0 完成后先跑 A 组；B 组只在决定开启贴边模式时跑。

### A. 首发必验（菜单栏外壳 + 数据）

1. **首次点击**：Finder/终端为前台时，点胶囊/面板是否一次就生效？
2. **关闭语义**：⌘W / 红绿灯关闭是否只隐藏、应用仍在菜单栏？⌘Q 才退出？
3. **启动焦点**：开机自启/启动时是否抢前台？（预期：不抢）
4. **菜单栏**：模板图标在浅/深色菜单栏与选中态是否正常？左键菜单/面板是否符合预期？
5. **二级窗口**：settings/trend/tools 的红绿灯位置、拖边 resize、⌘W 行为是否自然？
6. **凭据**：保存 Key → 重启 → Keychain 是否只弹一次授权？重新构建后是否重复弹？
7. **数据**：工具页/模型页是否有数（home 解析修好后）？Cherry Studio 是否命中？
8. **通知**：签名 bundle 下阈值通知是否出现？权限被拒后的引导是否可用？
9. **拖拽**：按住胶囊拖动是否跟手？中途停顿 200ms 会不会被误判松手？
10. **多显示器**：Retina 内屏 + 1080p 外屏，窗口/贴边/把手是否错位？
11. **mDNS**：首次开启 LAN 同步是否弹本地网络权限？拒绝后是否优雅降级？
12. **设置窗**：13" 屏上 820×780 是否超出工作区？
13. **唤醒**：锁屏/休眠恢复后位置、轮询、通知是否正常？

### B. 只有开启"贴边模式"才验（不阻塞首发）

1. **悬停唤出**：贴边收起 → 鼠标划过把手 → 胶囊是否滑出（非活跃 App 状态）？
2. **贴边几何**：左/右/上三条边、Dock 在左/下/隐藏三种布局下是否严格贴边、
   把手居中、展开 400×680 后仍贴边不出屏？
3. **把手命中**：7px（或改成 10–12pt）在 Retina 上是否稳定可命中？
4. **glide 动画**：8ms 步进的落位是否平滑、有无跳帧/拖影？
5. **跨 Space / 全屏**：切 Space 是否跟随？别人全屏时是否隐藏（预期行为）？

---

## 九、需要拍板的决策

**已定（2026-11-02 拍板）**：

- ✅ macOS **主入口 = 菜单栏**，无 Dock 图标（`LSUIElement`）；Windows 保持托盘。
- ✅ macOS **关窗 = 隐藏**，⌘Q 才退出；`close_to_tray` 在 Mac 上不生效。
- ✅ macOS **二级窗口用原生标题栏**（Overlay + 红绿灯），不自绘 resize。
- ✅ macOS **贴边模式默认关闭**，作为可选增强；开启前先做 hover 原生化。
- ✅ **共享核心 + 平台外壳**：用 `PlatformProfile` 暴露能力差异，前端不写
  `if os === "macos"` 散点逻辑。
- ✅ **菜单栏形态一步到位做 popover 小面板**（不做"先菜单后面板"的过渡）。
- ✅ **补浅色模式**：新增一套浅色令牌，跟随系统外观切换。
- ✅ **字体保持 MiSans**，macOS 不切 SF Pro（等宽栈仍补 `SF Mono`/`Menlo` 兜底）。
- ✅ **分发：未签名 DMG + README 说明**（Gatekeeper 绕过指引），公证留待以后。
- ✅ **最低系统 macOS 13.1**（`color-mix()` 的硬性下限，零 CSS 回退）。
- ✅ **先出原型再实现**：外壳原型见
  `docs/mockups/2026-11-02-macos-shell.html`（dev server 预览）。

**遗留小决策**（可在实现中按默认值走，不必阻塞）：

1. **"降低透明度"回退色**：默认回退到 `--tum-glass`（深/浅各一档），原型已示意。
2. **菜单栏百分比直显**：默认开启（模板图标 + `set_title("42%")`），设置里可关。
3. **贴顶**：贴边模式开启时，Top 边实际贴在菜单栏下方；保留该边但文档注明语义。

---

## 附：关键证据索引

- 透明窗依赖私有 API：`node_modules/@tauri-apps/cli/config.schema.json` →
  `WindowConfig.transparent` 的 platform note；`tauri-build-2.7.4/src/manifest.rs`
  （`check` + `AppConfig::features()`）负责让 Cargo features 与配置保持一致。
- `skip_taskbar` 在 macOS 空实现：`tauri-runtime-wry-2.12.0/src/lib.rs:1204-1207`。
- macOS `work_area` = `NSScreen.visibleFrame`：
  `tauri-runtime-wry-2.12.0/src/monitor/macos.rs:8-34`。
- 无边框 resize 不含 macOS：`tauri-runtime-wry-2.12.0/src/undecorated_resizing.rs` 顶部 `#![cfg(any(windows, ...))]`。
- 托盘 `set_title` / `icon_as_template` / `rect`：`tauri-2.12.5/src/tray/mod.rs:538,561,620`。
- `set_activation_policy`（Accessory 备选）：`tauri-2.12.5/src/app.rs:658`。
- `acceptFirstMouse` / `focusable` / `visibleOnAllWorkspaces` builder：
  `tauri-2.12.5/src/webview/webview_window.rs:586,622,927,962`。
- `backgroundThrottling` 仅 macOS 14+ 生效（Windows/Linux 不支持）：
  `config.schema.json` → `WindowConfig.backgroundThrottling` 的 platform note。
- keyring 无 `apple-native` 即无 macOS store：`keyring-3.6.3/src/lib.rs:279-281`。
- 平台配置文件：`tauri-utils-2.10.1/src/config/parse.rs:54`（`tauri.macos.conf.json`）。
