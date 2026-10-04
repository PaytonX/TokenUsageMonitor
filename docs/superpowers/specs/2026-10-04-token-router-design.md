# TokenRouter — 本地路由代理设计

- 日期：2026-10-04
- 状态：已批准（rev2，含用户修订：设置页承载配置 / 主界面快速开关 / 新分支实施）
- 分支：`feat/token-router`
- 关联：`docs/usage-ledger.md`（账本双写规则）、`src-tauri/src/hub.rs`（本地服务先例）

## 1. 背景与问题

长任务（Claude Code / ZCode 等 CLI 驱动的编码会话）运行中，当前 provider 的 token plan
用尽时任务即受阻暂停：工具本身只认单一上游，用户必须人工换配置、换账号才能继续。

TokenUsageMonitor 已经聚合了全部 provider 账户的配额快照（`WindowUsage` 五窗口、燃烧率
ETA、窗口重置检测），但只"看"不用。TokenRouter 把这份数据变成行动：在本机开一个反向
代理端口，工具把 baseURL 指过来，请求经 TokenRouter 转发到当前可用的 provider 账户；
配额耗尽自动切换到备选，长任务无感继续。

## 2. 决策记录

| # | 决策点 | 结论 | 理由 |
|---|---|---|---|
| D1 | 形态 | 内置本地反向代理 | 只有请求流经路由器才能无感切换（用户确认） |
| D2 | 协议面 | Anthropic `/v1/messages` + OpenAI `/v1/chat/completions`（含 SSE），同协议转发；跨协议翻译后补 | 翻译错译风险大；路由链上候选必须同协议 |
| D3 | 路由识别 | 多条命名路由，每条一个本地 token（`tr_<routeId>_<hex>`，存 config.toml）；工具以该 token 作 API key；仅绑定 127.0.0.1 | Claude Code 只能改 BASE_URL+TOKEN，无法加路径前缀；按 token 识别对工具零侵入 |
| D4 | 代理实现 | axum + reqwest(stream)；独立无总超时的上游 Client | hub.rs 阻塞实现承载不了 SSE；共享 http client 有 15s 总超时，不能复用 |
| D5 | 切换触发 | **四层判定**：① 主动预警（可设阈值，0 = 关闭）以**最小（最短周期）在报窗口**的剩余 % 为准；② 硬墙兜底（任一窗口/上限打到 100% 即不可用）；③ 被动观测（限流/配额立即冷却换线、连接异常原地重试一次 + 连续熔断才标记、4xx 透传）；④ 切回探视（资格判定 + 真实请求探针 + ×2 指数退避 + 达上限等 reset_at） | 避免「月剩 5% / 5h 剩 98%」误切；不预测、只观测，误切为零 |
| D6 | 配额输入 | 复用 SharedProviderState/UsageSnapshot.min_remaining_percent；**候选级**手填日上限 `Candidate.plan_limit_tokens_daily`（无配额 API 的 provider；路由器直读 settings，不动 Provider 构造签名）；实际消耗由路由器自记账 | provider 无配额 API 时主动切换的依据 |
| D7 | UI 布局 | 设置页"路由"pane 承载全部配置；主界面 header 加快速开关（绿/灰/琥珀状态点）；切换时事件 + 系统通知 | 用户修订；快速开关关闭时监听保留、请求返回 503 + 原因 |
| D8 | 凭据纪律 | 上游凭据一律 keyring（BearerKey 账户才可作候选）；路由本地 token 为本机自生成值存 config.toml（hub_token 先例）；源码/测试零凭据字面量 | Mimosa 约束 + keyring 既有体系 |
| D10 | 路由数量 | **单路由**：整个 TokenRouter 一条链（`routes` 仅保留首条以平滑迁移旧配置）。用户提出多路由 + 模型别名分派后评估：两条链的切换状态、探视退避、状态展示都要独立维护，维护成本与收益不成比例 | 简化配置与心智负担；并行多工具的需求用「多窗口/多实例」而非多路由满足 |
| D9 | 记账口径 | 路由消耗写 `usage_daily`，`source = "router:<instance_id>"`、`kind='provider'`、仅 model='' 总量行，增量累计；不与 REPLACE 回放型来源冲突 | 账本规定"差分类不与回放类同键"；`router:*` 键无其他写入方 |

## 3. 详细设计

### 3.1 配置模型（config.toml）

