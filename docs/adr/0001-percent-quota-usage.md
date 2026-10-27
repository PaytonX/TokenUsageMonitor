# ADR-0001：百分比额度用量进入账本与账户额度详情

- 状态：已接受
- 日期：2026-10-24
- 关联：`docs/usage-ledger.md`、`src/lib/quota-usage.ts`、`src/lib/trend-data.ts`、
  `src/lib/calendar-linkage.ts`、`src-tauri/src/providers/opencode.rs`、
  `src-tauri/src/storage.rs`

## 背景（Context）

### 一类 Provider 只报百分比

OpenCode Go、MiniMax Token Plan 这类订阅制服务的用量接口只回「已用额度百分比」
（`src-tauri/src/providers/opencode.rs:80-83` 的 `percent: Option<f64>`，经
`window()` 钳到 `0..=100`），既没有 token 分量，也没有绝对金额。它们在 TUM 里
天然没有可加的量纲。

### 三个历史视图只读一张表

总览页日历热力图、趋势页、模型页共用 `usage_daily`（见
[用量口径与统一账本](../usage-ledger.md)）。百分比制 Provider 此前只写
`daily_snapshots`（卡片热力图），与 `usage_daily` 无关，于是它们在统计页
**没有任何入口**——用户看到的「OpenCode Go 用量从不出现在趋势页」不是数据缺失，
是这条链路从来没接到账本上。

### 卡片还存在第二个独立缺陷

`src/App.svelte` 旧代码在设置焦点前先判 `if (!ledgerSeriesForKind(...)) return`，
而 `KIND_TO_PROVIDER_KEY` 里没有 `opencode` / `codex`。于是这类卡片**点不动**：
没有焦点圆环、顶棚读数不变、日历也没有反应。点击被静默吞掉，用户读到的是
「卡片坏了」。现在 `toggleCardFocus`（`src/lib/calendar-linkage.ts:112-124`）
把两件事拆开：「能否被选中」只看 `provider_id`，「能否高亮日历」才看有无序列，
没有序列就置 `highlightKey = null`——不猜，也不阻止选中。

### 两个记账 bug

1. **跨分母求和。** 早期实现把滚动 5 小时 / 周 / 月三个窗口的百分点增量相加。
   三个窗口各有自己的额度分母，同一笔消费在每个窗口里各涨一次，日值轻易超过
   100%，既不可解释也不可比。
2. **峰值合并写日值。** 日值写入用的是 `Storage::record_daily_on`
   （`src-tauri/src/storage.rs:181-206`，`ON CONFLICT … DO UPDATE SET value =
   MAX(value, excluded.value)`）。这个语义是为 MiniMax「同一窗口关闭日取峰值」
   设计的；而这里每次轮询算出来的是**一次 30 秒的增量**，取峰值等于把一天压成
   「一天里最热的 30 秒」。

### 多 Key 的统计口径问题

一个 API Key 在 TUM 里就是一个账户（账户 id = 账本 `source`）。同一份
OpenCode 订阅下可以签发多个 Key，它们的服务端额度**是同一份**。于是「这个 provider
整体用了多少」必须回答两件事：哪些 Key 属于同一份额度，以及多份额度之间怎么合并。

## 决策（Decision）

### D1 只有月窗口是可加口径，写入是累加

按日序列只由**月度窗口**的差分累加而成（`daily_increment`，
`src-tauri/src/providers/opencode.rs:188-201`）：月度窗口在计费周期内单调不减，
逐日差分之和即「本月已用百分比」（完整观察一个周期后成立；首次观察只建基线，
`window_delta` 返回 0，最多丢一个轮询周期），是唯一自洽的可加口径。滚动 5 小时与
周窗口降级为**纯展示**（卡片进度条），不参与记账，也不再保留基线（`KV_MONTHLY`
只存月度一条，旧版本写下的 `oc_go_rolling` / `oc_go_weekly` 留在库里但不再读取）。

日值写入改为 `Storage::accumulate_daily`（`src-tauri/src/storage.rs:155-174`，
`DO UPDATE SET value = value + excluded.value`），即一次次 30 秒增量累加到当天。

