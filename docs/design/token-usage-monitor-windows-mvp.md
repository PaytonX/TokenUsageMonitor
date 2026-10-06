# TokenUsageMonitor · Windows MVP 设计方案

> 目标：在 Windows 11/10 上实现一个轻量、透明、置顶的 Token 用量监看看板，支持 MiniMax Token Plan / DeepSeek API / Volcano AgentPlan 三家 Provider，未来可平滑扩展到更多平台与 Provider。

---

## 1. 当前状态分析

- 工作树为全新项目，仅含 `.gitignore`（已含 `node_modules` / `dist` / `build` / `.next` / `__pycache__` 等条目，与 Node + Rust + Python 混合栈兼容）。
- 同类参考项目已调研：
  - **Win-CodexBar**（Tauri + React）：56 个 Provider，tray-first 弹窗式，**没有透明置顶看板 + 桌宠形态**。
  - **token-usage-widget**（Electron）：corner widget + 浏览器仪表盘，~150MB 安装体偏重。
  - **deepseek-balance-widget**（tkinter + CustomTkinter）：~25MB 内存，单 Provider，UI 简陋。
  - **ai-usagebar**（Rust 原生）：仅 Wayland 之上 widget，Windows 仅 TUI，不满足"透明看板"诉求。
- 结论：**没有现成项目同时满足「透明看板 + 多 Provider + 用量热力图 + 轻量」**，需自建但可大量参考 Win-CodexBar 的 Provider 抽象与凭证管理思路。

## 2. 关键 API 调研结论

| Provider | 端点 | 鉴权 | 返回关键字段 | 热力图数据源 |
|---|---|---|---|---|
| **MiniMax Token Plan** | `GET https://www.minimaxi.com/v1/token_plan/remains` | `Authorization: Bearer <Token Plan Key>` | 套餐内剩余额度、已用、套餐档位、5h 窗口剩余 | API 不提供按天明细 → 本地 SQLite 存每日快照 |
| **DeepSeek API** | `GET https://api.deepseek.com/user/balance` + `GET /v1/billing/usage` | `Authorization: Bearer <API Key>` | `total_balance` / `granted_balance` / `topped_up_balance`，当月已消耗 token 与费用 | billing 接口按月聚合 → 本地每日采样补齐热力图 |
| **Volcano AgentPlan** | `POST https://ark.cn-beijing.volcengineapi.com/?Action=GetAFPUsage&Version=2024-01-01`，明细 `?Action=GetUsageDetails` | Access Key + HMAC-SHA256 签名（`X-Date` / `X-Content-Sha256` / `Authorization`） | `AFPFiveHour` / `AFPDaily` / `AFPWeekly` / `AFPMonthly`，每窗口含 `Quota` / `Used` / `ResetTime` | `GetUsageDetails` 支持 `QueryInterval=Day`，原生提供按天热力图 |

三家 Provider 字段口径差异较大（CNY 余额 vs AFP 积分 vs token 计数），统一抽象为 `WindowUsage { used, quota, unit, reset_at }` 即可承载。

## 3. 技术决策与理由

| 决策点 | 选择 | 理由 |
|---|---|---|
| 应用框架 | **Tauri 2.0** | 安装包 ~10MB，内存 ~40MB；Windows 11 内置 WebView2，Win10 可引导安装；原生支持透明无边框置顶窗口；与「尽可能轻量」契合度最高 |
| 前端栈 | **Svelte 5 + TypeScript + Vite** | 编译后体积小、运行时轻；响应式天然适合"定时数据刷新"场景；frontend-design skill 可直接产出可用 UI |
| 后端语言 | **Rust** | reqwest + tokio + serde 极适合 HTTP 轮询；签名计算（HMAC-SHA256）用 `hmac`/`sha2` crate 干净；sqlite-rs 持久化快照；与 Tauri 原生集成 |
| 凭证存储 | **Windows Credential Manager via `keyring` crate** | 走 DPAPI 加密；与 Win-CodexBar 同等安全等级；避免明文落盘 |
| 配置文件 | `config.toml`（用户级 `%APPDATA%\TokenUsageMonitor\config.toml`） | 仅存非敏感项：启用的 Provider、轮询间隔、看板位置、是否开机自启 |
| 组件形态 | **透明看板 + 可折叠迷你态** | 默认半透明置顶看板，双击折叠为迷你悬浮球（只显示综合进度环），右键菜单切换/设置/退出 |
| 热力图范围 | **MVP 即纳入** | 用户明确点名核心诉求；火山原生支持按天，MiniMax/DeepSeek 走本地每日快照补齐（首次启动可能缺数据，提示"积累中"） |
| 轮询策略 | **tokio 分时调度**：MiniMax/DeepSeek 默认 5 分钟，火山 AFP 5 分钟、GetUsageDetails 30 分钟 | 平衡新鲜度与请求成本；可在 Settings 调整 |