```toml
[router]
enabled = false
port = 43211
failover_threshold_percent = 20   # 主动切换：剩余% 低于此值跳过该候选
failback_threshold_percent = 50   # 链头候选恢复到该剩余% 以上才切回
error_cooldown_secs = 300         # 配额/凭据错误默认冷却

[[router.routes]]
id = "r1"
name = "Claude Code 主力"
protocol = "anthropic"            # anthropic | openai
token = "tr_r1_<32hex>"           # 保存时空 token 由 Rust 自动生成补齐

[[router.routes.candidates]]
account = "anthropic-123-0"       # AccountMeta.instance_id
model = "claude-sonnet-4-5"
base_url = "https://api.anthropic.com"

[[router.routes.candidates]]
account = "minimax-456-0"
model = "MiniMax-M2"
base_url = "https://api.minimaxi.com"
plan_limit_tokens_daily = 10000000   # 可选：手填日上限（tokens/自然日）
```

- `Settings.router: RouterSettings`（serde default，旧配置零迁移成本；v1 的
  `failover_threshold_percent` 等字段被 serde 忽略，新字段取默认值）。参数：
  `enabled` / `port` / `proactive_threshold_percent`（主动预警阈值，0 = 关闭）/
  `error_cooldown_secs` / `conn_breaker_count` / `probe_start_secs` /
  `probe_max_attempts`。
- 候选 `base_url` 必填：路由器对上游零假设（官方/中转站皆可），设置页按账户 kind 预填。
  Anthropic 中转站因此不需要独立的 `anthropic_api` 账户 kind——中转差异由候选 base_url 承载。
- 手填日上限在**候选级**：路由器直读 settings 计算 used/limit，无需触碰 Provider
  构造签名（13 个 provider 的 uniform registry 保持不动）。
- 凭据要求：候选账户必须是 `Credentials::BearerKey`（请求时校验，非 BearerKey 记错误态并跳过）。

### 3.2 四层判定（router/decision.rs，纯函数）

**① 主动预警（可关）**：`Constraint { period_rank, remaining_percent, full }` 集合经
`assess()` 汇总——`min_window_remaining` 取**周期序最小者**（five_hour 0 < daily 1 <
weekly 2 < monthly 3）的剩余 %（同周期多来源取更小），低于
`proactive_threshold_percent` → 该线路不可用。阈值 0 = 关闭。无刻度数据不拦截。

**② 硬墙兜底**：`full`（used ≥ quota / over_quota / 达到手填上限）→ 立即不可用，
与阈值无关。

**③ 被动观测**（`on_result`）：QuotaError → 冷却 `Retry-After ?? error_cooldown_secs`
（限流不会毫秒级恢复，不原地重试）；AuthError → 凭据错冷却；ServerError → **只累计
`consecutive_conn_errors`，达 `conn_breaker_count` 才短冷却 60s**（单次抖动不杀线路，
服务器层另有一次原地重试）；ClientError → 不动状态。

**④ 切回探视**：选择恒按链序取第一个可用者，链头恢复自然回主线路；主线路「无窗口
打满 + 最小窗口有余量」= 有资格。资格满足但实测仍限流 → `ProbeState` 记录
`attempts` 与 `next_probe_at`（×2 退避），未到期的探视**连降级路径都跳过主线路**；
达 `probe_max_attempts` 后 `next_probe_at` = 快照里的 `reset_at`（拿不到则 15 分钟
低频）。探视成功（主线路 2xx）清空探视态。资格丢失（有墙打满）立即清空。

`pick_candidate(candidates, states, route_state, policy, now)`：主线路
（index 0）处于探视退避中且未到期 → 跳过；到期且未打满 → 放行一次作为探针；其余
按 `available()` 判定，无可用者时降级到第一个未冷却者。

每路由运行态 `RouteState { last_used_index: Option<usize>, candidates: Vec<CandidateState> }`，
`CandidateState { cooldown_until, cooldown_reason(Quota|Auth|Error), last_error }`。
low_quota 不落运行态，每次请求由快照现算。

可用性（按链序遍历，取第一个可用者）：
1. 冷却中（`cooldown_until > now`）→ 不可用；
2. 配额视图剩余% < `failover_threshold_percent` → 不可用（主动切换）。配额视图 =
   min(快照 `min_remaining_percent()`，手填日限剩余比)。**无快照 ≠ 不可用**（数据缺失不拦截请求）；
3. 其余 → 可用。

无任何可用者时的降级：取第一个**不在冷却中**者；全部冷却 → 429 报给客户端。
被动结果回写 `on_result`：
- 429/402（配额类）→ 冷却 `Retry-After` 或 `error_cooldown_secs`，reason=Quota；
- 401/403（凭据类）→ 冷却 `error_cooldown_secs`，reason=Auth；
- 5xx/网络错 → 冷却 30s（防打死去上游），reason=Error；
- 2xx → 清 last_error 与 Error 冷却。

fail-back 无需独立逻辑：选择恒按链序取第一个可用者，链头恢复（冷却到期/配额回升）
即自然切回；`failback_threshold_percent` 用于"链头处于主动切换区间但未冷却"时避免立即回切。
活跃候选变化（`last_used_index` 变更）→ emit `router-switched` + 系统通知（每变化一次）。

