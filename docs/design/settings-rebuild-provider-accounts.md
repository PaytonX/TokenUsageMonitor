# 设置界面重构 + Provider 多账号 + 交互增强

## Context（背景与目标）

本应用是 Tauri 2 + Svelte 5 的 AI 用量监控面板（当前内置 provider：DeepSeek / MiniMax / 火山引擎，设置界面为单列滚动）。用户希望参考开源项目 qunqin24/Pulse（一个 macOS Swift 的屏幕边缘配额监控器）的设计范式，做三件事：

1. **设置界面完全重构**：改为「左侧分组导航 + 右侧内容面板」的分栏形态（Pulse 式），容纳更多设置项与内容。
2. **Provider 更灵活**：把内置 3 家做成可复用的「预设目录」，并引入**多账号**——同一提供商可挂多个账户，每个账户有独立凭证、自定义标签、强调色、启停开关。
3. **交互增强**（四项全做）：
   - **Countdown 模式**：圆环显示「已用%」↔「剩余%」切换并可记忆。
   - **每账号强调色**：把当前固定青色改为每账户可自定义强调色。
   - **hover 详情卡强化**：展示全部额度池、重置倒计时、ETA 预测、刷新/重连/诊断入口。
   - **通知分级可配**：警告/告急阈值（默认 80/95）可在设置中微调。

技术约束与既有能力（探查结论）：
- 后端已有统一 `Provider` trait（`id()`/`display_name()`/`auth_kind()`/`fetch_usage()` → `UsageSnapshot`），见 `src-tauri/src/providers/mod.rs`；新增 provider 只需实现 trait。
- 凭证按 provider_id 存入 OS keyring（`settings.rs::save_credentials/load_credentials`），已参数化，天然支持任意实例 id。
- 复用的前端 `Settings` 类型：`src/lib/types.ts`。
- 复用现有的 `App.svelte` 环形窗口(`ringWindow`/`ringWindowRemaining`)、`ProgressRing` 双环、`ProviderCard`、`HeatmapGrid`、compact 胶囊。

## 设计总览

引入**「账户(Account)」为一等实体**，替代当前「provider_id 单例」。

```
Account {
  instance_id: String        // 唯一，如 "minimax|家庭"/uuid(短)
  provider_kind: String      // "deepseek"|"minimax"|"volcengine"（映射到内置 preset）
  label: String              // 自定义显示名，默认取 provider 显示名
  accent_color: String       // #RRGGBB，每账户强调色
  enabled: bool              // 是否参与轮询
  note/extra: Option<String> // 可选备注
}
```

- **Registry 演进**：`ProviderRegistry` 从「只注册 provider 类型」改为「按账户实例产出 provider 实例」（同一 kind 可多实例）。`scheduler.rs::spawn_all`、`force_refresh`、`get_usage`、`get_providers` 从遍历 provider 改为遍历 Accounts。
- **凭证 keyring**：service name 仍 `TokenUsageMonitor`，entry key 用 `instance_id`。
- **向后兼容**：启动时若持久化的 accounts 为空，自动从旧 `enabled_providers` + 已有凭证构造初始账户，避免用户现有配置丢失。

## 分阶段任务

### Phase A — 后端数据模型：账户(Account) 支持多账号
文件主要在 `src-tauri/src/`。

1. `providers/mod.rs`
   - 新增 `AccountMeta`/账户元数据（instance_id, provider_kind, label, accent_color, enabled）。
   - `ProviderRegistry` 增加按实例创建 provider：`build_instance(kind, &Account) -> Arc<dyn Provider>`；同一 kind 可 `build` 多次。
   - 保留现有 3 个内置 provider 实现（`deepseek.rs/minimax.rs/volcengine.rs`）——它们是不同 kind 的工厂。
2. `settings.rs`
   - `Settings` 新增 `accounts: Vec<AccountMeta>`（`#[serde(default)]`，缺省空）。
   - 启动迁移：若 `accounts` 为空且存在 `enabled_providers`/凭证，则构造初始账户。
   - 凭证 keyring 存取 `load_credentials/save_credentials/delete_credentials` 改为按 instance_id（签名不变，语义改为实例）。
3. `lib.rs`
   - `build_registry` 改为「从 settings.accounts 构建账户实例 registry」。
   - `KNOWN_PROVIDER_IDS` 保留为「预设 provider 种类」目录（predefined provider 目录），供设置界面展示可选预设。
   - `load_credentials_into_cache` 改为按所有账户实例加载。
4. `scheduler.rs`
   - `spawn_all`/轮询按 `accounts`（enabled 的账户）遍历，`poll_one` 传入账户实例。
5. `ipc.rs`
   - `get_providers` → 改为返回「预设目录」+「已建账户列表」两部分（前端使用）。
   - 新增账户 CRUD：`upsert_account` / `remove_account`（同时清理其 keyring 凭证）。
   - `save_settings` 同步账户并触发轮询唤醒；`force_refresh` 按账户过滤。
   - `test_provider` 语义改为对某 kind/账户做连接测试。

