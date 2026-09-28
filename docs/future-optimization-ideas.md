# 未来优化想法池

> 定位：记录未来优化想法，持续追加；仅记录，不承诺实施时间。
> 创建：2026-09-27（基于 HEAD `c9a37b1`，文中的 file:line 行号以此为准，随代码演进可能漂移）
> 状态图例：待办 ／ 待调研 ／ 已实施

---

## 1. hover 详情弹窗改回最初模式 — 状态：✅ 已实施（`730b953`，底部停靠覆盖热力图）

**想法原文**：「主界面，但鼠标滑到卡片上的弹窗，感觉还是最初的模式好一些，需要改回去」

**现状定位**：详情卡当前为窗口级独立浮层
- `src/App.svelte` L140-154 浮层状态（hoveredId / anchorRect / OVERLAY_GAP=8）、L156-174 350ms 宽限期、L188-209 placeOverlay 定位、L219-235 ResizeObserver 先测量后显示、L1179-1199 浮层 JSX（fly 过渡）、L1702-1717 CSS `position:fixed; z-index:60; width:320px`
- 触发源：`src/lib/components/ProviderCard.svelte` L33-36 `onHover` props、L138-144 pointerenter/leave

**git 演变链**（`git log -S` 考古）：
- `44a6ba3` 最初形态：DetailCard 贴附卡片本体
- `49612fd` 改为窗口级浮层（动机：修复小卡内容裁切，反馈 #2）
- `6cf0947` anchor 到卡片旁 + provider logo

**已确认目标形态**：改回「贴附卡片的详情形态」；实施时必须一并解决当初促成浮层改动的小卡内容裁切问题（否则会回退掉反馈 #2 的修复）。

**初步方向**：恢复 DetailCard 渲染在卡片内部/紧贴布局，同时以「卡片尺寸自适应内容」或「内容精简/折叠」解决裁切，替代窗口级定位方案。

**⚠️ 已评估并回退（`4fc5e70` 实施 → `06b448d` 撤回）**：曾按「详情参与卡片文档流、卡片长高容纳内容」实现贴附形态，替代绝对定位溢出（该方案在 `.shell` overflow:hidden + `.shell__cards` overflow-y:auto 两层裁切下确实不裁切，且经隔离 HTML 实测三个边界场景均为「卡片自溢出 false」）。但实机验证后确认**观感不符并已完整回退**：hover 会使卡片增高、下方列表重排，与预期不符。当前维持窗口级浮层（`detail-overlay`：先测量再显示、视口夹取与上下翻转、350ms 宽限期、飞入过渡），provider logo 优化不受影响。

**若日后重做此条，需注意的取舍**：贴附形态无法同时满足「不重排」与「不裁切」——`4fc5e70` 选择了不裁切（代价是重排）。要两者兼得只能走第三个方向：浮层贴附卡片边缘但不占布局（如锚定到卡片右侧、随卡片滚动），而非让卡片长高。

**✅ 实施结果（提交 `730b953`）**：**不是**让详情卡参与卡片布局把卡片撑高（那是第一次的误解），
而是恢复"hover 时用详情**覆盖底部热力图区域**"的最初形态。

- **最终几何**：浮层停靠在窗口底部，左右满宽（`bottom: 48px`、`left/right: 14px`），
  正好压在主界面热力图上方，鼠标移开即让位给热力图。
- **保留的能力**：先测量后显示（`ResizeObserver`）、视口夹取与上下翻转、350 ms 离开宽限、
  `fly` 入场动画、provider logo。
- **穿透**：浮层 `pointer-events: none`，避免遮挡下方热力图的 hover 交互。
- **定位方式**：通过 `git log -S` 回溯历史形态，确认真正的"最初模式"对应提交 `dc5992e`
  （详情停靠底部覆盖热力图），而非 `44a6ba3`（贴附在卡片内）。
