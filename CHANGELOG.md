# 更新日志 / Changelog

本项目的变更记录。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

发布新版本时更新本文件，并把对应内容打上 `v*` tag 触发 Release 工作流。

## [未发布] / Unreleased

<!--
在此累积尚未发布的变更。新增条目请用下面的小节标题，PR 合并后由维护者归并。

### 新增
### 变更
### 修复
### 文档
-->

## [0.3.0] - 2026-10-07

多语言支持（zh-CN / en）、趋势弹窗日历改版、MiSans 可变字体接入，
以及 TokenRouter 归因与账本口径修正。

### 新增

- 界面多语言支持：前端 zh-CN / en 全量迁移，语言切换做成设置项，英文界面下导航短词不再被裁切
- 全局字体接入 MiSans 可变字体，中英同族，消除雅黑/宋体混排回退
- 趋势弹窗日历区改版：58% 宽日历 + 右侧统计栏，趋势窗口尺寸调整为 1080×660

### 变更

- TokenRouter 记账记实际承载模型：候选指定了模型就用它，透传（候选未指定模型）则取工具请求里的原始模型名，不再一律写空串
- 路由自记账的 `usage_daily.kind` 由 `'provider'` 独立为 `'router'`：它不是服务端日账，混在同一 kind 里会被前端的 provider 日账分支误当官方值做「替换本机归因」
- 手填日限与 Anthropic 卡片日窗的 used 口径改为对当日全部模型行求和（此前只认 `model=''` 总量行，路由开始记模型后会归零导致日限失效）
- 经本地路由的用量在模型页/日历/Provider 趋势里改用**路由台账的真实模型**归因：工具侧只认得 `custom_provider:*` / `LocalRouter` 这类占位名，打成哨兵 `__via_router__`，前端按日做替换；路由台账覆盖不足时差额回退工具兜底，覆盖超出时按序截断，两侧都不丢量也不虚增
- Codex 会话级模型回填：用量事件（`token_usage_record`）本身不带模型，改为从同文件 `turn_context` 事件取会话模型回填，避免 Codex 全部用量落「未标记模型」

### 修复

- MiniMax Code 按模型归因回退：上游 2026-08 起不再往 token 账本写 `model`，导致该工具全部用量落入"未标记模型"桶。现按 `session_id` 关联 `local_runtime_sessions.extra_data_json` 的 `effectiveModel` 恢复模型名，账本列优先于会话推断，两者皆无才落未标记桶
- Provider 跨工具趋势图双计：来源总量行（`model=''`）此前与分模型行一起累加，趋势值约为真实值的两倍。现总量行不再进入 Provider 序列
- 热力图日历单网格方案废弃：回到嵌套结构，星期轴标签改 `flex:1` 与格子共享行轨对位
- 停靠把手禁用下边缘收起：贴底把手与 Windows 任务栏交互异常

### 测试

- `local::roots` 测试串行化：5 个用例共享进程级 `CONFIG` 快照，cargo 并行执行时互相清空配置，实测约 2/3 概率失败
- `router_proxy` 集成测试端口选取改为绑定失败重试：`free_port()` 是 TOCTOU 的（读完端口即释放监听），并行用例可能挑到同一端口导致随机失败

## [0.2.0] - 2026-10-07

玻璃材质详情卡、双窗口原子化、设计令牌门禁与可达性修复。

### 新增

- 悬浮详情卡兑现玻璃材质：`blur(var(--tum-blur-card))` + 半透明深底
- 增补 `warn-fill` / `warn-stroke` / `crit-stroke` 状态表面令牌
- 路由状态点改用 ok/warn/crit 状态令牌，并新增设计令牌 lint 测试守护

### 变更

- 面板与窗口迁移到视觉原子（`PanelHeader` / `RangePills` / `ZoomButton` / `Stat` / `ColorSwatch`）
- 趋势 / 工具窗口标题区接入 `PanelHeader`，工具选择器改用 `PillsOrSelect`
- 值保持型字号 / 圆角 / 白面字面量替换为设计令牌
- `deviceColor` 收敛为共享实现；系列色提取为共享常量，消除面板 / 窗口双份定义
- 状态色收敛到 warn/crit 令牌，去除裸 hex 与偏色变体
- rgba 家族禁令补全，兜底灰统一 `rgb` 写法，弹层与文字外来色接入令牌

