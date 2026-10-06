# Benchmark：token-monitor + tokscale 调研与落地计划

> 状态：调研结论定稿 / 落地计划（只方案，未实现）
> 日期：2026-09-23
> 目标：评估开源项目 token-monitor 与其 upstream 引擎 tokscale，提炼可借鉴的前后端设计与功能，形成本项目（TokenUsageMonitor）的落地 roadmap。

---

## Part A · 调研结论

### A.1 开源项目概览

| | token-monitor | tokscale (upstream) |
|---|---|---|
| 仓库 | Javis603/token-monitor | junhoyeo/tokscale |
| Stars | ~2295 | ~5512 |
| 语言 | JavaScript (Electron) | Rust |
| 协议 | MIT | MIT |
| 定位 | 跨 37+ AI 编码工具的本地优先用量看板 widget + 多端同步 | tokscale 的引擎：终端 CLI / TUI 用量 dashboard |
| 数据源 | 各工具本地日志/SQLite + 部分 API | 各工具本地日志（70+ 工具） |
| 关系 | **依赖 tokscale** 做日志解析（vendor 二进制） | 独立 CLI，被 token-monitor 消费 |

**核心差异**：token-monitor 追踪"AI 工具在本机消耗的 token/cost"，数据源是本地日志；我们追踪"API 提供方账户的配额/余额"，数据源是各提供方 API。两者数据源互补。

### A.2 token-monitor 值得借鉴的方面

**架构**
1. `src/shared/` 单一共享库 + 多入口（widget / hub / agent / Cloudflare Worker）复用，CI 校验无漂移
2. Provider 按 `providers/<id>/` 一目录一集成（Feature-oriented）
3. Usage 采集 与 Limits 限额 双运行时解耦
4. 文件系统 watch + 增量分片扫描（3-5s 更新）+ watcher worker 线程隔离
5. adaptive 自适应限额轮询 + 智能熔断

**前端交互**
6. 多视图 dashboard（home/limits/tools/session/model/device）、可拖拽排序
7. 成本多币种换算（USD/TWD/HKD/CNY，汇率每日更新）
8. 会话级/项目级明细、缓存命中统计、年度热力图+streak+趋势
9. macOS 原生 widget / 悬浮气泡 / edge dock；冷启动预览；诊断面板
10. 公开 profile + SVG embed + leaderboard + Wrapped 年度回顾

**工程规范**：`AGENTS.md` 作为单一事实源，详尽文档化架构约定与 invariant。

### A.3 tokscale 值得借鉴的方面

技术栈与我们（Tauri+Rust）高度重合：reqwest、rusqlite、chrono/chrono-tz、tokio、clap、serde。

**1) `clients.rs`：声明式客户端注册表（宏驱动）**
每工具一行配置（id/display/logo/root/relative_path/pattern），宏自动生成 `ClientId` 枚举 + `CLIENTS` 数组 + 显示名/logo 数组，编译期断言索引连续。扩展新工具 = 加一行配置，无需改 match。

**2) `parser.rs`：通用容错解析引擎**
`parse_json_file` / `parse_jsonl_file`，simd-json 加速，逐行容错（坏行跳过不中断），完整单测。

**3) `sessions/<client>.rs`：per-工具归一化为 UnifiedMessage**
50+ 解析器（claudecode/codex/copilot/kimi/qwen/cursor...），全部输出统一的 `UnifiedMessage`。核心结构：

```rust
pub enum CostSource { Unknown, ProviderReported, Estimated }

pub struct UnifiedMessage {
    client, model_id, provider_id, session_id,
    workspace_key/label, timestamp, date,
    tokens: TokenBreakdown,   // input/output/cache
    cost: f64,
    cost_source: CostSource,  // 成本来源标注
    duration_ms, message_count, agent, dedup_key,
    session_title, is_turn_start,
    model_attribution_conflicted, // 归属冲突则拒计价
}
```