## 4. 项目结构

```
TokenUsageMonitor/
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   └── src/
│       ├── main.rs              # Tauri 入口，注册 commands + 插件
│       ├── lib.rs               # 应用状态、初始化
│       ├── ipc.rs               # #[tauri::command] 暴露给前端
│       ├── scheduler.rs         # tokio 定时轮询 + 事件推送
│       ├── storage.rs           # rusqlite：daily_snapshots 表
│       ├── settings.rs          # config.toml 读写 + keyring 凭证
│       ├── signing.rs           # Volcano HMAC-SHA256 请求签名
│       └── providers/
│           ├── mod.rs           # Provider trait + UsageSnapshot 通用类型
│           ├── minimax.rs       # MiniMax Token Plan 实现
│           ├── deepseek.rs      # DeepSeek balance + billing 实现
│           └── volcengine.rs    # Volcano AgentPlan 实现（含签名）
├── src/                          # Svelte 前端
│   ├── App.svelte
│   ├── routes/
│   │   ├── Dashboard.svelte     # 透明看板主视图
│   │   ├── CompactBall.svelte   # 折叠迷你态
│   │   └── Settings.svelte      # 独立设置窗口
│   ├── lib/
│   │   ├── types.ts             # 与 Rust 对齐的 TS 类型
│   │   ├── api.ts               # invoke() / listen() 封装
│   │   └── components/
│   │       ├── ProviderCard.svelte
│   │       ├── UsageBar.svelte
│   │       ├── ProgressRing.svelte
│   │       ├── ResetCountdown.svelte
│   │       └── HeatmapGrid.svelte
│   └── styles/
│       └── tokens.css           # 设计 token（颜色/间距/字体）
├── package.json
├── vite.config.ts
├── svelte.config.js
└── README.md
```

## 5. 关键模块设计

### 5.1 Provider 抽象层（`src-tauri/src/providers/mod.rs`）

通用 trait 是后期扩展的核心。新增 Provider 只需 `impl Provider` 并在 `lib.rs` 注册：

```rust
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;                  // "minimax" / "deepseek" / "volcengine"
    fn display_name(&self) -> &'static str;        // "MiniMax Token Plan"
    fn auth_kind(&self) -> AuthKind;               // BearerKey | AccessKeySecret
    async fn fetch_usage(&self, creds: &Credentials)
        -> Result<UsageSnapshot, ProviderError>;
}

pub struct UsageSnapshot {
    pub provider_id: String,
    pub timestamp: DateTime<Utc>,
    pub windows: UsageWindows,
    pub heatmap: Option<Vec<HeatmapCell>>,   // 已有按天明细时 Some
}

pub struct UsageWindows {
    pub five_hour: Option<WindowUsage>,
    pub daily: Option<WindowUsage>,
    pub weekly: Option<WindowUsage>,
    pub monthly: Option<WindowUsage>,
    pub balance: Option<BalanceInfo>,         // CNY 余额类（DeepSeek）
}

pub struct WindowUsage {
    pub used: f64,
    pub quota: f64,
    pub unit: UsageUnit,                      // Tokens / AFP / CNY / Credits
    pub reset_at: Option<DateTime<Utc>>,
    pub over_quota: bool,
}

pub struct HeatmapCell {
    pub date: NaiveDate,
    pub value: f64,                            // 用量（按 unit 归一化为百分比更佳）
    pub unit: UsageUnit,
}
```

> MiniMax `token_plan/remains` 仅返回剩余额度，`fetch_usage` 内部会同时写一条 `daily_snapshots` 记录补齐热力图；DeepSeek 同理。Volcano 直接用 `GetUsageDetails` 返回值。

### 5.2 调度器（`scheduler.rs`）

```rust
// 每个启用的 Provider 一个 tokio::task
// 用 interval 调度，emit("usage-updated", snapshot) 推给前端
// 失败时保留上次数据 + emit("provider-error", { id, message })
// 用户右键"立即刷新"→ 调用 force_refresh(id) 立刻跑一次
```

