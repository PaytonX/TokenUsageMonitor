# 悬停锚定 / 新增 Provider（xAI · Kimi · Codex）/ Provider Logo / 本地代理 设计

- 日期：2026-09-25
- 状态：已批准（待实施）
- 对象：悬停详情卡交互、Provider 注册表扩展、品牌 Logo 组件、HTTP 代理支持

---

## 1. 背景与问题

本次迭代解决四个独立但相关的问题：

1. **悬停详情卡难以触达**：详情卡固定停靠在窗口底部（`.detail-overlay` fixed bottom:48px），与触发卡片距离远；鼠标移开卡片 160ms 即隐藏，滑过其他卡片时弹窗立即被替换。用户几乎无法把鼠标从卡片移动到弹窗上，点不到弹窗内按钮。
2. **常用 Provider 缺失**：用户希望接入 ChatGPT、Anthropic、Gemini、Grok、Kimi、GLM。调研结论：
   - **xAI（Grok）**：`GET https://api.x.ai/v1/usage`（Bearer 认证）公开可用，返回日粒度用量含消费金额；预充值 credits 计费，余额端点需实施时实测。
   - **Kimi（Moonshot）**：`GET https://api.moonshot.cn/v1/users/me/balance` 公开可用，返回可用余额/代金券/现金余额；国内站（api.moonshot.cn）与国际站（api.moonshot.ai）key 不通用。
   - **ChatGPT Codex**：`GET https://chatgpt.com/backend-api/codex/usage`（实验性、未文档化），用本机 Codex CLI 的 OAuth access_token（`~/.codex/auth.json`）访问，返回 5 小时窗口与每周窗口的用量百分比及重置时间。
   - **Anthropic**：用量 API 属 Admin API，需组织账户 + Admin key（`sk-ant-admin01-...`），个人账户不可用 → 暂缓。
   - **GLM / Gemini**：无公开余额查询端点 → 暂缓。
3. **缺少品牌视觉**：目前卡片用 PulseDot 圆点 + 文字缩写标识 Provider，pill 侧栏用吉祥物图片（mascotUrl），无品牌原色 Logo，识别度低。
4. **无代理支持**：所有 Provider 请求与 hub 上报直连；需要网络代理的用户（http/socks5）无法配置。

## 2. 决策记录

| # | 决策点 | 结论 | 理由 |
|---|--------|------|------|
| D1 | 悬停弹窗优化方式 | 就近锚定（弹窗出现在触发卡片旁） | 解决距离远 + 移开即消失的根本问题；接受遮挡相邻卡片的代价 |
| D2 | ChatGPT 接入方式 | 两者都要：保留现有 openai API 直连 + 新增 codex 订阅用量 | openai kind 已存在无需动；Codex 面向订阅用户，二者场景不同 |
| D3 | 数据受限 Provider | 只接数据完整的：仅新增 xAI 与 Kimi | Anthropic 需组织 Admin key、GLM/Gemini 无公开余额端点，强行接入体验差 |
| D4 | Logo 视觉 | 品牌原色内联 SVG | 无额外资产文件、缩放无损、与软件风格（圆角、克制配色）兼容 |
| D5 | 本地代理 | Settings 配置显式代理 URL（http/socks5），空值保持 reqwest 默认行为 | reqwest 原生支持；显式 URL 覆盖默认系统代理探测 |

## 3. 详细设计

### 3.1 悬停弹窗就近锚定

**现状链路**（均在 `src\App.svelte`）：卡片 `pointerenter` → `onCardHover` 设置 `hoveredId` → `detailSnapshot` derived 取对应快照 → `.detail-overlay`（fixed，停靠窗口底部）渲染 `DetailCard`；离开卡片走 `scheduleOverlayHide`（160ms 宽限），进入 overlay 钉住；滑到其他卡片直接替换。

**改动**（仅 `App.svelte`）：

1. **锚定定位**：`onCardHover` 时用 `event.currentTarget.getBoundingClientRect()` 记录卡片矩形到本地状态 `anchorRect`。`.detail-overlay` 保持 fixed 定位（`getBoundingClientRect()` 返回视口坐标，与 fixed 坐标系一致，免换算），由底部停靠改为动态坐标：弹窗左上角 = 卡片矩形正下方 `top = rect.bottom + 8`，水平方向与卡片左缘对齐后做视口内收拢（`clamp` 到 `rect.right - overlayWidth` 与窗口边界之间）。空间不足（`rect.bottom + 8 + overlayHeight > 视口底`）时翻转到卡片上方（`top = rect.top - 8 - overlayHeight`）。
2. **测量校正**：弹窗内容高度事先未知——首次渲染到离屏位置后用 `bind:clientHeight/clientWidth`（或 `getBoundingClientRect`）实测尺寸，再计算最终坐标；尺寸变化（内容更新）时重算。用 `requestAnimationFrame` 或 Svelte `$effect` 触发，避免闪烁。
3. **宽限延长**：`scheduleOverlayHide` 延时 160ms → **350ms**。移到弹窗内仍然钉住（现有逻辑保留）。
4. **滑过切换保留**：鼠标滑过其他卡片时 `hoveredId`/`anchorRect` 一并更新，弹窗原地跟随新卡片（不做整窗淡出淡入，仅内容切换）。
5. **过渡**：`.detail-overlay` 加 fade + 6px 纵向位移过渡（Svelte `transition:fly={{ y: 6, duration: 120 }}` 或 CSS），遮挡相邻卡片由 D1 决策接受。

