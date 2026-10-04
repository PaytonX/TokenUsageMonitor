# TokenRouter — 本地路由代理实施计划

- 日期：2026-10-04（v2 重构：单路由 + 四层判定，见文末「v2 重构」）
- 原始日期：2026-10-04
- 状态：已实施（自动化验收完成；真机验收步骤见文末，待用户执行）
- 分支：`feat/token-router`
- Spec：`docs/superpowers/specs/2026-10-04-token-router-design.md`（D1-D9 决策记录）

## Goal

长任务运行中某 token plan 用尽时，工具（Claude Code / ZCode / 其他 OpenAI 兼容工具）
不再被配额墙打断：工具把 baseURL 指向 TokenRouter 本地代理
（`http://127.0.0.1:43211`），由它在多个 Provider 账户间自动切换（主模型 + 有序
备选路由链），任务无感继续。

## Architecture

```
工具(Claude Code/ZCode)                     ┌─ 候选1: account+model+base_url
   │  Authorization: Bearer tr_<route>      │  （配额快照 ∩ 手填日限）
   ▼                                        │
RouterCore(axum, 127.0.0.1:port) ─ 决策 ────┼─ 候选2 ...
   │  model 重写 + 头清洗 + 凭据注入        │
   ▼                                        └─ 候选N
上游(官方/中转站) ── usage 旁路扫描 ──> usage_daily(router:<account>)
```

- `src-tauri/src/router/config.rs`：`Settings.router`（enabled/port/阈值/路由表）+
  `generate_route_token`（保存时空 token 自动补发）。
- `src-tauri/src/router/decision.rs`：纯函数状态机。链序取第一个可用者（冷却未过
  / 剩余% < failover 阈值都算不可用）；全部不可用降级到第一个未冷却者；配额冷却
  到期后须恢复到 failback 阈值以上才回用（防抖）；fail-back 无需独立逻辑。
- `src-tauri/src/router/forward.rs`：头白名单清洗 + 上游凭据注入（anthropic:
  `x-api-key` / openai: Bearer）、model 字段重写、非流式 JSON usage 提取、
  SSE 行扫描器（跨 chunk 缓冲，anthropic `message_start`/`message_delta` 与
  openai 终帧两种语义，均累计覆盖）。
- `src-tauri/src/router/server.rs`：axum 路由（`/v1/messages`、
  `/v1/messages/count_tokens`、`/v1/chat/completions`、`/v1/models`、`/health`），
  token→路由鉴权，被动错误分级（429/402 配额冷却、401/403 凭据冷却、5xx 短冷却
  30s、其余 4xx 透传不切换），仅未向客户端发出字节前换候选重试，活跃候选变化
  emit `router-switched` + 系统通知。
- `src-tauri/src/router/mod.rs`：`RouterCore`（AppState.router）；独立上游 client
  （无总超时，connect 10s / read 300s，跟随用户出站代理）；只有端口变化才重启，
  enabled/路由表每请求现读；停用时监听保留、请求 503 + 原因。
- `src-tauri/src/providers/anthropic.rs`：`anthropic` kind（实验）。无配额 API，
  日窗 = 账本当日 `router:<id>` 消耗（used-only，quota=0，Kimi 月窗先例）。
- `storage.rs`：`accumulate_usage_daily`（`router:*` 键 ON CONFLICT 累加，与
  REPLACE 回放来源隔离）+ `sum_usage_daily_today`。
- 前端：`types.ts`/`api.ts` 路由类型与封装；`Settings.svelte` 新「路由」pane
  （总开关/端口/阈值/路由卡编辑器/token 复制/候选状态徽标）；`App.svelte` 头部
  ⇄ 快速开关（绿=服务中/灰=停用/红=端口异常，点击翻转 enabled 即时生效）。

## Tech Stack

新增依赖：`axum 0.8`（default-features=false, http1+json+tokio）、`futures-util`、
`bytes`。与既有 tokio/reqwest 同栈；hub.rs 的阻塞实现不承载 SSE，故未复用。

## Tasks（全部完成）

- [x] Task 1: spec 文档落盘并提交（`docs/superpowers/specs/2026-10-04-token-router-design.md`）
- [x] Task 2: 配置与状态骨架（settings/settings_delta/router config/AppState/IPC 注册）
- [x] Task 3: axum 代理 + 转发管道（SSE 双向、token 鉴权、禁用 503、64MB body limit）
- [x] Task 4: 决策状态机 + 被动 failover + 重试 + 冷却（12 个单测）
- [x] Task 5: 主动切换（快照 min_remaining_percent ∩ 手填日限、fail-back 防抖）
- [x] Task 6: 记账 ledger + router-switched 事件 + 系统通知
- [x] Task 7: providers/anthropic.rs（单 kind；`anthropic_api` 变体取消——中转差异由候选 base_url 承载）
- [x] Task 8: 前端（Settings 路由 pane + App 快速开关 + types/api/dev-mock）
- [x] Task 9: 集成测试 7 用例（axum mock 上游，端到端）
- [ ] Task 10: 真机验收（步骤见下，待用户执行）