- 迭代过程：`4fc5e70` 曾误实现为"贴附卡片、卡片增高"，经用户两次澄清后由 `730b953` 纠正。

---

## 2. 剩余时间 >24h 换算为天/时/分 — 状态：✅ 已实施（`c5ebdb3`）

**想法原文**：「主界面卡片上的剩余时间，特别周和月倒计时，但>24h时，换算成天，时，分的显示模式更直观」

**现状定位**：`src/lib/components/ResetCountdown.svelte` L21-30，格式化仅有 h/m/s 三档、无「天」档位；月窗口倒计时显示为「719h 59m」式长小时数，不直观。

**初步方向**：>24h 时显示 `Xd Yh Zm`（天/时/分三段），24h 以内维持现有 h/m/s。相关但不受影响的：`src/lib/components/DetailCard.svelte` L77-87 resetLabel（显示时间点，非倒计时）。

**✅ 实施结果（提交 `c5ebdb3`）**：`>24h` 显示 `29d 23h 59m` 式，24h 以内维持 `23h 59m` / `59m 59s` / `59s`。
- **关键点**：小时的取模基数随之改为 `(totalSec % 86400)/3600`，否则 719 小时仍会呈现为 `719h`，天数档形同虚设。
- 已逐档实测边界：已过期、0 秒、分钟/秒档、23h 与 24h 边界、719h 场景、30 天整、365 天。
- 影响面：`ResetCountdown` 的三处调用（5h / 周 / 月窗口）自动受益。页脚「下次刷新」用独立的分秒格式（最长几分钟），不受影响。

---

## 3. 多端同步 P2P + 鉴权替代 hub — 状态：✅ 调研完成 + 阶段 0/1 均已实施（`0013a10`、`3b3d0a3`）

**想法原文**：「现在的多端模式是基于hub, 我在想能不能基于p2p+鉴权的方式进行连接同步，这可能需要先看看有没有成熟的实现方案」

**现状定位**：多端同步基于中心 hub——`src-tauri/src/hub.rs` 手写 HTTP/1.1（POST /ingest、GET /devices、可选 Bearer 鉴权、SQLite 持久化）；`src-tauri/src/lib.rs` L350-391 agent 模式 30s 循环上报近 90 天摘要；`src-tauri/src/settings.rs` hub_mode 默认 off、hub_port 43210。

**初步方向**：先调研成熟方案再定架构，候选：libp2p、mDNS 局域网直连 + 配对鉴权、Tailscale 等组网方案、CRDT 冲突合并。产出调研结论前不动代码。

**✅ 调研结论（详见 `docs/p2p-sync-research.md`）**：**推荐 mDNS 发现 + 局域网直连 + 配对鉴权，与 hub 并存；不推荐 libp2p 与 CRDT。**
- **libp2p 过度设计**：其价值在于跨 NAT 去中心化与抗审查，而本项目是个人自用的局域网汇总。单条上报仅 90 个 `{date,total}`（< 10 KB，`tool_daily_from_cache` L277-279 裁剪），远达不到需要 P2P 的规模；而依赖成本是数百个传递依赖，与项目「不引入额外依赖」的取向冲突。
- **CRDT 当前无解可解**：数据是**单写者汇总**（每台设备只写自己的用量，汇总是并集），不存在并发写冲突。Automerge 2.0 / yrs 0.28 均成熟，但要等出现「多端并发改配置」的需求才值得引入。
- **mDNS 方案优势**：`mdns-sd` 纯 Rust、无异步运行时依赖、27.7 万次下载；**完全复用现有 `HubDevice` 与两个端点**，只把「知道 hub 地址」换成「自动发现」。
- **调研中的关键发现**：
  1. **现有角色互斥是改造前提**——`lib.rs` L352-391 中 `hub_mode` 为单值（`hub`/`agent` 二选一），hub 节点**看不到自己的用量**。这本身是个待修的缺陷，且 P2P 要求节点既监听又上报，必须先打破互斥。
  2. **P2P ≠ 无中心节点**：NexDesk（Tauri + webrtc-rs）这类成熟 P2P 产品**仍依赖信令服务器**。本项目真正的收益是「免手动填 hub 地址」，而非去中心化。
  3. **鉴权现状偏弱**：`token` 为空则完全放行（`hub.rs` L160-165），局域网内等于无鉴权。