边界：窗口被拖动/缩放时弹窗跟随卡片矩形失效 → 监听期间若 hovered 卡片已移除（rect 失效），按现有 overlay hide 逻辑关闭即可；不处理拖动中的实时跟随。

### 3.2 新增 Provider：xAI / Kimi / Codex

三者均走现有 registry 驱动架构：新模块 + `PROVIDER_REGISTRY` 一行 + `PRESETS` 条目 + 前端短名（`types.ts` 的 `SHORT_KIND_NAMES` 已预置 `kimi: "Kimi"`，xAI/codex 需补两条）。内置三单测（kind 唯一 / preset 暴露 / 可纯接线构建）自动覆盖。

#### 3.2.1 xAI（Grok）— `providers/xai.rs`

- kind `xai`，display_name `xAI Grok`，accent `#1d1d1d`（品牌黑），auth `BearerKey`。
- `fetch_usage`：`GET https://api.x.ai/v1/usage`，`Authorization: Bearer <api_key>`。响应为日粒度用量数组，逐项含消费金额（USD）。
- 映射：聚合当月各日 cost → `windows.monthly.used`（unit `Usd`，`cost_source: ProviderReported`）；逐日数据写入 heatmap；token 数若有则填 `tokens`。
- 余额：credits 余额端点实施时实测，能取到则填 `BalanceInfo`（currency `Usd`），取不到则本期只展示消费。

#### 3.2.2 Kimi（Moonshot）— `providers/kimi.rs`

- kind `kimi`，display_name `Kimi`，accent 青绿 `#16c2a3`（实施时按品牌微调），auth `BearerKey`。
- **sub_modes 区分站点**：主 mode = 国内站（`https://api.moonshot.cn`）；sub_mode `kimi_global` = 国际站（`https://api.moonshot.ai`）。两站 key 不通用，实例隔离。
- `fetch_usage`：`GET {base}/v1/users/me/balance`，Bearer。响应含可用余额、代金券余额、现金余额（字段名以实测为准，聚合口径：现金 + 代金券 = 可用总额）。
- 映射：`windows.balance` = `BalanceInfo { total, currency: CNY|Usd（按站点）, is_available: true }`，unit `Cny`/`Usd`。
- **消费记账复用 DeepSeek 余额差先例**（`provider_kv` 存 `last_balance` / `month_key` / `month_start_balance` / `month_topup`）：余额下降差额记入当日消费（`cost_source: Estimated`），月初用 `month_start_balance` 重置基线；`window_monthly` 据此展示月消费。

#### 3.2.3 ChatGPT Codex（订阅用量）— `providers/codex.rs`

- kind `codex`，display_name `ChatGPT Codex`，accent `#10a37f`（与 openai 同绿），auth 新变体 `LocalToken`（见 3.2.4）。Preset 新增 `experimental: true` 标记，Settings 网格与卡片显示「实验」徽章。
- **凭证来源**：
  - 自动检测：读 `%USERPROFILE%\.codex\auth.json`，解析 `tokens.access_token`（JSON 结构含 `tokens.{id_token,access_token,refresh_token,account_id}` 与 `last_refresh`）。
  - 手动兜底：Settings 表单可直接粘贴 access_token。
- `fetch_usage`：`GET https://chatgpt.com/backend-api/codex/usage`，`Authorization: Bearer <access_token>`；若响应/凭证含 `account_id` 则附对应 header（实施时实测）。
- 映射：5 小时窗口 → `windows.five_hour`（`used`/`quota: 100`/unit `Percent`/`reset_at`），周窗口 → `windows.weekly`（同理）。`plan_tier` 显示订阅档位（响应或 id_token 中可辨识时）。
- 错误语义：401 → `ProviderError::Auth`，前端提示「Codex 凭证失效，请在本机运行 `codex login` 重新授权」。
- **实验性声明**：端点未文档化，随时可能变动/失效；失败时卡片显示明确错误，不影响其他 Provider。

#### 3.2.4 凭证模型扩展 `AuthKind::LocalToken`

`providers/mod.rs`：

