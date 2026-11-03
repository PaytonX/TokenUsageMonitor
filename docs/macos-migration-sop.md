# macOS 迁移 SOP（从 Windows 换到 Mac 逐步操作）

> 面向不熟悉 macOS 的操作者。按顺序执行，每一步都有「做什么 / 命令 / 预期结果 /
> 失败怎么办」。全程约 **1.5–2 小时**（其中首次 Rust 编译 5–15 分钟，可离开）。
>
> 配套文档：`docs/macos-adaptation-research.md`（背景与取舍）、
> PR [#1](https://github.com/PaytonX/TokenUsageMonitor/pull/1)（本次全部改动）。

---

## 0. 全景图

| 阶段 | 在哪台机器 | 时间 | 产出 |
|---|---|---|---|
| A. 出发前：备份 + 导出数据 | Windows | 15 min | U 盘/网盘里一份 `tum-migrate/` |
| B. Mac 基础操作速览 | Mac | 5 min | 会开终端、会进隐藏目录 |
| C. 装工具链（Xcode CLT / Homebrew / Rust / Node） | Mac | 20–30 min | `node -v`、`cargo -V` 有输出 |
| D. 拉代码跑起来 | Mac | 15 min（+首次编译） | 菜单栏出现图标 |
| E. 迁用户数据（账本/配置/凭据/工具日志） | 两台 | 15 min | 历史用量还在 |
| F. 真机验收清单 | Mac | 15 min | 全部打勾 |
| G. 排错（按需） | Mac | — | — |

---

## A. 出发前（在 Windows 上做）

> **先退出 TokenUsageMonitor**（托盘右键 → 退出）。SQLite 带 WAL，运行中复制会拿到
> 不一致的快照。

### A1. 建一个迁移目录

在 D 盘或桌面建 `tum-migrate\`，下面会有 `appdata\` 和 `tool-logs\`。

### A2. 复制应用数据目录

```powershell
# PowerShell
$src = "$env:APPDATA\com.tokenmonitor.app"
$dst = "D:\tum-migrate\appdata"
New-Item -ItemType Directory -Force $dst | Out-Null
Copy-Item "$src\*" $dst -Recurse
Get-ChildItem $dst
```

预期看到（至少）：

- `config.toml` —— 设置（账户列表、轮询间隔……）
- `token_usage_monitor.db` —— 用量账本（历史数据都在这）
- 可能还有 `token_usage_monitor.db-wal` / `-shm`、`.window-state.json`

> ⚠️ **有 `-wal`/`-shm` 就一起拷**（上面命令已含），否则可能丢最后一小段数据。

### A3. 复制本地工具日志（可选，但历史工具用量靠它）

复制这些目录（存在哪个拷哪个）到 `D:\tum-migrate\tool-logs\`：

| 工具 | Windows 路径 |
|---|---|
| Claude Code | `%USERPROFILE%\.claude` |
| Codex | `%USERPROFILE%\.codex` |
| ZCode | `%USERPROFILE%\.zcode` |
| MiniMax | `%USERPROFILE%\.minimax` |
| OpenCode | `%USERPROFILE%\.local\share\opencode` |
| Hermes / DSH | `%USERPROFILE%\.hermes`、`%USERPROFILE%\.dsh` |
| Cherry Studio | `%APPDATA%\CherryStudio\Data`（只拷 `cherrystudio.sqlite` 即可） |

> 日志里的路径字段是 Windows 格式，不影响统计——扫描器只读 token 计数字段。

### A4. 记下三条信息

1. **你配置了几个账户、分别是什么**（DeepSeek/MiniMax/…）——到 Mac 上核对用；
2. **API Key**（Keychain 不可迁移，到 Mac 要重录；**不要**写进任何文件，记在密码管理器里）；
3. **代理端口**（如果设置里填了 `127.0.0.1:xxxx` 代理，确认 Mac 上代理软件用同一端口）。

### A5. 确认代码已推送

```powershell
cd D:\Lab\AI_Agents\TokenUsageMonitor
git log --oneline -6
git status --short   # 应只剩并行会话的 WIP；没有你本次的工作
```

预期 HEAD 包含 `feat(shell)` / `feat(platform)` / `chore(ci,docs)` 等提交，
且 `git branch -vv` 里 `feat/macos-shell` 跟踪 `origin/feat/macos-shell`。

---

## B. Mac 基础操作速览（5 分钟）

不熟 Mac 的话先记住这 6 件事：

| 要做什么 | 怎么做 |
|---|---|
| 打开终端 | `启动台` 搜 **终端**（Terminal），或 `访达 → 应用程序 → 实用工具 → 终端` |
| 进入某个文件夹（图形界面） | 访达里按 **⇧⌘G**，粘贴路径（如 `~/Library/Application Support`） |
| 显示隐藏文件（如 `~/.claude`） | 访达里按 **⇧⌘.**（句号）切换 |
| `~` 是什么 | 你的用户主目录，即 `/Users/你的用户名` |
| `⌘` 是什么 | Command 键（空格键两侧）；本文快捷键里的 ⌘ = Command |
| 粘贴到终端 | **⌘V**；终端里选中即复制，**⌘V** 粘贴 |

补充两条 Mac 特有的行为，后面会用到：

- **关窗口 ≠ 退出程序**：⌘W 只关窗口，程序还在；退出用 **⌘Q**。
  TokenUsageMonitor 就是这么设计的（关窗隐藏到菜单栏）。
- **Gatekeeper**：网上下载的未签名程序第一次打开会被拦，需要「系统设置 →
  隐私与安全性 → 仍要打开」。本次是源码构建，开发阶段遇不到；以后装 DMG 会用到。

---

## C. 装工具链（Mac 上做）

> 以下命令都在**终端**里执行。每装完一步按提示验证。

### C1. Xcode Command Line Tools（编译必需）

```bash
xcode-select --install
```

预期：弹出系统对话框 → 点「安装」→ 同意协议 → 等 5–10 分钟。
验证：

```bash
xcode-select -p
# 预期输出类似 /Library/Developer/CommandLineTools
```

### C2. Homebrew（Mac 的包管理器）

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

安装结束会提示把 brew 加进 PATH。**Apple Silicon（M1/M2/M3/M4）** 执行它打印的两行：

```bash
echo 'eval "$(/opt/homebrew/bin/brew shellenv)"' >> ~/.zprofile
eval "$(/opt/homebrew/bin/brew shellenv)"
```

验证：`brew -v` 有版本输出。

### C3. Rust

```bash
brew install rustup-init
rustup-init -y
```

> 如果 `rustup-init` 不可用，改走官方脚本：
> `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`，一路回车默认。

让当前终端认识 cargo：

```bash
source "$HOME/.cargo/env"
cargo -V    # 预期 cargo 1.8x.x
```

> 不需要额外装 x86_64 目标——**只有要打 universal 安装包时**才需要
> `rustup target add x86_64-apple-darwin`，见 C5。

### C4. Node.js 与 Git

```bash
brew install node
node -v    # 预期 v20 及以上；低于 20 请 brew install node@20 并按其提示 link
git --version
```

> Git 是 Xcode CLT 自带的，通常无需另装。

### C5.（可选）打包用的 universal 目标

仅当你要在本机执行 `npm run tauri:build:mac` 出 DMG 时需要：

```bash
rustup target add x86_64-apple-darwin
```

---

## D. 拉代码并跑起来

### D1. 克隆 + 切分支

```bash
mkdir -p ~/dev && cd ~/dev
git clone https://github.com/PaytonX/TokenUsageMonitor.git
cd TokenUsageMonitor
git checkout feat/macos-shell
git log --oneline -6   # 能看到 feat(shell)/feat(platform) 等提交
```

### D2. 装前端依赖

```bash
npm ci
```

预期：2–4 分钟，无红色 `ERR`。若提示 node 版本不符，回到 C4。

### D3. 首次运行（关键一步）

```bash
npm run tauri:dev:mac
```

> ⚠️ **不要**用 `npm run tauri:dev` 或裸 `cargo`——仓库的 `rust-toolchain.toml`
> 钉的是 Windows 工具链，在 Mac 上会报错。`:mac` 后缀的脚本已经帮你覆盖了。

预期过程：

1. Vite 起在 127.0.0.1:5173；
2. 首次编译 Rust（**5–15 分钟**，屏幕不动是正常的，别中断）；
3. 编译完：**屏幕右上角菜单栏**出现一个圆环图标 + `NN%`；
4. 同时屏幕左上（约 100,100）出现悬浮主面板（400×680）；
5. Dock 里**没有**图标（这是 macOS 设计：菜单栏常驻应用）。

### D4. 第一次交互自检

- 点菜单栏圆环图标 → 弹出面板（现在还没配账户，显示「暂无额度数据」）；
- 再点一下收起；按 `Esc` 也能收；
- 按 **⌘Q** 退出（或菜单栏图标右键 → 退出）。

到此为止，"能跑"已经成立。下一步迁数据。

---

## E. 迁移用户数据

> 把 A 阶段准备好的 `tum-migrate/` 拷到 Mac（U 盘 / 网盘 / 局域网共享均可）。
> 下面假设它放在 `~/Downloads/tum-migrate`，请按实际路径替换。

### E1. 先退出应用（⌘Q）

### E2. 复制账本与配置到 macOS 的应用数据目录

```bash
mkdir -p "$HOME/Library/Application Support/com.tokenmonitor.app"
cp -R ~/Downloads/tum-migrate/appdata/* "$HOME/Library/Application Support/com.tokenmonitor.app/"
ls -la "$HOME/Library/Application Support/com.tokenmonitor.app"
```

预期看到 `config.toml`、`token_usage_monitor.db`（以及可能的 `-wal`/`-shm`）。

> 📁 图形界面查看：访达按 **⇧⌘G**，粘贴
> `~/Library/Application Support/com.tokenmonitor.app`（`~/Library` 默认隐藏，
> 所以必须用「前往文件夹」）。

### E3. 清理配置里的 Windows 残留（必做）

用文本编辑打开 `config.toml`（或终端 `open -e "…/config.toml"`），处理三处：

```toml
# 1) 清空 Windows 绝对路径（D:\Lab\.agentdata 之类在 Mac 上无效）
tool_data_roots = []

# 2) 删除这两类行（如果存在）：
#    - dashboard_x = …
#    - dashboard_y = …
#    - dock = { … }        ← 贴边锚点是 Windows 物理坐标

# 3) 保留其余内容，尤其是 accounts 里的每个账户（instance_id 是账本外键！）
#    不要删除或重建账户——改了 instance_id，旧账本行会在启动时被当作孤儿清掉。
```

### E4. 还原本地工具日志（可选）

```bash
# 逐个复制，存在哪个拷哪个（示例：Claude Code 与 Codex）
cp -R ~/Downloads/tum-migrate/tool-logs/.claude   ~/.claude
cp -R ~/Downloads/tum-migrate/tool-logs/.codex    ~/.codex
cp -R ~/Downloads/tum-migrate/tool-logs/.zcode    ~/.zcode
cp -R ~/Downloads/tum-migrate/tool-logs/.minimax  ~/.minimax
mkdir -p ~/.local/share && cp -R ~/Downloads/tum-migrate/tool-logs/opencode ~/.local/share/opencode
```

Cherry Studio 的库放到：

```bash
mkdir -p "$HOME/Library/Application Support/CherryStudio/Data"
cp ~/Downloads/tum-migrate/tool-logs/cherrystudio.sqlite \
   "$HOME/Library/Application Support/CherryStudio/Data/"
```

### E5. 重录 API Key（Keychain 不可迁移）

1. 菜单栏图标 →「设置…」（或 ⌘,）；
2. 找到每个账户 → 点开 → 填 API Key → 保存；
3. 首次保存会弹 **钥匙串（Keychain）授权**：
   > “TokenUsageMonitor 想要使用您存储在钥匙串中的机密信息”
   → 输入 **Mac 登录密码** → 点「始终允许」。

> - 开发构建（`tauri dev`）每次重新编译签名会变，钥匙串可能反复询问——这是开发期
>   现象，正式安装包不会。
> - 若提示「找不到钥匙串」，检查 `~/Library/Keychains/login.keychain-db` 存在；
>   正常 Mac 都会有。
> - **不要在设置里「删除账户再重新添加」**——如果要改 Key，就在现有账户行里改。
>   instance_id 变了，旧账本会被启动自愈当作孤儿行删除。

### E6. 重新开启自启（只在打包后的 App 里开）

`tauri dev` 里不要开「开机自启」——它会把开发二进制注册进登录项。
等装正式 DMG 后再开；开启后可在 `系统设置 → 通用 → 登录项` 看到。

---

## F. 真机验收清单

> 启动方式：`npm run tauri:dev:mac`（或已装 DMG 直接打开）。
> 逐项打勾；任何一项不符，把现象记到 PR #1 的评论里。

### F1. 外壳与交互

- [ ] 菜单栏有圆环图标，右侧显示 `NN%`（有数据后）；Dock / ⌘Tab 里**没有**本应用
- [ ] 点菜单栏图标弹出面板；再点 / 点外面 / Esc 收起
- [ ] 面板里：环的百分比 = 最紧张窗口；账户行显示「窗口类型 + 百分比 + 重置天数」
- [ ] 面板底部「打开主面板」能打开悬浮面板；「设置…」能开设置窗
- [ ] 悬浮面板：按住标题栏/页脚拖动（`data-tauri-drag-region`）；首次点击无需点两下（`acceptFirstMouse`）
- [ ] 设置窗：左上角有**红绿灯**，可拖边改大小，⌘W 只隐藏不退出
- [ ] 任意窗口按 ⌘W → 隐藏；应用仍在菜单栏；**⌘Q 才退出**
- [ ] 切到别的桌面（三指滑 / Control+←）→ 悬浮面板跟随（跨 Space）
- [ ] 启动时不抢前台焦点（切到别的 App 再启动一次观察）

### F2. 数据与凭据

- [ ] 工具页 / 模型页有历史数据（E4 拷了日志的话）
- [ ] 账户卡片显示 NotConfigured → 重录 Key 后变正常数值
- [ ] 重启应用后 Keychain 不再询问（同一构建内）
- [ ] 用量账本历史还在（趋势/日历能看到迁过来的日期）

### F3. 系统集成

- [ ] 设置里语言切换后，菜单栏菜单与通知文案跟着变
- [ ] 开启阈值通知 → `系统设置 → 通知` 里出现 TokenUsageMonitor（首次通知后）
- [ ] 开启 LAN 同步 → 弹「允许访问本地网络」→ 允许后设备页出现对端
- [ ] 浅色外观（系统设置 → 外观 → 浅色）：面板/设置窗变浅色，文字可读
- [ ] 锁屏再唤醒：图标与面板正常，无残留黑块

### F4. 已知现象（不是 bug）

- 面板圆角外看到的是桌面（正常，透明窗）
- 悬浮面板默认**不**贴边——贴边胶囊是 macOS 的可选模式，默认关闭
- 弹通知在 `tauri dev` 下可能不显示（无正式 bundle id），装 DMG 后正常

---

## G. 常见问题排查

| 现象 | 原因 | 处理 |
|---|---|---|
| `error: toolchain 'stable-x86_64-pc-windows-gnu' is not installed` | 裸跑了 `cargo`/`tauri:dev` | 用 `npm run tauri:dev:mac`；或命令前加 `RUSTUP_TOOLCHAIN=stable` |
| `cargo: command not found` | 新终端没加载 cargo env | `source "$HOME/.cargo/env"`，或重开终端 |
| `xcrun: error: invalid active developer path` | CLT 没装好 | 重跑 `xcode-select --install`；`sudo xcode-select -s /Library/Developer/CommandLineTools` |
| `npm ci` 报 node 版本 | brew 装的是旧 node | `brew install node`（或 `node@20` 并按提示 link），`node -v` ≥ 20 |
| 127.0.0.1:5173 被占用 | 上一个 dev 没退 | `lsof -ti:5173 | xargs kill -9`（或重启终端） |
| 菜单栏没有图标 | 应用没启动 / 编译失败 | 看终端报错；`cargo check` 先过一遍 |
| 点图标不出面板 | 首次建窗失败 | 看终端 `tum.window` 日志；退出重开 |
| 钥匙串每次都问 | dev 构建签名变化 | 正式包不复现；点「始终允许」 |
| 通知不出现 | dev 无 bundle / 权限未给 | 装 DMG 再试；检查 `系统设置 → 通知` |
| 工具页全空 | 日志没拷 / 路径不对 | 确认 `~/.claude/projects` 等存在；`ls ~/.claude` |
| 悬浮面板位置诡异 | 配置里带了 Windows 坐标 | E3 清 `dashboard_x/y`、`dock` |
| 想彻底重置 | — | 先 ⌘Q，再 `rm -rf "$HOME/Library/Application Support/com.tokenmonitor.app"`（**删除全部本地数据，谨慎**） |

---

## H. 双机开发约定（Windows + Mac 并行）

- 代码同步走 GitHub：小步提交、只 `git add` 显式路径（仓库里有 `.mimosa/`
  等不该提交的目录，严禁 `git add -A`）；
- 提交前跑三条：`npm test`、`npx svelte-check`、
  `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc cargo test`（Windows）/
  `cargo test`（Mac）；
- PR #1 的 CI 已包含 **rust-macos** job——Mac 专属代码在 GitHub 上也能编译，
  不必等本机；
- 每个分支只做一件事；合并前在 PR 里贴 F 段验收结果。

---

## 附录：路径与命令速查

| 用途 | 路径 / 命令 |
|---|---|
| 应用数据 | `~/Library/Application Support/com.tokenmonitor.app` |
| 打开该目录 | 访达 ⇧⌘G 粘贴上面路径 |
| 设置文件 | 上面目录下 `config.toml` |
| 账本 | 上面目录下 `token_usage_monitor.db` |
| 钥匙串 | `应用程序 → 实用工具 → 钥匙串访问`，搜索 `TokenUsageMonitor` |
| 登录项 | `系统设置 → 通用 → 登录项` |
| 通知权限 | `系统设置 → 通知 → TokenUsageMonitor` |
| 开发运行 | `cd ~/dev/TokenUsageMonitor && npm run tauri:dev:mac` |
| 退出应用 | ⌘Q |
| 打包 DMG（可选） | `npm run tauri:build:mac`，产物在 `src-tauri/target/universal-apple-darwin/release/bundle/dmg/` |