**4) `pricing/`：多来源价格服务**
`mod.rs`（PricingService 单例）、`lookup.rs`（compute_cost）、`aliases.rs`（模型别名）、`models_dev.rs`/`litellm.rs`/`openrouter.rs`（多价格源）、`custom.rs`/`self_hosted.rs`（自定义）、`fetch.rs`/`cache.rs`（拉取缓存）。
关键机制：provider 前缀分类（openai/anthropic/deepseek/minimax/qwen...）+ 别名归一 + 裸品牌词/通用词**禁止模糊匹配**（防止误计价）+ 订阅 $0 排除（LiteLLM github_copilot 前缀）。

### A.4 已定的借鉴落地项（承接后续落地计划）

| 编号 | 借鉴来源 | 落地项 |
|------|---------|--------|
| B1 | tokscale `clients.rs` | 后端 Provider 注册表收敛（手写 match → 静态表） |
| B2 | tokscale `UnifiedMessage.CostSource` | `UsageSnapshot` 增加成本来源标注 |
| B3 | tokscale `TokenBreakdown` | 增加 input/output/cache token 拆分与 `model_id` |
| B4 | tokscale `pricing/` | token→成本换算（多源价格表 + 别名 + 防误判） |
| B5 | token-monitor 多视图/趋势 | 前端 dashboard 多维视图 + 趋势看板 |
| B6 | token-monitor 成本换算 | 前端多币种展示 + 汇率更新 |
| B7 | token-monitor 目录采集 | 本地工具日志采集通道（可引 tokscale） |
| B8 | token-monitor 多端同步 | hub 同步（远期） |

---

## Part B · 前端与后端落地实施方案

### B.0 总体方向

在现有"API 配额/余额监控"之上，**叠加"本地工具用量采集"与"成本换算"两个差异化能力**，并把 Provider 注册、snapshot 模型升级到位。分三期推进，先做低风险、高收益的结构改造，再做体验增强。

---

### B.1 [后端] Provider 注册表收敛（toB1，先做）

**现状痛点**：新增 provider 需改 `src-tauri/src/lib.rs` 的 `build_account_provider` match + `mod.rs` PRESETS + `types.ts` SHORT_KIND_NAMES，三处手动同步，易漏。

**方案**：把 `kind → Provider 构造器` 收敛为一张静态注册表。

- 在 `providers/mod.rs` 新增：
```rust
// 统一构造签名（现有 provider 均已满足）
pub type BuildProvider =
    fn(Client, Arc<Storage>, String, String) -> Arc<dyn Provider>;

pub static PROVIDER_REGISTRY: &[(&'static str, BuildProvider)] = &[
    ("minimax",        |h,s,i,l| Arc::new(minimax::MiniMaxProvider::new(h,s,i,l))),
    ("minimax_api",    |h,s,i,l| Arc::new(minimax_api::MiniMaxApiProvider::new(h,s,i,l))),
    ("deepseek",       |h,s,i,l| Arc::new(deepseek::DeepSeekProvider::new(h,s,i,l))),
    ("volcengine",     |h,s,i,l| Arc::new(volcengine::VolcengineProvider::new(h,s,i,l))),
    ("volcengine_api", |h,s,i,l| Arc::new(volcengine_api::VolcengineApiProvider::new(h,s,i,l))),
    ("openai",         |h,s,i,l| Arc::new(openai::OpenAIProvider::new(h,s,i,l))),
    ("xiaomi_plan",    |h,s,i,l| Arc::new(xiaomi::XiaoMiPlanProvider::new(h,s,i,l))),
    ("xiaomi_api",     |h,s,i,l| Arc::new(xiaomi::XiaoMiApiProvider::new(h,s,i,l))),
];
```
- `build_account_provider` 改为查表，删除 match：
```rust
pub fn build_account_provider(account, http, storage) -> Option<Arc<dyn Provider>> {
    PROVIDER_REGISTRY
        .iter()
        .find(|(k, _)| *k == account.provider_kind)
        .map(|(_, f)| f(http, storage, account.instance_id.clone(), account.label.clone()))
}
```
- `accent_for` 沿用现有遍历（已覆盖子模式）。
- **效果**：新增 provider = 加一行注册表 + 一行 PRESETS；前端只加 SHORT_KIND_NAMES；三处已文本对齐到"驱动同一份 kind 列表"。可选进一步：由注册表的 kind 反向生成 PRESETS，彻底单一事实源（列入可选优化）。