## 实施中发现并修掉的缺陷

1. **windows-gnu 集成测试 exe 起不动（STATUS_ENTRYPOINT_NOT_FOUND）**：任何
   `tests/` 下的测试 exe 都缺 Common-Controls v6 manifest，loader 解析到
   comctl32 v5 缺 TaskDialogIndirect 入口。lib 测试早已由 `lib.rs::windows_test_manifest`
   + build.rs 的 windres 资源解决；集成测试需在自己的文件里补同款
   `#[link(name = "cargo_test_manifest_res")]` 块。
2. **集成测试静态 `LAST_MODEL` 互斥量被并行用例共享**：poison 后连锁导致其他
   用例 429。改为每 mock 实例持有自己的状态（`MockState`）。
3. **`mark_active` 曾用 `Option::replace` 的返回值判切换**：首次承接请求时旧值
   为 None 与「无切换」不可区分，会丢 initial 事件。改为显式捕获旧值。
4. **真机反馈（2026-10-04）**：①候选模型名改为可留空 = 透传工具原始模型名
   （原实现强制必填，且 sanitize 会把不完整的候选/路由**静默丢弃**——用户留空
   模型名后整条路由没存进去，token 自然没发）；②前端保存后回读后端落盘结果，
   补发的 token 才能在路由卡显示（原实现只回显自己发出去的入参，token 永远
   显示「保存后自动生成」占位）；③路由不再因候选未配全而整条丢弃（请求会得到
   明确报错），避免吞掉用户填了一半的内容；④设置窗口监听 settings-changed，
   主面板 ⇄ 快速开关与设置页编辑态互不覆盖；⑤模型名支持**从上游拉取选择**
   （`fetch_upstream_models`：GET {base}/v1/models，keyring 凭据注入，data[].id
   排序去重；前端 datalist + ▼ 按钮，按「账户|上游地址」缓存）。并明确用法口径：
   工具端模型名可任填（如 auto），实际模型由候选决定；候选留空才是透传模式。
   ⑥**404 (no body) 根因**：Cherry Studio 类客户端在 API 地址（不带 /v1）后直接
   拼接 /chat/completions 与 /models，此前只注册了带 /v1 的路径，未匹配路径落
   axum 默认 404 空 body。已注册无 /v1 别名路径，且 fallback 返回带可用端点
   指引的 404 JSON；⑦候选行网格改为 minmax(0,…) 全列可收缩——输入框固有最小
   宽度 + auto 最小尺寸把整行撑爆面板，字段溢出错位；⑧路由服务区新增**工具
   接入地址**一键复制（http://127.0.0.1:{port}）；⑨**按量付费的阈值切换补齐**：
   新增候选级 `monthly_cost_limit`（账户币种金额），对比 provider 快照月窗已用
   （余额差分账户本就统计），与 token plan 的窗口剩余%、手填日限共用同一
   failover 阈值——此前按量付费只有 402 被动兜底，主动切换恒不触发（月窗
   quota=0 → 剩余% 恒 100%）。阈值综合口径 = min(快照最紧窗口, 月消耗上限,
   手填日限) 三者取最小，任一低于阈值即主动绕开该候选。

## 验收标准

- cargo test 全绿（lib 274 + 集成 7）✅
- `npx svelte-check` 0 错 ✅；`npm test` 43 过 ✅；`npm run build` 成功 ✅
- 凭据零字面量：上游凭据一律 keyring；测试用运行时拼接的合成 key（Mimosa L3 扫描通过）✅
- UI harness 视觉验收（浏览器 820×780 / 400×680 实尺寸截图）✅
- 真机切换链路 ⏳（下节）

## 验收记录

- 2026-10-04 自动化部分：
  - `cargo test`：lib 274 通过 + `router_proxy` 集成 7 通过（直通记账 120 tokens、
    SSE 流式记账 190、429 failover 落候选 2、错误 token 401、停用 503 且监听保留、
    /v1/models 不记账、全冷却后不再打上游直接 429）。
  - 前端：svelte-check 0 错、vitest 43 过、vite build 成功。
  - 浏览器 harness（dev-mock）截图核验：设置「路由」pane 与主界面 ⇄ 开关布局
    正常（visual-judge 子代理供应商不可用，按协议降级为人工核验，两张图均 pass）。

### 真机验收步骤（待用户执行）

