# AI 交接手册：macOS 端接续开发与真机验收

> **读者是 AI 代理，不是人。** 人类已把本仓库（分支 `feat/macos-shell`）复制到 Mac。
> 你的任务：在这台 Mac 上把应用跑起来、完成数据迁移、执行真机验收、修复发现的问题，
> 最后产出一份可复核的报告。人类不懂 macOS，所有终端操作由你完成；需要人类操作的
> 部分（输密码、点钥匙串「允许」、肉眼确认视觉项）明确列在 §7。
>
> 配套阅读：`docs/macos-adaptation-research.md`（为什么这么设计）、
> `docs/macos-migration-sop.md`（给人看的版本，命令可参考但**以本文为准**）。

---

## 0. 一句话任务

```text
在 macOS 13.1+ 上：装工具链 → 跑 npm run tauri:dev:mac → 迁数据 → 按 §7 验收 →
修 bug（如有）→ 写 docs/macos-verify-report.md → 汇报给人类决定是否合并 PR #1。
```

---

## 1. 硬约束（先读，违反即失败）

### 1.1 绝对不要做

- ❌ `git push --force`、改写历史、直接推 `main`、合并 PR #1；
- ❌ `git add -A` / `git add .`——仓库里有 `.mimosa/`、`target/`、`node_modules/`、
  以及**人类在 Windows 侧未提交的并行 WIP**，卷进去就是事故；
- ❌ 删除 `~/Library/Application Support/com.tokenmonitor.app` 里的数据（除非人类
  明确要求重置，且先备份）；
- ❌ 把 API Key / 密码写进任何文件、命令历史、commit；
- ❌ 修改 Windows 侧行为（见 §2.4 的代码地图：Windows 路径必须保持原状）；
- ❌ 用 `sudo` 跑 npm/cargo；`brew` 可能要求系统密码，那是正常的；
- ❌ 在 `tauri dev` 里开启「开机自启」（会把开发二进制注册进登录项）；
- ❌ 声称"项目安全/无漏洞"——安全扫描结论不完整是已知噪音，不要据此下结论。

### 1.2 必须先问人类

- 需要**系统密码**（Xcode CLT 安装、brew 的 sudo、钥匙串授权）时；
- 需要**录入 API Key** 时（由人类在 UI 里输入，你不要经手）；
- 需要 `gh auth login` 才能 push 时（浏览器授权由人类完成）；
- 发现需要改变**产品行为**（不只是修 bug）时——先写方案，不要直接改。

### 1.3 环境前提

- macOS **13.1 或更新**（`sw_vers -productVersion` 验证；浅色令牌依赖
  `color-mix()`，13.1 是硬下限）；
- 有可用的图形界面会话（不是纯 SSH）：`launchctl managername` 应为 `Aqua`；
- 磁盘 ≥ 10 GB 空闲（Rust 编译产物很大）。

---

## 2. 项目快照（你接手时的状态）

### 2.1 代码