```rust
pub enum AuthKind { BearerKey, AccessKeySecret, LocalToken }

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Credentials {
    BearerKey { api_key: String },
    AccessKeySecret { access_key: String, secret_key: String },
    LocalToken { token: String },
}
```

- `Credentials` 序列化形式 `{ kind: "local_token", token: "..." }`，随现有 keyring 流程存储（`save_credentials`/`delete_credentials` 不需要结构改动，仅 match 分支补全）。
- `fetch_usage` 实现处（新 Provider）match 到 `LocalToken { token }` 使用；`test_provider` 通用链路自动兼容。
- 前端 `Settings.svelte` 的 `buildCredentials` 按 preset 的 `auth_kind === "local_token"` 产出对应表单（单输入框，placeholder 提示「留空则自动读取本机 ~/.codex/auth.json」）。
- 自动读取：新增 ipc 命令 `detect_codex_token() -> Option<{ token, account_id, last_refresh }>`，前端在 codex 表单挂载时调用并预填。

### 3.3 Provider Logo 组件

新增 `src\lib\components\ProviderLogo.svelte`，props `{ kind: string, size?: number = 18 }`。

- **实现**：内部 `switch (normalize(kind))` 返回品牌原色内联 SVG path（minimax / deepseek / volcengine / openai / xiaomi / xai / kimi / codex ...），外包统一圆角容器。**`_api` 变体复用母品牌**（`kind.split("_")[0]` 归一化）。
- **fallback**：未知 kind → 首字母色块（取该实例 accent_color 或默认蓝）。
- **接入点**：
  1. 卡片头：`PulseDot` 保留状态色，右侧叠加 Logo 小徽标（右下角叠放，圆形描边）；
  2. 详情弹窗标题旁（`DetailCard` 头部）；
  3. `MiniPanel` 每行行首替换现有文字缩写；
  4. `Settings.svelte` preset 网格项；
  5. pill 侧栏 `pill__avatar` 与 dashboard `shell__avatar`：`headerIsPayAsYouGo` 等分支不变，mascotUrl 图片替换为焦点 Provider 的 Logo（非订阅场景即焦点 Provider Logo；无焦点时保留现 mascot 或默认 Logo）。
- mascot 资产文件保留不删，仅展示切换。

### 3.4 本地代理支持

**原则**：reqwest `Client` 的代理在构建后不可变 → 代理变更 = 重建 Client + 全量重建 registry。空值/清空 = 不显式设置（保持 reqwest 默认行为：跟随系统代理环境变量）。

1. **配置**：`settings.rs` 的 `Settings` 新增 `proxy_url: Option<String>`（支持 `http://host:port`、`socks5://host:port`，URL 内嵌用户名密码由 reqwest 原生解析）。config.toml 持久化，缺省 `None`。
2. **Client 构建**（`lib.rs` 187-190 附近）：抽出 `fn build_http_client(proxy_url: Option<&str>) -> Client`——`Client::builder().timeout(15s)`，`proxy_url` 非空时 `.proxy(reqwest::Proxy::all(url)?)`；启动与保存时共用。
3. **共享状态**：`AppState.http: Client` → `AppState.http: Arc<RwLock<Client>>`；所有取用处读锁 clone（Client 内部 Arc，clone 廉价）。
4. **hub/exchange 复用**：
   - `hub.rs` `report_to_hub` / `fetch_devices`（170、194 行各自 `Client::new()`）→ 签名增加 `client: &Client` 参数，调用方（ipc.rs）从 `state.http` 传入；
   - `exchange.rs` 生产路径已接收注入 client，不改（仅测试代码自建 `Client::new()`，无需代理）。
5. **保存生效**（`ipc.rs` `save_settings`）：比较新旧 `proxy_url`，变化时：`build_http_client(new)` → 写回 `state.http` → **全量重建 registry**（新增辅助函数：从 `settings_store.read_blocking().accounts` + `state.credentials` 重新 `build_registry` 替换，进行中的轮询任务随之重建；`settings_wake` 照常 ping）。非代理变更仍走现有增量 reconcile，避免无谓中断。
6. **Settings UI**（`Settings.svelte`）：新增「网络」区块（样式参照 hub 区块）——启用开关 + URL 输入框（placeholder：`http://127.0.0.1:7890 或 socks5://127.0.0.1:1080`）+ 「测试连接」按钮 → 新 ipc 命令 `test_proxy(url) -> { ok, status, error }`（用临时 client 请求一个轻量 204 端点），提示文案说明「保存后生效，已连接的账号会自动重建」。

## 4. 改动面