默认间隔：MiniMax/DeepSeek 5 min、火山 AFP 5 min、火山 GetUsageDetails 30 min。可在 Settings 全局调慢（节流模式适合免费层用户）。

### 5.3 凭证管理（`settings.rs`）

- `keyring::Entry::new("TokenUsageMonitor", "minimax")` → DPAPI 加密存储 API Key
- 公共配置（启用列表、轮询间隔、看板位置）写入 `config.toml`
- 首次启动走引导：选择已订阅的 Provider → 录入对应凭证 → 测试连通性（调一次 `fetch_usage`）

### 5.4 火山签名（`signing.rs`）

实现 Volcano V4 签名：
1. 构造 `X-Date`（RFC1123 GMT）、`X-Content-Sha256`（body SHA256 hex）
2. 拼接签名串：`GET\n/\nAction=...&Version=...\n...`
3. HMAC-SHA256（SK, SigningStr）→ hex → 写入 `Authorization: HMAC-SHA256 Credential=AK/...`
4. 通过 `reqwest::RequestBuilder` middleware 自动应用到所有 Volcano 请求

### 5.5 IPC Commands（`ipc.rs`）

```rust
#[tauri::command]
async fn get_usage(state) -> Vec<UsageSnapshot>;          // 全量快照

#[tauri::command]
async fn get_heatmap(state, provider_id, days) -> Vec<HeatmapCell>;

#[tauri::command]
async fn force_refresh(state, provider_id: Option<String>);

#[tauri::command]
async fn open_settings(state);                            // 弹出设置窗口

#[tauri::command]
async fn set_credentials(state, provider_id, creds) -> Result<(), Error>;

#[tauri::command]
async fn test_credentials(state, provider_id, creds) -> Result<UsageSnapshot, Error>;

#[tauri::command]
async fn set_window_mode(state, mode: WindowMode);       // dashboard | compact
```

事件：`usage-updated` / `provider-error` / `reset-tick`（每秒推一次以更新倒计时）。

### 5.6 透明窗口配置（`tauri.conf.json`）

```json
{
  "app": {
    "windows": [{
      "label": "dashboard",
      "title": "TokenUsageMonitor",
      "width": 360,
      "height": 480,
      "decorations": false,
      "transparent": true,
      "alwaysOnTop": true,
      "skipTaskbar": true,
      "resizable": false,
      "shadow": false
    }]
  }
}
```

折叠到 compact 模式时通过 `appWindow.setSize(200, 80)` + 重新渲染 `CompactBall.svelte`。

### 5.7 前端组件（Svelte）

- `Dashboard.svelte`：列布局，每个启用的 Provider 一张 `ProviderCard`；底部 `HeatmapGrid` 缩略（点击展开全屏）；顶部综合进度环 + 当前时间。
- `ProviderCard.svelte`：Provider 名 + 套餐档位徽章；`UsageBar` 显示 5h / 周 / 月用量百分比，颜色 < 70% 绿、70-90% 黄、> 90% 红；`ResetCountdown` 显示下次重置倒计时；MiniMax/DeepSeek 显示 CNY 余额，Volcano 显示 AFP。
- `HeatmapGrid.svelte`：7×N 周日历网格，颜色用 4 级绿色渐变（GitHub 风格）；hover tooltip 显示当日用量。
- `CompactBall.svelte`：圆形进度环，颜色按最低剩余百分比；hover 弹出 mini 列表；双击展开回 Dashboard。
- 设计 token：参考 frontend-design skill 输出，深色背景 `#0f172a` / 卡片 `#1e293b` / 主色青紫渐变；圆角 8px；字体 SF Mono / Segoe UI Variable；半透明 92%。

## 6. 假设与边界

- 假设：用户已自行在三家平台开通对应套餐并拿到 API Key / Access Key。
- 不在 MVP 范围内：
  - 多语言（先做中文）。
  - 移动端 / Web 端（仅 Windows 桌面）。
  - 桌宠形态（列为 P1）。
  - 自定义 Provider 插件市场（只暴露 trait，不提供运行时加载）。
  - 自动开机自启（提供开关，但引导用户手动加 shell:startup 快捷方式以避免权限提示）。
- 网络失败策略：保留上一次数据，卡片右上角显示"上次刷新失败 · 5 min ago"，不阻塞其他 Provider。

## 7. 实施步骤

按依赖顺序执行，每步可独立验证：