### D2 百分比日值双写进账本，但永不与 token 相加

同一串日值按 volcengine / openai / xai 的模式写进 `usage_daily`
（`kind='provider'`、`unit='percent'`，整段 REPLACE 幂等，
`src-tauri/src/providers/opencode.rs:327-354`）。这是它能被历史视图看见的**唯一**
理由——视图只读这张表。

单位隔离不加「例外清单」，而是**按单位过滤**：token 序列只吸收
`isTokenUnit(unit)`（`unit === "tokens" || unit === ""`，
`src/lib/calendar-linkage.ts:235-236`，在 `:283` 处过滤 provider 行）。
percent 行因此永远进不了日历/模型页的 token 累加，也不会出现 100% 和 1.2M token
被加在同一个数上的情况。

### D3 账户卡片详情是 percent 历史上唯一的呈现位置

新增前端模块 `src/lib/quota-usage.ts`：纯聚合、不读后端，判据是 `unit === "percent"`
（`isPercentUnit`，`src/lib/quota-usage.ts:31-33`），不依赖 provider 白名单——
新接入的百分比制 Provider 自动进入额度详情。数据源 `fetchQuotaGroups` 与 token 序列
**同源**（同一份 `usage_daily`，`src/lib/trend-data.ts:176-193`），区别只是它按
kind 汇总到「账户级额度序列」，而不是按模型拆成 token 分量。

额度渲染在**账户卡片的悬浮详情**（`src/lib/components/DetailCard.svelte` 的 percent
分支）：本月已用、今日增量、有效样本天数，多 Key/多订阅时追加一行
「N 个 Key / M 个订阅」。它不占趋势页版面——趋势页是纯 token 页面。

> 2026-10-24 修正（方案 C）：最初把额度做成趋势页里的一条独立轨道
> （`QuotaLane.svelte`，已删除）。用户实测后指出两个问题：轨道与当前焦点无关
> （切换 provider 时它不动），且本机只有 OpenCode 一处 percent 来源，看起来像
> 「opencode 被特殊照顾」。轨道因此被移除，额度收进它真正归属的那张卡片详情。

### D4 多 Key = 订阅指纹去重 + 容量归一均值

- **判据是订阅指纹**，不是 Key 数量：把三个窗口的 `reset_at` 拼起来
  （`subscriptionFingerprint`，`src/lib/quota-usage.ts:84-92`）。同一订阅签发的多个
  Key 在服务端看到的是同一批重置时刻，指纹相同 → 去重，只算一份。
- **N 份订阅取均值**（`groupQuotaSeries`，`src/lib/quota-usage.ts:195-233`）：
  N 个订阅的总容量是 N×100%，「整体已用 X%」按容量归一就是各订阅百分比的均值。
  相加会得到 200% 这种没有意义的数。
- **拿不到 `reset_at` 时退回账户 id**（`${kind}#${provider_id}`）。宁可把同一份额度
  按两份算（少合并），也不把两份额度当成一份（少报）——后者会让「还剩多少」的
  判断偏乐观，是更危险的错误方向。

### D5 账户卡片仍按账户，聚合只存在于详情

每张卡片继续对应一个 `provider_id`：卡片上的 `reset_at`、倒计时、阈值告警都取自
该账户真实的窗口重置时刻。**不新增「聚合账户」卡**——聚合卡的 `reset_at` 只能
编造（多订阅的重置时刻各不相同），而卡片 UI 的核心就是「还有多久重置」。聚合
数字落在该账户的悬浮详情里（D3/D4），那里本来就没有单一时点可承诺。

## 考虑过并否决的方案（Alternatives considered）