1. `npm run tauri:build`（带 custom-protocol）或 `npm run tauri:dev` 启动；停止旧实例后再构建。
2. 设置 → 账户：添加两个 API Key 型账户（如 Anthropic + MiniMax），填好凭据。
3. 设置 → 路由：启用代理，新建路由（协议选对），添加两个候选（账户、模型、
   base_url 预填检查），保存；从路由卡复制 token。
4. Claude Code：`ANTHROPIC_BASE_URL=http://127.0.0.1:43211`、
   `ANTHROPIC_AUTH_TOKEN=<路由 token>`；跑一个长任务。
5. 调低「主动切换阈值」到 99（或等候选 1 的配额耗尽）观察：请求自动落到候选 2、
   任务不中断、系统通知弹出、路由卡候选徽标变化、账本出现 `router:*` 消耗行。
6. 恢复候选 1 配额（阈值调回）观察自动切回；点主界面 ⇄ 关闭后工具请求应收到
   明确的 503 错误。

## 已知限制（有意不做）

- 跨协议翻译不做（OpenAI Responses / Codex 端点同）；路由链候选必须同协议。
- 直连上游的流量不经路由器，手填日限看不见（provider 自报窗口可弥补）。
- `router:*` 伪来源在趋势/日历视图以原名显示（后续可做显示名映射）。
- AccessKeySecret 凭据账户（火山 AFP）不可作候选（签名 scheme 不兼容透传）。
- 4xx（参数错误）不切换直接透传，避免掩盖真实错误。


---

## v2 重构（2026-10-04，交互原型确认后）

**触发**：真机使用反馈 + 三轮原型讨论。原 v1 存在三类问题：多路由/别名分派维护
成本高、「最紧窗口剩余 %」在大窗紧小窗富余时误切、切换阈值与切回逻辑参数含义不直观。

### 设计结论（决策 D5 / D10 重写）

- **单路由**（D10）：`routes` 只看第一条；旧配置平滑迁移。
- **四层判定**（D5）：
  1. 主动预警（可设阈值，`proactive_threshold_percent`，**0 = 关闭**）——以**最小
     （最短周期）在报窗口**的剩余 % 为准；手填日限/月上限视为对应周期窗口一并参与。
  2. 硬墙兜底（恒开）——任一窗口/上限打到 100% → 立即不可用。
  3. 被动观测（恒开）——限流/配额立即冷却换线；连接异常原地重试一次 + 跨请求
     `conn_breaker_count` 熔断；参数类 4xx 透传。
  4. 切回探视——资格判定（无窗口打满 + 最小窗口有余量）→ 下一个**真实请求**作探针
     → 失败按 ×2 退避（`probe_start_secs`）→ 达 `probe_max_attempts` 等 `reset_at`
     （拿不到则 15 分钟低频）；备用全不可用时退避中的主线路仍允许一试。

### 界面重构（v3 原型，docs/mockups/2026-10-04-token-router-ui-v3.html）

- 服务卡（总开关 + 接入地址一键复制 + 「服务设置」折叠）
- **切走线仪表**：可拖动阈值 + 当前承载线路的最小窗口剩余指针 + 状态文字
- **切回探视时间线**：第 N / 共 M 次 + 下次探视倒计时
- **线路卡**：序号圆点 ①②③（连接线）、账户/模型（▼ 拉取）、上游地址（自动带出标记）、
  额度上限折叠（日 tokens / 月金额）、↑↓✕、当前线路呼吸光晕
- 动效：卡片/线路入场 stagger、折叠高度过渡、开关弹性滑块、指针平滑滑动、
  `prefers-reduced-motion` 适配

### 实施结果

- 后端：`config.rs` / `decision.rs` / `mod.rs` / `server.rs` 全量重写
  （decision.rs 新增 `assess` / `QuotaVerdict` / `ProbeState` / 熔断计数）。
- 集成测试：14 个用例（新增主动预警提前绕行、月限窗口、探视排程、route.off 503）。
- 验证：`cargo test` lib 283 + 集成 14 全绿；`svelte-check` 0 错；`vitest` 43 过；
  `vite build` 成功；浏览器 harness 实尺寸截图核验（服务卡/仪表/探视时间线/线路卡）。
- 提交：`b56c870`（后端）、`f703cb2`（前端）。

### v1 → v2 配置迁移

| v1 字段 | v2 |
|---|---|
| `failover_threshold_percent` | 废弃（serde 忽略）→ `proactive_threshold_percent`（同名语义但按最小窗口） |
| `failback_threshold_percent` | 废弃 → 由探视退避取代 |
| `error_cooldown_secs` | 保留 |
| — | 新增 `conn_breaker_count` / `probe_start_secs` / `probe_max_attempts` |
| `routes[]`（多条） | 只取第一条；`RouteConfig` 新增 `on`（路由链开关） |