- **推荐分阶段**：阶段 0 先做零依赖改进（hub 节点补上自身用量 + token 默认必填）；阶段 1 再叠加 mDNS 发现。
- **落地摩擦预警**：Windows 防火墙需放行入站；mDNS 仅限同网段（跨网段仍需 hub 或 Tailscale）；Docker/VPN 环境下组播可能不可达。
- **待用户决策**：是否接受防火墙提示、是否需要跨网段、节点规模、与 hub 并存是否可接受（详见调研文档第五节）。

**✅ 阶段 0 已实施（提交 `0013a10`）**：上列两个既有缺陷均已修复。
- **hub 看不到自身用量** → hub 模式启动后按 **30s 周期** upsert 本机用量进 `hub_devices` 表；抽出 `build_self_device` 与 agent 共用，保证两角色数据口径一致。
  - 踩坑记录：第一版「启动 6s 后写一次」实测 `tool_tokens` 恒为 0（本地扫描未预热，写一次就留下永久空快照），改为 30s 周期刷新后才拿到真实值（1451853475.0 / 5 tools / 90 天 daily）。**周期刷新是必需的，不是优化。**
- **token 为空即放行** → 启用 hub 时若「尚未配置」则自动生成 32 位十六进制密钥（128 bit `getrandom`）并落盘。
  - 关键设计：新增 `hub_token_configured` 区分「从未设置」与「显式清空」。老配置无该字段 → 默认 false → 补发密钥；用户主动清空（有意关闭鉴权）→ 前端保存置 true → 不再被填回。否则「默认安全」会变成「无法关闭鉴权」。
- **副作用**：存量用户首次以 hub 模式启动后 `hub_token` 被自动写入，**agent 端需同步填写同一密钥**，否则上报被 401 拒绝；设备页现在会出现汇总节点自己这一行。
- 依赖影响：`getrandom 0.3.4` 本就是传递依赖，仅提升为直接依赖，Cargo.lock 只增一行。新增 3 个单测；`cargo test` 134 passed。真机实测三种语义（自动生成 / 401-401-200 三档鉴权 / 显式清空后不被覆盖）全部验证通过，测后已把 config.toml 还原为 `hub_mode="off"`。
- **阶段 1（mDNS）仍待上述四点决策**，未动代码。

**✅ 阶段 1 已实施（提交 `3b3d0a3`）**：新增 `hub_mode = "lan"` —— 同网段实例通过 mDNS 自动互相发现并全连接同步，用户无需填写任何地址；跨网段仍由既有 `agent` + `hub_base` 承担（用户要求两者并存）。
- **用户决策**：跨网段由 hub 承担、mDNS 只管同网段；同网段用全连接 mesh；配对复用现有共享密钥；规模 10 台以内。
- **关键推论**：≤10 台全连接 ⇒ 每台向其余 9 台上报，**一跳即全知**，因此**不做 Gossip/多跳转发**（会引入重复计数与环路风险）。`hub_device` 表以 `device_id` 为主键的设计天然适配。
- **只做发现、不做传输**：复用已有 `/ingest` 与 `/devices`，mDNS 只补"对端地址从哪来"这一个缺口。
- **实现要点**：对端以 `device_id` 去重（而非地址，否则 DHCP 续租后设备页出现重复行）；跳过回环与 `0.0.0.0`（否则自我上报）；过滤自身的回环发现；多网卡时排序选取地址保证可复现；网络禁 multicast 时**静默降级**为手动 hub。
- **依赖**：`mdns-sd` 0.21.4（纯 Rust、无 C 依赖）。
- **实测**：lan 模式监听 43210 成功；模拟对端上报 200；设备汇总正确列出 3 台；错误 token 401。新增 8 个 p2p 单测；`cargo test` 142 passed。测后已还原 config.toml 为 `hub_mode="off"`。
- **⚠️ 未验证**：真实双机 mDNS 互发现（Tauri 单实例插件不允许同机双开，且 `device_id` 取自 hostname），需用户在两台真实机器上实测含 Windows 防火墙放行。
- **阶段 2（CRDT）不适用**：单写者汇总，无并发冲突可解。

