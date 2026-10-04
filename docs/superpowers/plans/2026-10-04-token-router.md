# TokenRouter — 本地路由代理实施计划

- 日期：2026-10-04
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
   宽度 + auto 最小尺寸把整行撑爆面板，字段溢出错位。

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