| 文件 | 改动 |
|------|------|
| `src\App.svelte` | 悬停锚定（anchorRect、定位/翻转/测量校正、宽限 350ms、过渡）；卡片头/pill/shell avatar 接入 ProviderLogo |
| `src\lib\components\ProviderLogo.svelte` | **新增**：品牌原色内联 SVG + fallback |
| `src\lib\components\DetailCard.svelte` | 标题旁 Logo |
| `src\lib\components\MiniPanel.svelte` | 行首 Logo |
| `src\lib\types.ts` | `Settings.proxy_url`；`SHORT_KIND_NAMES` 补 `xai`/`codex`；`TestResult`/凭证类型补 `local_token` |
| `src\Settings.svelte` | codex 表单（LocalToken + 自动检测 + 实验徽章）；preset 网格 Logo；「网络」代理区块 + test_proxy |
| `src-tauri\src\providers\mod.rs` | `AuthKind::LocalToken` + `Credentials::LocalToken`；`Preset.experimental`；PRESETS 3 条目（xai/kimi/codex，kimi 含 kimi_global sub_mode）；PROVIDER_REGISTRY 3 条目 |
| `src-tauri\src\providers\xai.rs` | **新增** |
| `src-tauri\src\providers\kimi.rs` | **新增**（含余额差记账） |
| `src-tauri\src\providers\codex.rs` | **新增**（auth.json 读取 + usage 拉取） |
| `src-tauri\src\settings.rs` | `Settings.proxy_url: Option<String>` |
| `src-tauri\src\lib.rs` | `build_http_client`；`AppState.http: Arc<RwLock<Client>>`；全量重建 registry 辅助函数 |
| `src-tauri\src\ipc.rs` | `save_settings` 代理变更分支；`test_proxy`、`detect_codex_token` 命令；hub 调用传 client |
| `src-tauri\src\hub.rs` | `report_to_hub`/`fetch_devices` 接收共享 client |
| `src-tauri\src\providers\exchange.rs` | 不改（生产路径已注入 client） |

## 5. 范围外

- Anthropic / GLM / Gemini 接入（数据受限，待官方开放后再评估）。
- Codex `refresh_token` 自动刷新（本期仅 401 提示手动 `codex login`）。
- Codex 端点变动跟进（实验性端点，失效时下线该 Provider 即可）。
- mascot 图片资产删除（保留文件，仅展示切换）。
- PAC 脚本、按域名分流等高级代理形态（仅全局 http/socks5）。
- 代理应用于 Tauri webview 自身的网络请求（仅覆盖 Rust 侧 reqwest 流量）。

## 6. 已知限制与风险

| 风险 | 影响 | 缓解 |
|------|------|------|
| Codex 端点未文档化，可能变动/风控 | 该 Provider 拉取失败 | 实验徽章明示；错误独立展示不影响其他卡片；必要时下线 |
| 就近锚定遮挡相邻卡片 | 遮挡 | 用户已接受（D1）；350ms 移开即隐藏 |
| Kimi 国内/国际站 key 不通用 | 用户配错站点报 401 | sub_modes 隔离实例 + 表单注明 |
| xAI usage 字段（余额）未经实测 | 字段名可能不符 | 余额部分实施时实测，取不到则只展示消费，不阻塞 |
| 代理变更触发全量 registry 重建 | 进行中轮询请求中断一次 | 重建后 settings_wake 立即补偿拉取 |
| Codex auth.json 由 Codex CLI 维护，格式可能演进 | 自动检测失败 | 保留手动粘贴 access_token 兜底 |

## 7. 验收标准

- [ ] 悬停任意卡片，详情卡出现在该卡片正下方 8px（视口边缘自动翻转），水平不超出窗口
- [ ] 鼠标从卡片移向详情卡（路径 < 350ms）弹窗不消失，进入弹窗后可点击
- [ ] 滑过其他卡片时弹窗内容切换且位置跟随新卡片
- [ ] Settings 可添加 xai / kimi（含国际站）/ codex 三类账号，凭证保存于 keyring
- [ ] xAI 卡片显示当月消费（USD，ProviderReported）；Kimi 显示可用余额（CNY/USD 按站点）；codex 显示 5h 与周窗口百分比及重置倒计时
- [ ] Kimi 余额下降正确计入日消费（Estimated），月初基线重置正确
- [ ] codex 凭证留空时自动读取本机 `~/.codex/auth.json`；401 时提示 `codex login`
- [ ] xai/kimi/codex 实例在卡片头、详情弹窗、MiniPanel、Settings 网格显示品牌原色 Logo；`_api` 变体复用母品牌；未知 kind 显示首字母色块
- [ ] pill 侧栏与 dashboard 头像显示焦点 Provider Logo
- [ ] Settings「网络」区块可配置 http/socks5 代理，测试连接按钮返回可达性；保存后 Provider 请求实际走代理（对比测试连接前后的成功/失败）；清空恢复默认行为
- [ ] `npx svelte-check --threshold error`、`npm run build`（impl-pulse）与 `$env:CARGO_TARGET_DIR="D:\cargo-target"; cargo test --lib`（src-tauri）全部通过