---

## 4. 工具页 codex 90 天未用仍被扫描加入 — 状态：✅ 已实施（`3b1c468`，采用方案 a+c 合并）

**想法原文**：「工具页面，我都没有90天内使用codex,但是扫描还是把它加进去了，感觉没有意义，需要优化一下逻辑」

**现状定位**：`src-tauri/src/local/mod.rs` L54-56 保留规则 `tools.retain(|t| !t.daily.is_empty() || cache::tool_installed(&t.id))`——「有近 90 天数据 OR 已安装」；`src-tauri/src/local/cache.rs` L100-109 codex 的「已安装」判定 = `.codex/sessions` 目录存在（历史会话残留即算装了）。TOOL_IDS 见 `src-tauri/src/local/mod.rs` L61；codex 扫描含 Windows + WSL（`src-tauri/src/local/codex.rs`，KEEP_DAYS=90）。

**初步方向（三选一，实施时定）**：
- a) 纯 installed 无数据且超 N 天未更新的不再显示 ← **已采纳**
- b) 给用户「隐藏未使用工具」开关
- c) 安装判定加时效性（如按最近会话文件 mtime） ← **已采纳（作为 a 的判定手段）**

**✅ 实施结果（提交 `3b1c468`，用户选定方案 1）**：保留规则收紧为「**有近 90 天数据即保留；无数据才要求已安装且近期活跃**」。
- **新增能力**：`wsl.rs` 的 `newest_mtime`（取日志树最近写入时间，**深度 4 / 每工具 2000 条目有界遍历**，因为该判定在工具页刷新时同步执行）；`cache.rs` 的 `tool_last_active_at`（会话日志类取目录树最新文件、SQLite 类取数据库自身 mtime）与 `tool_recently_active`（阈值常量 `STALE_DAYS = 30`）。
- **防误伤**：读不到时间戳时回退到旧的「仅看存在性」判定，不因读不到时钟就隐藏工具。30 天阈值相对 90 天数据窗口留了余量。
- **真机实测（2026-09-28）**：codex 距今 **117 天**、90 天内 0 个会话文件 → `active=false` **被隐藏**（即本条诉求）；claude-code 距今 47 天但 90 天内有 9 个会话（含 2MB 真实会话）→ 靠「有数据即保留」留在面板；hermes 距今 0 天 → 保留。
- 新增 3 个单测（嵌套文件可被发现、深度与预算上限生效、目录不存在返回 None）；`cargo test` 131 passed / 0 failed。
- **已知限制**：极大历史目录下可能因遍历上限漏检最新文件而误判陈旧；WSL 侧（`\\wsl$\...`）路径的活动时间未单独验证。阈值目前是常量，若日后需要可改为设置项。

---

## 5. 新增工具适配器：DeepSeek harness / Trae CN — 状态：🟡 DeepSeek Harness 已实施（走本地日志型）／ Trae CN 挂起（本地无数据）

**想法原文**：「工具支持添加：DeepSeek harness支持 / Trae CN 支持」