### Phase B — 设置界面分栏重构（Pulse 式）
文件：`src/Settings.svelte`（大幅重构）、`src/lib/types.ts`（Account 类型 + Settings 字段）、`src/lib.ts`（invoke 封装新增账户 CRUD）。

- **布局**：左栏分组导航（通用 / 账户与额度 / 交互与通知 / 关于与诊断），右栏内容面板，每组一个 `settings__pane`。
- **通用**：轮询间隔、自动启动说明。
- **账户与额度**：预设目录展示（DeepSeek/MiniMax/火山引擎，含「添加账户」按钮）→ 点击展开凭证表单 + 自定义标签 + 强调色选择器 + 启用开关 + 测试/保存/删除。每账户一张卡片（复用现有 `behavior-row`/`toggle`/`field` 样式）。
- **交互与通知**：环形窗口下拉（已有 `ringWindow`）、countdown 模式开关、每账户默认强调色、通知开关 + 警告/告急阈值输入（0-100），保留靠边吸附开关。
- **关于与诊断**：版本、提供商列表、导出诊断文本（可选，占位）。
- svelte-check 必须 0 errors / 0 warnings；`overflow-y` + `min-height:0` 的滚动约束沿用修复后的写法。

### Phase C — 交互增强（前端）
文件：`src/App.svelte`、`src/lib/components/ProgressRing.svelte`、`src/lib/components/ProviderCard.svelte`、`src/lib/components/DetailCard.svelte`、`src/lib/types.ts`。

1. **Countdown 模式**：`Settings.countdown_mode: "used"|"remaining"`（默认 used）。`ProgressRing` 输入 `value + wantRemaining`，显示 `1 - used` 与「剩余 xx%」，主面板/胶囊/卡片统一。
2. **每账号强调色**：按账户 `accent_color` 生成 CSS 变量（HSV→RGBA 强调色 + 光晕），provider 卡、圆环、胶囊、heatmap 点缀取账户色。`tokens.css` 保留全局变量，账户色在 `App.svelte`/卡片内联覆盖。
3. **hover 详情卡强化**（`DetailCard.svelte` + `App.svelte` detail-overlay）：展示所有额度池行（上限/已用/重置倒计时）、ETA 行（按烧速预测耗尽时间，可复用 `BurnInfo`/`burn` 字段）、「刷新」按钮（`force_refresh`）、「打开设置 / 连接诊断」动作。
4. **通知分级可配**：`settings` 已有 `notify_warn_percent/notify_crit_percent`，后端 `scheduler.rs` 已读取；仅需在 **Phase B** 设置 UI 暴露为可配输入并写回（后端逻辑无需改动，改前先确认 `notify.rs::evaluate` 读取方式）。补充 UI 上对「0=关闭该级」的轻提示。

## 关键文件
- `src-tauri/src/providers/mod.rs`、`deepseek.rs`、`minimax.rs`、`volcengine.rs`（trait/Registry/内置实现）
- `src-tauri/src/settings.rs`（Account 字段 + keyring 按实例）
- `src-tauri/src/lib.rs`（registry 构建、加载凭证）
- `src-tauri/src/scheduler.rs`、`src-tauri/src/ipc.rs`（账户驱动轮询 + CRUD + 诊断）
- `src/Settings.svelte`、`src/lib/types.ts`、`src/lib.ts`
- `src/App.svelte`、`src/lib/components/{ProgressRing,ProviderCard,DetailCard,MiniPanel}.svelte`

## 复用清单
- 凭证存取：`settings.rs::save_credentials/delete_credentials`
- 环形窗口解析：`types.ts::ringWindowRemaining`、`mostCriticalWindow`
- 双环：`ProgressRing` 的 `outerValue/outerGap`；烧速字段 `BurnInfo`（`burn`）
- 设置滚动约束：`.settings__section:nth-of-type(N){flex:1;min-height:0;overflow-y:auto}`（修复后的写法）

## 验证
1. `cargo test`（settings 默认值/迁移用例）、`cargo check`（0 error）。
2. `npm run build`（svelte-check 0/0）+ `npm run tauri:dev` 手动验证。
3. 端到端手测：
   - 设置→账户与额度：添加第二个 DeepSeek 账户（不同 label/强调色），两账户卡片并存且分别可保存/测试/删除；删除会清 keyring 凭证。
   - 主面板与胶囊：两账户按各自强调色显示；hover 详情含额度池/ETA/刷新；countdown 切换到「剩余%」后圆环与数值即时翻转并可记忆。
   - 通知：改警告阈值到 95 保存，观察只在跨越新阈值时触发（`notify.rs` 状态机）。
   - 旧配置迁移：删掉 `accounts` 后重启仍能凭旧 `enabled_providers`+凭证自动建账户。