**验证**：`cargo check`、`npx svelte-check`、`cargo tauri dev` 下现有全部 provider 添加/查询/删除行为不变。

---

### B.2 [后端] UsageSnapshot 增加成本来源标注（toB2，先做）

**方案**：`mod.rs` 新增枚举 + `UsageWindows`/`WindowUsage` 增字段。

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostSource { #[default] Unknown, ProviderReported, Estimated }
```
- `WindowUsage` 增加 `#[serde(default)] pub cost_source: CostSource;`
- 现有各 provider fill window 时标注：OpenAI 每日 USD（官方返回）→ `ProviderReported`；DeepSeek CNY 余额推算 → `ProviderReported`；估算成本/占位 → `Estimated`。
- 前端透传，`types.ts` 同步 `CostSource` 类型与 `WindowUsage.cost_source`。

**效果**：为 B4 成本换算的"来源可信度"打基础，前端可显示"估算"标签而非误导为真值。

---

### B.3 [后端] TokenBreakdown 扩展（toB3，中期）

**方案**：新增统一的 token 明细类型（对齐 tokscale `TokenBreakdown`）：
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenBreakdown {
    pub input: f64,          // 含 cache_read
    pub cache_read: f64,     // 缓存命中输入
    pub output: f64,
    pub model_id: Option<String>,
}
```
- `WindowUsage` 增可选 `#[serde(skip_serializing_if = "Option::is_none")] pub tokens: Option<TokenBreakdown>`。
- 现有 provider（如 OpenAI `usage` 字段、火山 GetInferenceUsage 的 Input/OutputTokens）在拿到明细时填充。
- 前端类型同步；ProviderCard/DetailCard 在存在明细时展示缓存命中比。

**效果**：支撑"缓存命中统计"与"token→成本"（B4 需 input/output/cache 单价差）。

---

### B.4 [后端] token→成本换算（toB4，中期，依赖 B3）

**方案**：引入轻量价格服务（借鉴 tokscale `pricing/`，不必照搬全量）。

- 新增 `src-tauri/src/pricing.rs`（或 `pricing/` 模块）：
```rust
pub struct PricingEntry { input_per_1m: f64, output_per_1m: f64, currency: &'static str }
```
- 一张**静态表**覆盖常用模型（claude-*/gpt-*/deepseek-*/minimax-*/qwen-*/mimo-*…），值为每百万 token 单价；配 `aliases` 做模型别名归一。
- `compute_cost(breakdown, model_id) -> Option<f64>`：按 input×input价 + output×output价（cache_read 用 input 或单独价）。
- **防误判**：裸品牌词/无模型信息禁止匹配；订阅/TokenPlan 模型不参与成本换算。
- cost 存入 `window.cost`（f64，可选用 `UsageUnit::Usd`/`Cny` 表达币种），`cost_source=Estimated`。
- CLI 侧：`MixedMode` UI 或单测中验证。多来源（LiteLLM/openrouter）拉取列为远期可选。

---

### B.5 [前端] 多维视图 + 趋势看板（toB5，中后期）

**方案**：在现有"provider 卡片流 + 热力图"之上扩展 dashboard 视图。

- 新增视图切入口（top 或侧 tab）：`总量 / 限额 / 工具 / 会话 / 模型 / 设备`（对齐 token-monitor）。
- 工具视图：展示本地采集到的各 AI 工具用量（依赖 B7）。模型视图：各模型跨 provider 聚合。设备视图：多端（依赖 B8）。
- 趋势看板：年度热力图 + streak + 日趋势（堆叠/柱状/K线），按 tool/model 切换；固定区间（本周/近7天/近30天）按钮。
- 沿用现有 tokens.css 设计令牌与 `ProgressRing`/`ProviderCard` 视觉语言，避免风格割裂。

**效果**：从"按 provider 一屏"升级为"按任意维度下钻"。

---

### B.6 [前端] 成本多币种展示（toB6，中期，依赖 B4）

**方案**：前端新增币种换算。

- `lib/currency.ts`：汇率表（USD→CNY/TWD/HKD…），每日拉取 + 手动覆盖（persist 到 settings）。
- `formatUsage`/新 `formatCost`：按选定币种渲染成本。
- ProviderCard/DetailCard/趋势看板展示 cost（`cost_source` 为 Estimated 时附"估算"角标）。
- 设置页新增"偏好币种 / 汇率覆盖"。

---

### B.7 [后端+前端] 本地工具日志采集通道（toB7，中期，可选引 tokscale）

**方案**：新增"本地用量"数据源，与现有 API 查询并行的第二通道。

- 方案 A：引 tokscale 为 Rust 依赖，复用 `clients.rs`（声明式 client）/`scanner`/`parser`/`sessions`/`pricing`。
- 方案 B：仅取其设计模式，自己实现读少数主流工具（Claude Code / Codex / Kimi / Qwen / Cursor）的本地日志，归一为 `UsageSnapshot` 或新的 `LocalUsageSnapshot`。
- 新增 provider kind 空间（如 `local:claude`、`local:codex`）或独立"工具"数据区。
- 前端工具视图消费（B5）。

**注意**：A 方案需评估 tokscale 打包体积、平台（rusqlite bundled/reqwest）与我们构建链兼容性；推荐先 B 做 1-2 个工具验证。

---

### B.8 [后端+前端] 多端同步 hub（toB8，远期）

**方案**：借鉴 token-monitor 的 hub/agent 模型。

- 新增 `src-tauri/src/hub/`：Node/Rust HTTP hub + `/api/ingest`、`/api/stats`、SSE `/api/stats/stream`；设备定期上报本机汇总。
- 可选 Cloudflare Worker 兼容。
- 前端设备视图（B5）+ 同步状态展示。

---

## Part C · 分阶段实施计划

### 阶段一：结构底座（低风险，先做，各自独立可合入）
- [ ] C1. B1 后端 Provider 注册表收敛（match → 静态表）
- [ ] C2. B2 `UsageSnapshot` 增加 `cost_source` 标注 + 前端类型同步
- [ ] C3. 回归验证：`cargo check` + `svelte-check` + dev 全量 provider 行为不变

### 阶段二：成本能力（中期）
- [ ] C4. B3 `TokenBreakdown`（input/cache/output + model_id）接入现有能拿明细的 provider
- [ ] C5. B4 后端定价模块（静态价格表 + 别名 + compute_cost）
- [ ] C6. B6 前端币种换算 + 成本展示 + "估算"角标

### 阶段三：体验增强与数据源扩展（中后期）
- [ ] C7. B5 前端多维视图 + 趋势看板
- [ ] C8. B7 本地工具日志采集（先 1-2 工具验证，可选引 tokscale）

### 阶段四：多端（远期）
- [ ] C9. B8 hub 同步 / Cloudflare Worker / 设备视图

---

## Part D · 验证基线（每一阶段合入前必须通过）

- 后端：`cargo check` 0 error；`cargo test`（沿用现有单测风格，涉及新解析补测试）
- 前端：`npx svelte-check` 0 error；`npm run build` 通过
- 运行时：`npx tauri dev` 下验证——现有 provider 添加/删除/查询/设置不受影响；新能力按阶段可用
- 设计一致性：新增 UI 遵循 `docs` 中 Fluent Glass 设计令牌（颜色/间距/圆角/字体）与现有组件语言

## Part E · 风险与注意

1. **直接引 tokscale**：打包体积（vendored 二进制）与平台兼容性需评估；其 rustls/rusqlite 特性与我们构建链需联调。
2. **成本估算准确度**：`Estimated` 成本受模型价表覆盖与别名影响，须用 `cost_source` 明确标注不可当真值。
3. **多源/多币种**：汇率与价格需可离线、可覆盖，避免网络依赖破坏本机监控。
4. **本地采集合规**：读本地工具日志不读消息内容（tokscale 明确"不读消息内容"），仅取 usage/元数据，保护隐私。