**现状定位**：`src-tauri/src/providers/mod.rs` Provider trait（L506-522）、PRESETS（L171-288）、PROVIDER_REGISTRY（L591-616，12 个 kind）、L643-693 注册表自检单测。既有 7 步新增适配器清单：新建模块 → mod 声明 → PRESETS → PROVIDER_REGISTRY → 前端 `src/lib/types.ts` L340-356 SHORT_KIND_NAMES → `src/lib/brand-glyphs.ts` L13-36 BRAND_GLYPHS/EXPERIMENTAL_KINDS → 单测。

**数据源调研结论**：

- **DeepSeek Harness** → 有数据，走**本地日志型**（`local/` 模块），非 Provider trait。
  会话日志位于 `~/.dsh/sessions/<项目>/<会话>/session.v{3,4}.jsonl.zstd`（**zstd 压缩的 JSONL**）。
  实测本机 2 个 Windows 项目 + 1 个 WSL 项目、11 个会话目录、14 个会话文件（7 个 v3 + 7 个 v4）。
- **Trae CN** → **本地拿不到 token 用量**，无法实现。目录 `AppData\Roaming\TRAE SOLO CN` 是 VS Code fork，
  核心数据在 `state.vscdb` / `workspaceStorage` / `IndexedDB`；扫描 897 个文件后
  `totalTokens` / `inputTokens` / `outputTokens` / `promptTokens` / `cachedTokens` **命中全为 0**。
  大量 `usage` 命中实为 `"saas_usage": {"max": null, "default": null}` 配额配置，不是消耗记录。
  → **继续挂起**，除非官方后续提供本地用量落盘。

**✅ DeepSeek Harness 实施结果（用户选定「只做 DeepSeek harness」）**：

- **新增 `local/dsh.rs`**，扫描器 id `deepseek-harness`，显示名「DeepSeek Harness」，接入 `TOOL_IDS`（6 个）、`scan_all` 并行扫描、`scan_tool` 分派、`cache.rs` 的 `tool_installed` / `tool_recently_active` / `tool_fingerprint`（新增 zstd 遍历）。
- **数据格式要点（⚠ 经真机修正，见下）**：dsh 存在**两种 usage 载体**——
  旧版 `assistant/attempt` 的 `data.stream[].chunk.type == "usage"`，
  以及现行 `assistant/message` 的 `data.usage` 直挂。
  字段为 `inputTokens` / `outputTokens` / `cacheReadTokens` / `totalTokens`，时间戳是 epoch 毫秒。
- **三个关键取舍**：
  1. **重试取最后一个 usage chunk**：同一次 attempt 会流式推送多次 usage，取末位才是结算值，取首位会少算。
  2. **自行计算 input+output，不信 `totalTokens`**：失败请求也会写 totalTokens，照抄会混入不该计费的部分。
  3. **全零 usage 直接跳过**：真实日志里 usage 全为 0——跳过而非计成 0 行，
     避免用假数据掩盖真实故障。**零值成因是混合的**（下方专项核查），
     但对适配器而言成因无差别，故按"全零即跳过"统一处理。
- **v3/v4 去重（真机实证）**：dsh 升级时会把 v3 与 v4 并列写进同一会话目录。
  实测 11 个会话目录中有 3 个同时含 v3+v4。**逐事件比对确认 v4 是 v3 的严格超集**（`only-in-v3 = 0`，
  v4 事件数 ≥ v3），因此只读最高版本既不丢数据，又避免 token 与会话数翻倍。
  真机扫描 `session_count=13`、`project_count=3` 与磁盘结构精确吻合（Windows 11 + WSL 2）。