1. **项目骨架**
   - `cargo create --bin` + `npm create tauri-app` 选择 Svelte + TS 模板到工作树。
   - 配置 `tauri.conf.json` 透明窗口参数。
   - 写最小 `App.svelte` 显示一行 "TokenUsageMonitor"，启动后应得到半透明置顶悬浮窗。

2. **通用抽象层 + Mock Provider**
   - 写 `providers/mod.rs` 的 trait 与公共类型。
   - 实现 `MockProvider` 返回固定 `UsageSnapshot`。
   - 写 `ipc.rs` 的 `get_usage`，前端调一次能在 Dashboard 显示假数据。

3. **MiniMax Provider**
   - `providers/minimax.rs`：`reqwest` GET `/v1/token_plan/remains`，解析返回为 `UsageSnapshot`。
   - `storage.rs`：建 `daily_snapshots` 表，每次 fetch 成功后 upsert 今日 `used` 值。
   - `get_heatmap` 优先返回表里近 90 天记录，缺失日期补 0 + 灰色。

4. **DeepSeek Provider**
   - `providers/deepseek.rs`：并行调 `/user/balance` + `/v1/billing/usage`。
   - `balance` 填入 `UsageWindows.balance`；`billing` 填入 `monthly` 窗口 + 写 `daily_snapshots`。

5. **Volcano Provider + 签名**
   - `signing.rs`：实现 V4 HMAC-SHA256 签名单元，写一组 cargo test 用官方文档示例串验证。
   - `providers/volcengine.rs`：调 `GetAFPUsage` 填四窗口；调 `GetUsageDetails` `QueryInterval=Day` 填 `heatmap`。

6. **调度器 + 事件**
   - `scheduler.rs`：tokio::spawn 每个 Provider 一个 task；`emit("usage-updated", snapshot)`。
   - 前端 `listen("usage-updated")` 增量更新对应卡片。
   - `reset-tick` 每秒触发用于倒计时。

7. **凭证与设置**
   - `settings.rs`：keyring 读写 + `config.toml` 读写。
   - `Settings.svelte` + 独立 Tauri 窗口：Provider 启用开关、凭证录入、轮询间隔、连通性测试按钮。

8. **HeatmapGrid + ResetCountdown**
   - 实现日历热力图与重置倒计时，确保三家 Provider 数据都正确显示。

9. **Compact 模式 + 拖拽**
   - `CompactBall.svelte` + `set_window_mode` IPC + `data-tauri-drag-region`。
   - 持久化看板位置到 `config.toml`。

10. **frontend-design 打磨**
    - 调用 `trae-remote-official:frontend-design:frontend-design` skill 输出现代简约设计 token + 组件样式，按其建议重写 `tokens.css` 与各 `.svelte` 视觉。

11. **打包验证**
    - `npm run tauri build` 产出 `.msi` + `.exe`。
    - 在 Win11 与 Win10 验证：透明置顶、内存占用 < 80MB、三家 Provider 真实数据展示、热力图渲染、刷新间隔正常、拖拽位置持久化。

## 8. 验证步骤

- **签名单元测试**：用 Volcano 官方文档示例串跑 `signing.rs`，输出与文档 `Authorization` 头完全一致。
- **Provider 解析测试**：mock 三家 API 响应 JSON，断言 `fetch_usage` 输出 `UsageSnapshot` 字段映射正确。
- **透明窗口冒烟**：启动后窗口 `alwaysOnTop` 生效、`transparent` 生效、`skipTaskbar` 生效、可拖拽。
- **真实账号联调**：用户录入真实凭证，验证三家 Provider 都能在 5 分钟内拿到非零数据。
- **热力图冒烟**：MiniMax/DeepSeek 首次启动显示"数据积累中"，连续运行 3 天后能看到至少 3 个色块；火山立即有当日数据。
- **资源占用**：任务管理器查看进程内存 < 80MB、CPU 空闲时 ~0%。
- **失败恢复**：手动断网 5 分钟，验证 UI 显示"上次刷新失败"且恢复后自动续上。

## 9. 后续扩展路径（不在 MVP 内）

- 桌宠形态（Q4）：独立 Svelte 组件 + Lottie/像素动画。
- macOS / Linux 移植：Tauri 跨平台特性已就位，主要替换 keyring 后端、调整窗口透明 API。
- Provider 插件市场：开放 `Provider` trait + 用户级 WASM 模块加载。
- 移动端：复用 Rust provider 模块，前端改 RN/Tauri Mobile。