| 方案 | 否决理由 |
|---|---|
| 维持三窗口百分点求和 | 跨分母相加：同一笔消费在 5h / 周 / 月各记一次，日值可超 100%，且与「本月已用」对不上。任何一天的数字都无法解释 |
| 百分比完全不进账本（现状） | 需要「历史视图只读账本」这条硬约束保持成立，否则又回到「日历有数、趋势为 0」的分裂。双写成本只是把同一串 `daily_snapshots` 日值重放一次，没有第二份真相 |
| 跨订阅百分比直接相加 | 容量不归一时相加没有分母：两个订阅各用 30% 相加成 60% 读不出「还能用多少」，各用 60% 会得到 120%——超过 100% 的数字在额度语境里是错的 |
| 每个 Key 一张卡片 | 同一份额度被展示 N 次，卡片数量随 Key 增长而膨胀；用户以为是 N 份额度，实际是同一份。真正需要「按 Key」看的是明细，不是卡片 |
| 合成「聚合账户」卡 + 编一个 reset 时间 | 卡片的倒计时/告警依赖真实重置时刻。编造的时间会在那一刻之后开始倒数错误的周期，且无法反查。聚合的正当位置是账户卡片详情 |
| 用 provider 白名单而不是 `unit` 判据 | 每接一个百分比制 Provider 都要改前端判据，且白名单与单位漂移后会沉默地漏数据；按 `unit` 判定让新 provider 自动进入额度详情 |

## 后果（Consequences）

### 变得可能

- 百分比制 Provider 的日用量进入账户卡片详情，与 token 视图同源同一天。
- 多 Key / 多订阅有可解释的整体数字：`subscriptions` 份数 + 容量归一均值 +
  本自然月累计（`monthTotal`）。
- 新接入的百分比制 Provider 无需改前端：写进账本即出现在额度详情里。
- 卡片可点选，`highlightKey` 的正确性不再与「能否选中」耦合。

### 已知限制

- **没有金额**：额度详情只有百分点，不能折算成本。想知道「这个月花了多少钱」仍只能
  看订阅账单。
- **没有分模型/分 Key 的历史**：账本里 percent 行的 `model` 恒为空串，额度视图只到
  「账户（`source`）→ 订阅 → kind」三级。
- **趋势页看不到 percent（刻意）**：趋势页只画 token。percent 与 token 量纲不同，
  并列会诱导用户把两者相加；额度因此只在账户卡片的详情里出现。
- **指纹可能碰撞**：两份独立订阅若恰好共享完全相同的三个 `reset_at` 时刻，会被
  判为同一订阅而只算一份。这是刻意的方向性偏好（见 D4），碰撞概率取决于服务端的
  重置策略是否按订阅错峰。
- **历史从开始累计那天起**：百分比是本地观测出来的，不回溯。换机、清库、或首次
  安装之后没有更早的数据；同理「首次观察只建基线，不猜历史」
  （`window_delta` 在无基线时返回 0）。
- **跨午夜的归属**：增量按**轮询当天**落账。23:59 的消费在 00:00 的轮询里被计入
  新的一天，单日边界因此有最多一个轮询周期的误差。
- **只有双写的 Provider 才有额度数据**：本次只有 OpenCode 写账本；MiniMax Token
  Plan 的百分比仍只写 `daily_snapshots`
  （`src-tauri/src/providers/minimax.rs:292-297`），额度详情里暂时看不到它。这是待补的
  缺口，不是设计意图。
- **额度详情只覆盖本机账本**：`fetchQuotaGroups` 读的是本机 `usage_daily`，而全端汇总
  模式的设备序列只有 token 四分量（`LocalDay`），没有 percent。汇总模式下不显示额度摘要。
- **消费方必须自查单位**：`get_usage_history` 返回全部单位（`load_usage_daily`
  不过滤 `unit`）。任何新增的账本消费方都要自己按 `unit` 分流，否则会把 percent
  当 token 累加。这条约束没有类型系统兜底，只有测试与 review。

## 参考

- 账本口径：[docs/usage-ledger.md](../usage-ledger.md)
- 额度聚合：`src/lib/quota-usage.ts`（测试：`src/lib/quota-usage.test.ts`）
- 账本双写：`src-tauri/src/providers/opencode.rs:327-354`
- 日值写入语义：`src-tauri/src/storage.rs:155-174`（累加）与
  `:181-206`（峰值，仅 MiniMax 窗口关闭日用）
