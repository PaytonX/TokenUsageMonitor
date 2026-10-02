# 用量口径与统一账本（usage_daily）

> 2026-10 重构。本文回答一个问题：**日历图、趋势图、模型页的数字从哪来、为什么一致。**

## 唯一口径：本机工具的真实 token 消耗

三个历史视图（总览页日历热力图、趋势页、模型页）共用同一张 SQLite 表
`usage_daily`，数据由**本机工具日志扫描**写入——Claude Code / ZCode /
MiniMax Code / Cherry Studio / Hermes / Codex / DeepSeek Harness 的会话记录。
扫描直接读工具自己的历史库，与 TokenUsageMonitor 是否在运行无关，因此
"昨天用了但今天打开是 0" 这类问题在结构上不再可能发生。

- 单位：tokens（input / cache_read / output / total 四分量）。
- 键：`source`（工具 id）× `date`（本地日）× `model`（空串 = 工具总量行）。
- 写语义：全量回放 + 逐行 REPLACE，幂等。重扫两次值不变；滚动窗口（90 天）
  外的旧行随回放自然清除。
- 成本：只有实报成本的来源（Cherry Studio 等）计入，按日占比分摊到模型行；
  全局估算开关关闭时不产生估算成本。

## provider 数据的角色边界

provider 轮询回答的是**账户额度状态**（卡片圆环、进度条、告警），不是用量历史：

| 类别 | provider | 日历史处理 |
|---|---|---|
| 有真实服务端日账 | 火山（tokens）、OpenAI / xAI（USD 成本） | 双写进账本（`kind='provider'`），日历区可单独选中查看 |
| 差分类（窗口/余额差分） | MiniMax（percent）、DeepSeek / Kimi（cny）、OpenCode（percent） | **不进账本、不进历史视图**——它们只在 TUM 运行时按采样差分累计，既不完整也不可回溯，混入历史视图曾导致"模型页有数、日历为 0"的不一致 |

差分类数据的持续用途：卡片实时窗口（5h/周/月进度）、燃烧率基线、阈值告警。

## 数据流

```
本机工具日志/库 ──扫描──▶ LocalToolsPayload ──persist_all_tools──▶ usage_daily
                                                            │
     火山 GetUsageDetails ─┐                                │
     OpenAI daily_costs ───┼─双写───────────────────────────┘
     xAI usage ────────────┘
                                                            ▼
                              get_usage_history(days) ──▶ 日历图 / 趋势页 / 模型页
                              （三视图同源，同一天数字严格一致）
```

## 与旧实现的差异（历史背景）

- 旧 `daily_snapshots` 表仍是 provider 卡片热力图的存储（火山/OpenAI/xAI），
  但**不再是历史视图的口径来源**。
- 旧「MiniMax 卡片热力图优先读 minimax-code 键 + 1 天新鲜度闸门」已退役：
  账本永远有数，闸门失去存在理由。`minimax-code` 旧键在首次启动时迁移入账本。
- 旧趋势图把 percent / tokens / cny / usd 四种单位直接相加画线——这是单位
  治理前的错误口径，已随换源消失。

## 多端汇总模式

设置 →「页签显示」→「全端汇总模式」开启后，趋势/工具/模型页显示**全部设备**
的合并用量（设备报告携带的分工具/分模型序列，按设备去重、跨设备求和）。
账本（本机）与设备序列形状一致（LocalDay 四分量），两种模式仅范围不同。