### 修复

- `RangePills` 补键盘焦点描边（可达性）
- `PillsOrSelect` 支持 `label` prop，工具选择器读屏标签恢复为「选择工具」（可达性）

### 测试

- 设计令牌 lint 加固：模块锚定路径、精确豁免匹配、扫描覆盖哨兵
- 锁定 `hexToRgb` 对 hex / `rgb()` / `rgba()` 三种输入的解析行为

## [0.1.0] - 2026-10-06

首个可发布版本。覆盖多 Provider 额度监控、本机 AI 工具用量账本、
TokenRouter 本地路由代理、多端同步（P2P）与 Windows 透明置顶挂件形态。

### 新增

**多 Provider 额度监控**

- DeepSeek、Kimi (Moonshot)、MiniMax、火山方舟、OpenAI、xAI、
  ChatGPT / Codex、openCode Zen 等服务商的额度 / 余额 / 订阅用量接入
- 同一服务商支持多独立账户，每账户可配标签、强调色与启停开关

**本机 AI 工具用量账本**

- 扫描本机会话日志，汇入统一日账本：Claude Code、Codex、ZCode、
  DeepSeek Harness (dsh)、Cherry Studio、MiniMax Code、Hermes
- 支持工具数据目录重定位（额外根目录 + 新鲜度优先解析），避免僵尸副本重复计数
- WSL 发行版内的工具日志一并纳入扫描

**TokenRouter 本地模型路由代理**

- 仅监听 `127.0.0.1:43211` 的反向代理，兼容 Anthropic 与 OpenAI 两类协议
  （`/v1/messages`、`/v1/chat/completions`、`/v1/responses`）
- 单路由 + 四层判定：最小余量窗口主动预警 → 硬墙 → 被动观测（429/404）→ 恢复探视切回
- 工具自身凭据绝不透传给上游，路由器注入候选账户自己的 Key
- 响应旁路记账：经路由的每个请求用量同样进入统一账本
- v2 路由界面：服务卡 + 切走线仪表 + 探视时间线 + 线路卡与动效
- 工具接入地址一键复制；模型名支持从上游拉取选择
- AK/SK 账户支持可选推理 API Key（火山 AgentPlan 可作为路由线路）

**多端同步（P2P）**

- mDNS 设备发现与设备页管理
- 趋势 / 工具 / 模型页签支持全端汇总模式
- 设备卡片、共享密钥鉴权、每 30s 上报

**桌面形态**

- 400×680 透明置顶挂件，四边贴边胶囊 + 把手，落点判定统一到 Rust
- 日历 / 趋势 / 模型 / 工具多视图，热力图与账本联动
- 托盘常驻、开机自启、单实例

**工程**

- Tauri IPC 开发期桩：真实组件可在浏览器中定尺预览
- CI（前端 vitest + svelte-check，Rust GNU 工具链 cargo test）与 Release 工作流
- 开源准备：双许可、中英双语 README、SECURITY、CONTRIBUTING、
  Issue/PR 模板、设计令牌 lint

### 修复

- Codex 新版 `token_count` 事件用量解析
- upstream URL 版本段自适应；404 参与换线（方舟 Plan 404 根因）
- 修改凭据时回填已存值——补可选路由 Key 不再被迫重填 AK/SK
- 注册不带 `/v1` 的路径别名；404 带指引 body；候选行网格可收缩防溢出
- 模型名可留空透传；保存后回读补发 token；不再静默丢弃未配全的路由
- 换模式时按锚点重算位置，修掉贴边往返后掉出贴边态
- 贴边把手几何泛化到四边；修上边把手黑砖、贴边态启动丢失、把手与胶囊错位

### 测试

- TokenRouter 端到端集成测试：直通记账 / SSE / 429 failover / 鉴权 /
  停用 503 / 全冷却

[未发布]: https://github.com/PaytonX/TokenUsageMonitor/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/PaytonX/TokenUsageMonitor/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/PaytonX/TokenUsageMonitor/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/PaytonX/TokenUsageMonitor/releases/tag/v0.1.0