- **⚠ 格式修正（2026-09-28，真机实测发现严重漏读）**：
  初版只解析 `assistant/attempt`。在**正在使用的** dsh v4 会话里该事件数为 **0**，
  真实用量挂在 `assistant/message` 的 `data.usage` 上——实测两载体数量为 **921 : 24**，
  即初版漏读约 **97%** 的数据，且因历史数据恰好多为零，**表面看不出任何异常**。
  这是本次实施中最危险的缺陷：静默少算比报错更难发现。
  - **两载体时间戳无交集**（attempt 描述一次请求的结算，message 描述每步回复的用量），
    因此**只能取其一、不能相加**，否则同时含两种布局的会话会被重复计数。
  - 实现改为**优先 `assistant/message`、`assistant/attempt` 仅作旧版回退**，每行至多产出一次。
  - **`cacheReadTokens` 单列**：`totalTokens ≠ input + output`，差额正是缓存读取部分
    （实测如 `in=403 cache=12800 out=266 total=13469`）。若把 total 当作 input+output
    就会把缓存量并入输入，缓存与普通输入费率不同，成本会虚高。
    现与 `claude.rs` 一致地记为 `(input, cache_read, output)` 三元组。
  - **修复效果（同一份真实数据）**：`total_tokens` 由 **0 → 87,945,563**，
    覆盖 5 个自然日；按日合计与总数**对账一致**；`daily with >0 = 5`（不再全空）。
- **安全：解压上限约束的是"解压后输出"**（初版误套在压缩输入上，形同虚设）。
  限流器置于 decoder 之外，多读 1 字节以判超限；超限整份丢弃而非返回截断内容——
  否则会把残缺会话当完整数据解析，悄悄少算。
- **✅ 模型拆分（2026-09-28 修复，模型字段就在 usage 同一事件上）**：
  此前判断"model 需跨事件关联、暂不拆分"是**错的**——`assistant/message` 自身就带 model：
  `data.message.source.model`（镜像于 `data.message.source.replayState.response.model`）。
  实测 **924/924** 条 usage 行都带 model，两处取值 100% 一致，无需任何跨事件推断。
  - **取值用 `source.model`（请求模型），不用 `responseModel`**：后者是服务端版本化别名
    （如 `deepseek-v4-flash-ga-260731`），而价目表按请求模型 id 前缀匹配，
    误用它会让所有行匹配不到费率、整桶无成本。
  - **会话中途确实会切模型**：`MiniMax-M3` 675 行 / `deepseek-v4-flash` 249 行，
    所以按模型拆分是实质区分而非装饰。
  - **两个模型均可计价**：`deepseek-v4-flash` 命中兜底 `deepseek` 档，
    `MiniMax-M3` 归一化后命中 `minimax-m3` 档。
  - 仍保留 `data.model` 作为旧版回退；完全无 model 的行照旧进 `UNCLASSIFIED_MODEL`，保证对账。
  - **真实结果**：`MiniMax-M3` 4 天 / in 1,285,538 / cache 54,842,825 / out 250,887 / 估算 **$285.66**；
    `deepseek-v4-flash` 2 天 / in 5,982,124 / cache 25,241,344 / out 342,845 / 估算 **$8.81**；
    **按模型合计 = 按日合计 = 总计 87,945,563，对账一致**。
- **验证（2026-09-28）**：`cargo test` **166 passed / 0 failed**（dsh 测试 24 个）；
  `cargo clippy` 对 dsh.rs 零告警；`dsh.rs` rustfmt 干净；`npx svelte-check` 0 错误；`npx vite build` 成功。
- **zstd 炸弹实测**：构造 16 KB → 512 MB（压缩比 32000:1）的恶意文件，
  在 8 MB / 64 MB / 256 MB 三档上限下耗时 31 ms / 198 ms / 1.01 s **严格线性增长**，
  证明内存被上限钳住、且限流确实作用在解压输出端（而非碰巧因文件损坏而失败）。
- **前端零改动**：`ToolPanel` / `ToolWindow` / `ModelPanel` 的工具名与 id **全部由后端 `get_local_tools` 驱动**，
  组件内无任何工具 id 硬编码，新增工具不需要前端映射。品牌色沿用 `local` 的灰色。