| 项 | 值 |
|---|---|
| 仓库 | `PaytonX/TokenUsageMonitor`（公开） |
| 分支 | `feat/macos-shell` |
| 交接时 HEAD | 以 bundle 内实际 tip 为准；校验 `git log -1 --format=%s` 应为 `docs(macos): AI 交接手册…`（本文件随该提交入库） |
| PR | [#1](https://github.com/PaytonX/TokenUsageMonitor/pull/1)（draft，勿合并） |
| 设计原则 | 共享核心 + 两套平台外壳：Windows 保持胶囊/托盘；macOS 菜单栏常驻 |

### 2.2 已完成（不要重做）

- **P0**：keyring `apple-native`（Keychain）、`macOSPrivateApi` 透明窗、
  `tauri.windows.conf.json` / `tauri.macos.conf.json` 拆分、`macos/Info.plist`
  （LSUIElement + 本地网络说明）、跨平台 `open_url`、`:mac` npm 脚本；
- **P1 外壳**：`platform.rs` 画像、菜单栏模板图标 + 百分比直显、popover 面板、
  macOS 应用菜单（⌘, / ⌘W / ⌘Q）、关窗恒隐藏、二级窗口原生标题栏
  （Overlay 红绿灯）、启动不抢焦点、跨 Space、macOS 作用域浅色令牌、
  popover 页面与能力清单；
- **CI**：`rust-macos` job 已在 GitHub macOS runner 上编译 + 387 测试通过；
- **文档**：调研报告、交互原型、迁移 SOP。

### 2.3 已验证 / 未验证

| 已由 CI/本机验证 | 只能在你这台真机验证 |
|---|---|
| Windows：cargo test 394+21、vitest 239、svelte-check 0/0 | 菜单栏图标/百分比的视觉与位置 |
| 分支树（worktree）：cargo check、svelte-check、vitest 236 | popover 定位（`TrayIcon::rect` 坐标翻转、刘海屏） |
| macOS runner：全部 Rust 测试 387 通过（含 3 个 platform 单测） | WKWebView 下的 hover / 首次点击 / 拖拽手感 |
| 前端逻辑单测（popover 数据聚合等） | 浅色模式观感与组件级暗色残留 |
| | 钥匙串授权次数、通知是否出现、VPN/本地网络权限 |

### 2.4 代码地图（改 macOS 行为只动这些）

| 文件 | 职责 |
|---|---|
| `src-tauri/src/platform.rs` | `PlatformProfile`：主入口/关闭语义/菜单栏能力/窗口装饰。**所有平台分支的单一事实来源** |
| `src-tauri/src/lib.rs` | `setup` 里的激活策略、托盘/菜单构建、全局菜单事件、关闭拦截、`update_tray_title`、`build_app_menu` |
| `src-tauri/src/ipc.rs` | `build_popover_window` / `position_popover` / `toggle_popover_panel_sync`、`show_dashboard_window` / `quit_app`、三个二级窗口的 macOS 标题栏分支 |
| `src-tauri/src/scheduler.rs` | 轮询入库后调用 `update_tray_title`（写锁已先释放） |
| `src-tauri/src/i18n.rs` | 托盘菜单文案（`tray_show` / `tray_settings` / `tray_quit`） |
| `popover.html` + `src/popover/main.ts` | 菜单栏面板页（纯 TS 拼 DOM；浏览器直开时 dev-mock 供数据） |
| `src/lib/platform.ts` | 把 `window.__TUM_PLATFORM__` 写进 `<html data-platform>` |
| `src/styles/tokens.css` | 文件尾部「macOS 外壳分支」：浅色档 + 二级窗口避让 40px 红绿灯 |
| `src-tauri/tauri.macos.conf.json` + `src-tauri/macos/Info.plist` | 打包配置（最低 13.1、dmg、LSUIElement） |
| `src-tauri/capabilities/popover.json` | popover 窗能力（仅事件 + 自定义命令） |
| `docs/mockups/2026-11-02-macos-shell.html` | 已拍板的视觉契约（改视觉前先对照它） |

### 2.5 Windows 行为不可退化

Windows 侧的菜单/关闭语义/无边框窗口/胶囊贴边**均为有意保留**。
新增分支只用 `cfg(target_os = "macos")` 或 `platform::profile()`；
改完后必须跑 §8 的检查（Windows 侧不在本机，靠 CI 的 MSVC job 兜底）。

---

## 3. 校验交付物（bundle）

人类复制过来的目录里应有：

```text
token-usage-monitor-macos.bundle
SHA256SUMS.txt
```

执行：

```bash
cd <复制过来的目录>
shasum -a 256 -c SHA256SUMS.txt          # 期望 OK
git bundle verify token-usage-monitor-macos.bundle
mkdir -p ~/dev && cd ~/dev
git clone <复制过来的目录>/token-usage-monitor-macos.bundle TokenUsageMonitor
cd TokenUsageMonitor
git log -1 --format=%s                  # 应以 docs(macos): AI 交接手册 开头（见 §2.1）
test -f docs/AI-HANDOFF-macos.md && echo "handoff doc OK"
```

若人类是直接从 GitHub clone 的：`git checkout feat/macos-shell` 后同样校验 HEAD。

---

## 4. 工具链（幂等：先检查，缺了才装）

逐步执行，每步都有验证；失败按 §9 处理。

### 4.1 系统与架构

```bash
sw_vers -productVersion        # ≥ 13.1
uname -m                       # arm64 或 x86_64
df -h / | tail -1              # 可用空间 ≥ 10G
launchctl managername          # 必须是 Aqua（有图形会话）
```

### 4.2 Xcode Command Line Tools

```bash
xcode-select -p || xcode-select --install
```

`--install` 会弹 GUI 对话框 → **请人类点「安装」并同意协议**（约 5–10 分钟），
然后重新验证 `xcode-select -p`。

### 4.3 Homebrew

```bash
command -v brew || /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

装完按提示把 brew 加进 PATH（Apple Silicon 是 `/opt/homebrew`），
`brew -v` 通过即可。安装过程可能要求系统密码 → 人类输入。

### 4.4 Rust

```bash
command -v cargo && cargo -V || { brew install rustup-init && rustup-init -y && source "$HOME/.cargo/env"; }
cargo -V        # 验证
```

> 不要改 `rust-toolchain.toml`（Windows 贡献者的钉子）。macOS 侧靠
> `npm run tauri:dev:mac` 里的 `RUSTUP_TOOLCHAIN=stable` 覆盖。

### 4.5 Node 与 sqlite3

```bash
command -v node && node -v || brew install node
node -v         # ≥ v20
sqlite3 -version   # macOS 自带，用于 §6 数据校验
```

### 4.6（可选，仅打包时）universal 目标

```bash
rustup target add x86_64-apple-darwin
```

---

## 5. 跑起来（非交互式）

```bash
cd ~/dev/TokenUsageMonitor
npm ci
```

首次 `npm ci` 约 2–4 分钟。然后：

```bash
cd ~/dev/TokenUsageMonitor
( npm run tauri:dev:mac > /tmp/tum-dev.log 2>&1 & echo $! > /tmp/tum-dev.pid )
```

**首次编译 Rust 5–20 分钟**，按下面方式轮询（不要用固定 sleep 一刀切）：

```bash
# 每 30s 检查一次，最多 25 分钟：
pgrep -fl "target/debug/token-usage-monitor" && echo APP_UP
curl -sf -o /dev/null http://127.0.0.1:5173/ && echo VITE_UP
tail -5 /tmp/tum-dev.log
```

判定"启动成功"的充分条件（三者都要）：

1. `pgrep` 能查到 `target/debug/token-usage-monitor` 进程；
2. Vite 5173 可访问；
3. 日志无 `error`/`panicked`（`grep -iE "error|panic" /tmp/tum-dev.log` 为空）。

> GUI 窗口属于当前登录会话：本步骤必须在**有桌面**的终端里跑。若
> `launchctl managername` 不是 `Aqua`，停下来告诉人类改用桌面终端。

停止：`kill "$(cat /tmp/tum-dev.pid)"`（或 `pkill -f "tauri dev"`；**先确认 pid**）。

---

## 6. 数据迁移（人类已把数据拷到某个目录）

### 6.1 找到迁移目录

人类应提供一个目录（U 盘/下载），里面形如：

```text
appdata/config.toml
appdata/token_usage_monitor.db        （可能有 -wal / -shm）
tool-logs/.claude/  tool-logs/.codex/  ...
```

**路径必须由人类给出**（用 `ls` 确认存在），不要猜 `/Volumes/xxx`。

### 6.2 应用已退出（重要）

`pgrep -fl token-usage-monitor` 必须为空（先 kill 掉 §5 的 dev 进程，或让人类 ⌘Q）。

### 6.3 复制账本与配置

```bash
APP="$HOME/Library/Application Support/com.tokenmonitor.app"
mkdir -p "$APP"
cp -R <迁移目录>/appdata/* "$APP"/
ls -la "$APP"
```

### 6.4 清理 Windows 残留（必做，否则工具页为空、位置错乱）

```bash
# 1) tool_data_roots 置空
/usr/bin/sed -i '' -E 's/^tool_data_roots *=.*/tool_data_roots = []/' "$APP/config.toml"
# 2) 删掉 Windows 物理坐标/贴边锚点（行存在才删）
/usr/bin/sed -i '' -E '/^dashboard_[xy] *=/d; /^dock *=/d' "$APP/config.toml"
# 3) 人工核对（这一步不要自动化跳过）
grep -nE "tool_data_roots|dashboard_[xy]|dock|accounts" "$APP/config.toml"
```

要求：`tool_data_roots = []`；无 `dashboard_x/y`、无 `dock`；`accounts` 条目**保留**
（`instance_id` 是账本外键，删了旧数据会在启动时被清）。

### 6.5 账本完整性校验（sqlite3）

```bash
DB="$HOME/Library/Application Support/com.tokenmonitor.app/token_usage_monitor.db"
sqlite3 "$DB" ".tables"
sqlite3 "$DB" "select count(*) from usage_daily;"
sqlite3 "$DB" "select min(date), max(date) from usage_daily;"
```

> 若列名/表名与上面不一致，先 `.schema usage_daily` 看真实结构再查——**不要改库**。
> 预期：行数 > 0，日期覆盖你在 Windows 上使用的区间。

### 6.6 工具日志（人类说"需要"才做）

```bash
mkdir -p ~/.local/share
cp -R <迁移目录>/tool-logs/.claude  ~/.claude 2>/dev/null || true
cp -R <迁移目录>/tool-logs/.codex   ~/.codex  2>/dev/null || true
cp -R <迁移目录>/tool-logs/.zcode   ~/.zcode  2>/dev/null || true
cp -R <迁移目录>/tool-logs/.minimax ~/.minimax 2>/dev/null || true
cp -R <迁移目录>/tool-logs/opencode ~/.local/share/opencode 2>/dev/null || true
test -d ~/.claude/projects && echo CLAUDE_LOGS_OK
```

### 6.7 API Key（人类操作，你只做验证）

1. 请人类：菜单栏图标 →「设置…」→ 逐个账户填 Key → 保存；
2. 人类会看到钥匙串弹窗，要求输入 **Mac 登录密码** 并点「始终允许」；
3. 你验证（不读取密钥内容）：

```bash
security find-generic-password -s TokenUsageMonitor >/dev/null 2>&1 && echo KEYCHAIN_OK || echo KEYCHAIN_MISSING
```

> 若人类误删了账户：不要重建（instance_id 变化会让旧账本成为孤儿）。
> 从 `config.toml` 恢复账户条目后再让人类填 Key。

---

## 7. 验收矩阵（逐项）

> 每项记录 `PASS` / `FAIL` / `BLOCKED` + 证据（命令输出或人类原话）。
> "人眼确认"项：如果你有截图能力就自己核对；否则**明确请人类看一眼并复述结果**，
> 不要替人类假定 PASS。

### 7.1 AI 可独立验证

| # | 项 | 命令 / 判据 |
|---|---|---|
| A1 | 进程在跑 | `pgrep -fl "target/debug/token-usage-monitor"` 非空 |
| A2 | Vite 正常 | `curl -sf http://127.0.0.1:5173/ >/dev/null` 退出码 0 |
| A3 | 日志无错误 | `grep -iE "panicked|ERROR" /tmp/tum-dev.log` 为空 |
| A4 | 账本已迁移 | §6.5 行数 > 0 且日期区间合理 |
| A5 | 配置已清理 | §6.4 grep 结果符合要求 |
| A6 | 钥匙串已写入 | §6.7 `KEYCHAIN_OK` |
| A7 | 无 Dock 图标（间接） | `lsappinfo list` 里该进程的 `ApplicationType` 不含 `Foreground`；或人类确认 |
| A8 | 设置窗标题栏（间接） | 日志中无 `title_bar` 相关报错；窗口尺寸/位置与配置一致 |

### 7.2 需要人类确认（你用问题清单收集）

| # | 项 | 怎么问 |
|---|---|---|
| H1 | 菜单栏图标 + `NN%` 直显 | "右上角菜单栏是否有一个圆环图标，旁边显示百分比？" |
| H2 | 点图标弹面板 / 再点或 Esc 收起 | "点一下图标，面板是否出现在图标下方？点外面能收吗？" |
| H3 | 面板内容 | "面板里是否显示环 + 最紧张窗口 + 账户行（名称/百分比/重置天数）？" |
| H4 | 底部按钮 | "「打开主面板」「设置…」「退出」是否都能用？" |
| H5 | 首次点击 | "面板不是前台时，第一次点击是否能直接生效（不需要点两下）？" |
| H6 | ⌘W / ⌘Q | "⌘W 后应用是否还在菜单栏？⌘Q 才退出？" |
| H7 | 设置窗 | "左上角有红黄绿三个圆点吗？能拖边缘改大小吗？" |
| H8 | 跨 Space | "切到另一个桌面，悬浮面板是否还在？（切换动画后观察）" |
| H9 | 浅色外观 | "系统外观切浅色后，面板和设置窗文字是否清晰？有没有刺眼的深色块？" |
| H10 | popover 定位 | "面板是否对齐图标下方、没有遮住菜单栏、没有跑到屏幕外？" |
| H11 | 通知 | "开阈值提醒后，系统设置→通知里出现 TokenUsageMonitor 了吗？"（dev 下可能没有，属已知） |
| H12 | 本地网络 | "如果开了 LAN 同步，有没有弹「允许访问本地网络」？" |
| H13 | 悬停把手（可选模式） | 贴边模式在 macOS **默认关闭**，不要主动开启；仅当人类要求测试时再按研究文档 §8 B 组执行 |

---

## 8. 修 bug 的循环

### 8.1 定位

- 面板/菜单：`/tmp/tum-dev.log`（Rust tracing，target 如 `tum.window`）；
- 前端：Vite 终端输出 / WebKit 控制台（`npm run tauri:dev:mac` 的 devtools：
  右键页面「检查元素」在 debug 构建可用）；
- 窗口坐标：临时在 Rust 里 `tracing::debug!` 打 `outer_position/rect`，改完删掉。

### 8.2 改完必跑（顺序固定）

```bash
cd ~/dev/TokenUsageMonitor
npm test                       # vitest
npx svelte-check               # 0 error 0 warning
cd src-tauri && cargo test     # 用默认工具链（Mac 上就是 stable）
```

> 若 `cargo test` 因仓库钉的 Windows 工具链报错：在 Mac 上直接 `cargo test` 即可
> （`rust-toolchain.toml` 只影响 Windows 语义；Mac 上 rustup 找不到该宿主工具链时
> 会自动用默认 stable——若报错，加 `RUSTUP_TOOLCHAIN=stable`）。

### 8.3 提交规范（沿用仓库历史风格）

```text
fix(menu): 一句话说明（中文）
feat(popover): ...
test(...): ...
docs(...): ...
```

- 只 `git add <显式路径>`；
- 一个 commit 只做一件事；
- 提交前 `git status --short` 人工核对没有 `.mimosa/`、`target/`、`node_modules/`。

### 8.4 推送（需要人类授权一次）

```bash
gh auth status || gh auth login     # 浏览器授权由人类完成
git push                            # 目标分支 feat/macos-shell，不要推 main
```

推送后等 CI（`gh run list --branch feat/macos-shell`），三个 job 全绿再继续。
**不要**合并 PR；把结果报告给人类。

### 8.5 视觉改动的前置

任何**视觉**变更（颜色、间距、材质、体积）必须：
1. 先对照 `docs/mockups/2026-11-02-macos-shell.html`（已拍板契约）；
2. 需要偏离契约时，先做 HTML 原型给人类确认，再动代码。

---

## 9. 故障树（按出现顺序）

| 现象 | 判据 | 处理 |
|---|---|---|
| `error: toolchain 'stable-x86_64-pc-windows-gnu' is not installed` | 裸跑 cargo/tauri:dev | 用 `npm run tauri:dev:mac`；或 `RUSTUP_TOOLCHAIN=stable <cmd>` |
| `The tauri dependency features ... add the macos-private-api feature` | Cargo.toml 与配置不一致 | 不要手改 features；用 `npm run tauri:dev:mac`（CLI 会校验/同步） |
| `xcrun: error: invalid active developer path` | CLT 未装/路径失效 | `xcode-select --install`；必要时 `sudo xcode-select -s /Library/Developer/CommandLineTools` |
| `windres`/链接错误 | 不应该出现（那是 Windows GNU 的问题） | 确认在跑 Mac 工具链：`rustc -vV` 的 host 应为 `aarch64-apple-darwin` |
| 5173 被占用 | `lsof -ti:5173` 有输出 | `lsof -ti:5173 \| xargs kill -9`，重跑 |
| 菜单栏无图标 | 进程在但无图标 | 看日志 `tum.window`；确认 `set_activation_policy(Accessory)` 未 panic；重开会话 |
| popover 跑出屏幕 | 位置异常 | 检查 `position_popover` 的 `TrayIcon::rect` 坐标（见 §2.3 未验证项），必要时按 work_area 再夹一次 |
| 钥匙串每次都问 | dev 重建签名变化 | 正常现象，记录到报告；正式 DMG 复测 |
| 通知不出现 | dev 无 bundle id | 已知限制，记录；`tauri:build:mac` 出的 .app 里再测 |
| 工具页全空 | `~/.claude` 等不存在 | 确认 §6.6 拷贝；`ls ~/.claude/projects` |
| 浅色下发黑/看不清 | 组件硬编码暗色 | 登记为 issue（P3 组件级浅色精修），不要当场大改 |
| 面板四角"露白" | 只在浏览器预览出现 | 正常（透明窗在浏览器里无宿主）；真机看桌面 |

---

## 10. 完成定义（DoD）与汇报格式

### 10.1 产出

1. **`docs/macos-verify-report.md`**（新建）：
   - 环境：`sw_vers`、`uname -m`、`rustc -V`、`node -v`；
   - §7 全表逐项 `PASS/FAIL/BLOCKED` + 证据（命令输出摘要 / 人类原话）；
   - 发现的问题：现象、复现步骤、日志片段、你的修复（commit hash）或"未修+建议"；
   - 遗留项（P2/P3 里不在本次范围的）。
2. **PR #1 评论**：贴报告摘要 + 结论（可合并 / 需人类决策）。
3. **修复提交**（如有）：已 push 到 `feat/macos-shell`，CI 全绿。

### 10.2 不要做的事

- 不要自行合并 PR；
- 不要为了让验收项变绿而修改验收标准；
- 不要删除未通过的检查项——如实记录 BLOCKED。

---

## 附录 A：路径与命令速查

| 用途 | 值 |
|---|---|
| 应用数据 | `~/Library/Application Support/com.tokenmonitor.app` |
| 设置 | 同上 `config.toml` |
| 账本 | 同上 `token_usage_monitor.db` |
| dev 日志 | `/tmp/tum-dev.log` |
| dev 进程 | `target/debug/token-usage-monitor` |
| 开发运行 | `npm run tauri:dev:mac` |
| 打包 DMG | `npm run tauri:build:mac` → `src-tauri/target/universal-apple-darwin/release/bundle/dmg/` |
| 三条检查 | `npm test` / `npx svelte-check` / `cd src-tauri && cargo test` |
| 键串服务名 | `TokenUsageMonitor`（`security find-generic-password -s TokenUsageMonitor`） |

## 附录 B：给人类的一句话（可转发）

> "请把 `tum-handoff/` 目录拷到 Mac，然后在 Mac 上对 AI 说：
> 读 `~/dev/TokenUsageMonitor/docs/AI-HANDOFF-macos.md`，按它完成 macOS 验收。"