### 3.3 代理服务（router/server.rs + forward.rs）

- axum Router：`POST /v1/messages`（anthropic 面）、`POST /v1/chat/completions`（openai 面）、
  `GET /health`（无鉴权诊断）。`.layer(DefaultBodyLimit::max(64MB))`（axum 默认 2MB 会截断长上下文请求）。
- 鉴权：`Authorization: Bearer <token>` → 查路由表；未命中 401；`router.enabled == false` → 503 + 协议形错误体；路由 protocol 与路径不符 → 400。
- 上游 Client：独立构建，`connect_timeout(10s)` + `read_timeout(300s)`，无总超时（SSE 分钟级）。
- 请求改写：JSON body 的 `model` 字段替换为候选模型；头清洗（剥 authorization/x-api-key/host/
  content-length/accept-encoding/connection），注入上游凭据（anthropic: `x-api-key` + 保留/补默认
  `anthropic-version`；openai: `Authorization: Bearer`），透传 `anthropic-beta`/`accept`/`content-type`/`user-agent`。
- 重试边界：读到上游状态码即决策；一旦开始向客户端回写 body，不再换候选。
- 响应：透传 status + content-type；body 流式转发。流式响应包一层行扫描流
  （跨 chunk 缓冲、按 `data:` 行解析 usage：anthropic `message_start.input_tokens` +
  `message_delta.usage.output_tokens`；openai 终帧 `usage`），流结束时记账。
- 超时语义：连接 10s / 首字节由 read_timeout 兜底 / 流空闲 300s；客户端断开 → drop 取消上游。

### 3.4 记账与配额感知

- `storage::accumulate_usage_daily(source, date, model, input, cache_read, output)`：
  ON CONFLICT 累加，供路由器写 `router:<instance_id>` 行（D9 口径）。
- `storage::sum_usage_daily_today(source) -> f64`：手填日限的 used 来源 + anthropic 卡片日窗数据。
- `providers/anthropic.rs`：kind `anthropic`（固定 `https://api.anthropic.com`）+
  `anthropic_api`（自定义中转 base）。fetch_usage 无配额 API → 日窗 = 账本今日路由消耗 vs
  `plan_limit_tokens_daily`（cost_source=Estimated），供卡片与决策共用一套数据。
- 已知口径：手填日限的 used 只含**经路由**的流量（直连不可见，见 §3.6）。

### 3.5 状态暴露与 UI

- Rust：`get_router_status` 命令 → `{ health: {listening, port, bind_error}, routes: [{id, name,
  protocol, active_index, candidates: [{account, model, state(ok|cooldown|low_quota|error),
  cooldown_until, remaining_percent, last_error}]}] }`；`router-switched` 事件
  `{route_id, route_name, from, to, reason}`；保存设置时对空 token 路由自动补发 token。
- `settings_delta` 增加 `router_changed`（enabled/port/routes 任一变化）；port 变化 → 旧 server
  graceful shutdown 后重启；其余字段每请求现读，无需重启。
- 设置页"路由"pane：总开关、端口、双阈值、冷却秒数；路由列表（增删/命名/协议）+
  候选链编辑（账户下拉限 BearerKey 账户、模型输入、base_url 预填可改、上下移/删除）+
  每路由 token 显示/复制 + 候选实时状态徽标。
- 主界面 header 快速开关：状态点绿=服务中 / 灰=停用 / 琥珀=bind 失败；点击翻转
  `router.enabled` 走 save_settings。仅 dashboard 模式。

### 3.6 错误处理与已知限制

- 端口占用：bind 失败 → health=bind_error（UI 琥珀点），不影响监控主功能；改端口后重启即恢复。
- 全候选不可用：429 + 协议形错误体（含原因摘要）。
- 已知限制（有意不做 / v1 边界）：
  - 跨协议翻译不做；OpenAI Responses（Codex）端点不做；
  - 直连上游的流量不经路由器，手填日限看不见（文档明示，靠 provider 自报窗口弥补）；
  - `router:*` 伪来源会以原名出现在趋势/日历的来源序列里（后续可做显示名映射）；
  - AccessKeySecret 凭据账户（火山 AFP）不可作候选（签名 scheme 不兼容透传）；
  - 4xx（参数错误等）不切换直接透传，避免掩盖真实错误。

## 4. 测试计划

- 单测：decision 状态机（链序选择/冷却到期/主动阈值/降级/全冷却/manual limit 数学）、
  model 重写与头清洗、SSE usage 扫描（跨 chunk 边界）、token 补发、配置 round-trip。
- 集成（wiremock 模拟上游）：直通 200、SSE 透传 + 记账、429 → 换候选重试、401 冷却、
  全冷却 429、禁用 503、错误 token 401。
- 前端：svelte-check / vitest 全绿；真机验收见 plan 文档。