- **零值 usage 成因专项核查（2026-09-28，更正此前"仅 API key 失效"的表述）**：
  按每个 `assistant/attempt` 的 `finish.reason` 归类，两侧会话共 25 个 usage chunk **全部为零**，
  成因分布如下（**并非**同一种故障）：

  | 侧 | 成因 | 次数 |
  |---|---|---|
  | Windows | 429 配额类（含 6 次 `AccountQuotaExceeded` 周配额耗尽） | 18 |
  | Windows | `UnknownError / PI_AI_ERROR` | 5 |
  | Windows | 401 `AuthenticationError`（key 格式错误） | **仅 1** |
  | WSL | 401 鉴权失败 | 1 |

  **更正说明**：早前结论"本机 dsh API key 已失效导致 usage 全为 0"把次要原因当成了主因。
  实际主因是 **429 配额超限**（Windows 侧 18/24），401 只占 1 例；WSL 侧则确为鉴权失败。
  该更正**不影响代码**——三类成因都产生零值，`全零即跳过` 的处理完全一致；
  但影响排查方向：换 API key 未必能恢复数字，需先确认配额与服务端状态。
  另注：WSL 侧日志中出现的 `API key` 字样均为插件文档正文的自然语言，非报错，勿混淆。
- **真实用量已可统计**：修复格式后工具页可显示实际数字（实测总计 87,945,563 tokens / 5 天）。
- **部署形态**：DeepSeek Harness 是 **Windows 桌面应用**
  （`AppData\Local\Programs\DeepSeek Harness`，非 CLI，故无 `dsh` 命令），
  会话日志仅落在 Windows 侧；WSL 侧 `.dsh/sessions` 存在但无用量记录。
- **已知限制**：成本为按公开价目表的**估算**（`cost_estimated = true`），
  与 dsh 实际计费（`volcanoagentplan` 渠道价）可能有偏差；
  缓存读取（`cacheReadTokens`）按输入费率计入——`compute_cost` 现有实现即如此，
  真实缓存折扣因模型而异，故 $285.66 可能偏高；
  WSL 侧仅经 `existing_dotdirs` 通用路径扫描，未针对 dsh 单独验证读取性能；
  若 dsh 后续把 model 移出 `message.source`，需回退到"未标记模型"。

---

## 6. 关于页常驻内存占用信息错误 — 状态：✅ 已实施（`08364db`，采用方案 a）

**想法原文**：「关于页面更新，现在的常驻内存占用信息是错的」

**现状定位**：`src/Settings.svelte` L1146 硬编码静态文案 `<li>最小化资源占用：常驻内存 ≈ 39 MB</li>`；全仓库无任何运行时内存采样代码。

**初步方向（二选一，实施时定）**：
- a) 直接删除/改写该文案（低成本） ← **已采纳（用户选定方案 A）**
- b) Rust 侧实测当前进程 RSS（Tauri command + 前端动态显示），更准确但需新增 command

**✅ 实施结果（提交 `08364db`）**：文案由「最小化资源占用：常驻内存 ≈ 39 MB」改为「**常驻后台：仅托盘图标与数据轮询，无弹窗打扰**」，不再出现任何数字。
- **新表述逐项可验证**：托盘图标（`src-tauri/src/lib.rs` 的 `TrayIcon` 字段与 `TrayIconBuilder`，窗口隐藏后仍常驻）、数据轮询（`rates-updated` 事件广播）、无弹窗（托盘常驻的设计本身）。原则是「不编造新数字，只讲可验证的行为」。
- **全仓库排查**：确认这是**唯一**一处硬编码的资源占用宣传数字——其余 `≈` 出现在币种与成本估算处（`currency.ts`、`ModelPanel.svelte`），属功能性的近似标记，与资源宣称无关。同段落其余四条本就可验证，未改动。
- 验证：`svelte-check` 0 error 0 warning；`vite build` 通过。
- **若日后想展示真实占用**：走方案 b（Rust 侧 RSS 采样 + Tauri command）。注意实测值会随 WebView2 与缓存状态波动，单个瞬时值仍可能被认为不准确。
