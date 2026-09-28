# 未来优化想法池

> 定位：记录未来优化想法，持续追加；仅记录，不承诺实施时间。
> 创建：2026-09-27（基于 HEAD `c9a37b1`，文中的 file:line 行号以此为准，随代码演进可能漂移）
> 状态图例：待办 ／ 待调研 ／ 已实施

---

## 1. hover 详情弹窗改回贴附卡片形态 — 状态：待办（已评估，维持现状）

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

## 3. 多端同步 P2P + 鉴权替代 hub — 状态：✅ 调研完成（`docs/p2p-sync-research.md`），待决策后实施

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

## 5. 新增工具适配器：DeepSeek harness / Trae CN — 状态：待办（数据源待确认）

**想法原文**：「工具支持添加：DeepSeek harness支持 / Trae CN 支持」

**现状定位**：`src-tauri/src/providers/mod.rs` Provider trait（L506-522）、PRESETS（L171-288）、PROVIDER_REGISTRY（L591-616，12 个 kind）、L643-693 注册表自检单测。既有 7 步新增适配器清单：新建模块 → mod 声明 → PRESETS → PROVIDER_REGISTRY → 前端 `src/lib/types.ts` L340-356 SHORT_KIND_NAMES → `src/lib/brand-glyphs.ts` L13-36 BRAND_GLYPHS/EXPERIMENTAL_KINDS → 单测。

**待决问题**：两者的用量数据源（API 端点 / 本地日志格式）需先确认，才能定走 API 型（Provider trait）还是本地日志型（local/ 模块）。

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
