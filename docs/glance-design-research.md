# glance 设计研究报告 — 精读 lulu-loopp/glance 的视觉 / 动效 / 设置 / 贴边

> 研究对象：[lulu-loopp/glance](https://github.com/lulu-loopp/glance) @ commit `6974ed4` — Windows 边缘触发式系统监控面板，纯 Rust 手绘 UI（Direct2D/DirectWrite + WinRT UI_Composition），单 exe 约 2MB。
> 研究方式：4 路并行子代理逐文件精读源码（视觉 / 动效 / 设置窗口 / 贴边触发），另加 1 路 TokenUsageMonitor 四领域现状盘点做对照。
> 本文目标：把 glance 的**设计意图与精确参数**提炼出来，映射到我们（Tauri v2 + Svelte 5）的技术栈上，形成可执行的落地清单。
> 注：glance 是纯 Rust 手绘 UI，没有 Web 前端——所以值得学的不是它的实现方式，而是它验证过的**参数、原则与手感**。

---

## TL;DR — 十个最值得偷的东西

1. **两条曲线管全局**：出现 `cubic-bezier(0.16,1,0.3,1)`（≈easeOutExpo）+ 淡入 `linear`；消失 `cubic-bezier(0.4,0,1,1)`。全项目没有第二组时序常数——观感统一即高级。我们的 token 已对标，欠的是全项目接线。
2. **位移与透明度分轨**：开 = 260ms 位移 + 140ms 线性淡入（淡入永远更快更线性）；关 = 180/180（离场更快）。
3. **一切可中断、从当前位置续走**：动画引擎的 retarget 从当前值起步，任何动画被打断都不跳变——"跟手"的根源。
4. **单色墨水 + 单点彩色**：整个面板一种墨色，仅数值过热才出现警示色、仅 game 模块例外用绿——克制产生高级。
5. **大数字用窄体**：可变字体 `wght 620 + wdth 66` + `letter-spacing -0.01em` + `line-height .84`，仪表盘级数字排版。
6. **玻璃边缘光三件套**：顶 inset 高光 α0.5 / 底反光 α0.12 / 全周 18px 内发光 α0.07 + 1.25px 的 135° 渐变描边环——CSS 可以逐字复刻（见 §2.6）。
7. **自适应磨砂**：玻璃浓度随背景"繁忙度（σ 0.03–0.14）"与"明暗冲突（跨 0.35）"连续变化，文字永远可读而不必永远浓雾。
8. **设置=live-apply + 三路合并**：改即存，`merged(held, base, mine)` 只写本窗改过的键，多窗口并发写不打架；滑块拖动只进预览、松手才落库。
9. **贴边触发是"推"不是"碰"**：光标物理压住最外 1px 继续向外推，鼠标 raw counts 在 250ms 窗口内累计过阈值（默认 120）才呼出——沿边滑过、滚动条、角落 48px、按键拖拽全部不误触；"需要有意识地推一下"是一个精确算法，不是玄学。
10. **设置窗细节三连**：焦点环只在键盘使用后出现（`:focus-visible` 零成本等价）· 行内 hint 替代 tooltip、状态反馈内联化 · 坏配置救捞而非拒绝（半坏 JSON 逐字段嫁接回默认基底 + 备份原文件 + journal 留痕，永不给用户"配置损坏"死局）。

---

## 1. 架构总览

- **纯 Rust 手绘 UI**：分层 `canvas.rs`（跨平台抽象）→ `gfx.rs`（D2D 实现 + 资源缓存）→ `theme.rs`（三套皮肤主题数据）→ `skins.rs`（皮肤背景）→ `view.rs`（布局控件）→ `icons.rs`（Lucide 图标）。无 UI 框架，一切立即模式绘制。
- **三套内置皮肤**（整套替换）：`Paper`（记录纸）/ `Fluent`（Win11 亚克力）/ `Glass`（磨砂玻璃）。连"色彩哲学"都不同：Paper 单色墨水、Fluent 用系统 accent 作数据色、Glass 多彩 lane。
- **性能设计贯穿始终**：阴影 9-slice 缓存、静止层缓存、按需逐帧（有动 60fps，静止零重绘）、不模糊就不付模糊的钱。
- 对我们的启示：**不学它的实现路线**（那是 2 万行手绘引擎），学它把"每一个视觉决策都写成带注释的精确常数"的纪律。

---

## 2. 视觉设计系统

### 2.1 颜色

- 三级文字灰阶 `text/text2/text3` + 分割线 `rule` + 轨道 `track`（= 文本色低透明度）+ 警示 `signal`。
- **透明度阶梯 = color-mix 语义**：`Ink { trace, trace2, wash, wash_end }` 即一色四强度 `ink(色, 0.55, 0.34, 0.0)`，源码注释直接点名 CSS `color-mix(in srgb, c p%, transparent)`。
- **hover 永远是低透明度填充**（暗 6% / 亮 4%），永不高亮边框。
- 过热信号：数值、meter 填充、图表线三处同步换 `signal` 色，深浅色各配专门 shade。
- **自动深浅色**：跟随壁纸平均亮度（Rec.709 luma ≤ 0.5 为暗色）——比"跟随系统"更贴场景。

### 2.2 字体与排版

- 打包 Archivo + Inter 的 **latin 子集**（共 ~140KB woff2），中文走系统回退（"Microsoft YaHei UI"）——正是我们的 MiSans 方案可借鉴的减重思路（我们 4.6MB 全量 56 分片可裁）。
- 字号阶梯：body 13 / small 12 / title 13–14(650) / unit 15 / **figure（大数字）40–42(600–620)**。
- figure 三件套：**负字距**（-0.01 ~ -0.035em）+ **行高 0.84** + **宽度轴 66%**（`wdth` condensed）——高级感的关键一招，Archivo 是可变字体直接可用。

### 2.3 空间语言（DIP 度量）

| 度量 | 值 | 备注 |
|---|---|---|
| 列宽 | 356 | 跨皮肤不变 |
| 标签列 | 96 / gap 12 | |
| 行高 | 16 | 正文行 |
| 表格行 | 22 | |
| 按钮 | 高 32，胶囊形 | 所有控件 `radius = min(control_radius, h/2)` |
| 面板圆角 | Paper 16 / Fluent 8 / Glass 24 | 圆角阶梯 24→16→8→4→胶囊 |
| meter 高 | 3–6 | 轨道 = track 色 |

### 2.4 玻璃材质（三套真实配方）

- **Glass 皮肤**：背景模糊 `blur(9px) saturate(1.3)` + 位移图扭曲模拟凸玻璃边缘折光（CSS 无对应物）+ tint 随 frost 0.16→0.84 连续变化。
- **Acrylic**：`blur(30px) saturate(1.25)` + Luminosity 混合层（深 0.96 / 浅 0.85）抹平桌面亮度——桌面再花哨底色也均匀。
- **自适应磨砂 frost()**：`busy = clamp((σ-0.03)/0.11)` 与 `against = clamp((0.4-|mean-参考|)/0.35)` 取 max → 背景越乱越磨砂。
- **画布内 frosted**：模糊后提饱和补偿发灰（saturation 1.3，"each colour keeps more of itself"）。

### 2.5 阴影与边缘光

- **双层阴影全皮肤标配**（CSS 语义直译，blur/2=σ）：Paper 深 `0 18px 40px -12px rgba(0,0,0,.6)` + `0 2px 6px rgba(0,0,0,.3)`——大而虚的环境影 + 近而实的接触影。
- **三重 inset 内发光**：`inset 0 1px 2px rgba(255,255,255,.5), inset 0 -1px 2px rgba(255,255,255,.12), inset 0 0 18px rgba(255,255,255,.07)`。
- **135° 渐变描边环**（1.25px）：stops α = 0.75 / 0.2 / 0.05 / 0.08 / 0.22（左上受光、右下余光）。
- hairline 描边画在**外侧**（`box-shadow: 0 0 0 1px` 等价物）；贴边侧圆角伸出屏幕外（纸"长在"屏幕边上）。
- 玻璃下的阴影只画在玻璃外（环形 mask），不污染透视区。

### 2.6 可直接抄的 CSS（等价实现）

```css
/* 玻璃面板 */
background: rgba(20,20,26,.68); /* dark */
backdrop-filter: blur(9px) saturate(1.3);
/* 边缘光三件套 */
box-shadow:
  inset 0 1px 2px rgba(255,255,255,.5),
  inset 0 -1px 2px rgba(255,255,255,.12),
  inset 0 0 18px rgba(255,255,255,.07),
  0 10px 30px rgba(0,0,0,.3),   /* 双层阴影 */
  0 1px 3px rgba(0,0,0,.2);
/* 135° 渐变描边环（stops 照抄 rim()） */
border: 1px solid transparent;
background:
  linear-gradient(var(--panel-bg)) padding-box,
  linear-gradient(135deg, rgba(255,255,255,.75), rgba(255,255,255,.2) 22%,
    rgba(255,255,255,.05) 50%, rgba(255,255,255,.08) 78%, rgba(255,255,255,.22)) border-box;
/* 外描边 hairline */
box-shadow: 0 0 0 1px var(--stroke);
/* 大数字 */
font-variation-settings: "wght" 620, "wdth" 66;
letter-spacing: -0.01em; line-height: .84;
```

### 2.7 视觉显得高级的 12 条手法（速查）

双层阴影（ambient+key）· 单色墨水+单点彩色 · color-mix 透明度阶梯 · 窄体大数字 · 玻璃边缘弯折（CSS 无解，可用边缘更亮更透暗示）· 135° 渐变边缘光 · 三重 inset 内发光 · 自适应磨砂 · Acrylic 亮度抹平层 · 阴影只落玻璃外 · 像素级对齐纪律（图标吸附偶数像素、hairline 画外侧）· 小字 GDI 式精修（灰度对比 2.0）。

---

## 3. 动效设计

### 3.1 引擎

- 非 spring、非逐帧，是 **CSS transition 的单值复刻**：`Transition { from, to, start, duration, easing }`，`value(now) = from + (to-from)·easing.at(x)`，与 CSS/WAAPI 同一套贝塞尔求值算法（牛顿 8 次 + 二分 32 次兜底）。
- **`retarget()` 从当前位置续走**（`from = value(now)`）——全项目可中断性的来源：滑块中途改选续滑不回弹、布局切换中途再切从当前插值出发。
- **没有第二组时序常数**：出现 `OPEN=(260ms, (0.16,1,0.3,1))`、消失 `CLOSE=(180ms, (0.4,0,1,1))`、内部形变统一 260ms + 四次幂 easeOut `1-(1-t)⁴`。

### 3.2 面板显隐（核心参数）

| 项 | 打开 | 关闭 |
|---|---|---|
| 位移 | 260ms `c-b(0.16,1,0.3,1)` | 180ms `c-b(0.4,0,1,1)` |
| 透明度 | 140ms `linear` | 180ms `linear` |
| reduced-motion | 只留 120ms 纯淡入 | 同左 |

- 入场动画随皮肤性格：Paper 整块从屏幕外推入（Beyond 40），Glass 短滑淡入（Slide 36）。

### 3.3 微交互常数表

| 交互 | 参数 | 出处 |
|---|---|---|
| thumb hover/按住 | 放大 1.4× / 缩小 0.85×，指数趋近 τ=40ms | thumb.rs |
| 选中标记滑移 | 220ms easeOutCubic，中途改选续滑 | thumb.rs |
| 拖拽重排 | 200ms easeOutCubic，松手飞回槽位 | thumb.rs |
| 列表滚动 | 指数趋近 τ=60ms（掉帧不动感） | view.rs |
| 歌词行切换 | 420ms cubicOut，交叉淡化 | playing.rs |
| 歌词 rail（唯一真弹簧） | stiffness 142 / damping 28 / mass 0.82，子步 ≤1/240s | music.rs |
| 频谱柱 | **上升瞬时、下落 ≤1.7/s 线性封顶**——画显示值不画瞬时值 | music.rs |
| 控件淡入 | 180ms 线性 + smoothstep | music.rs |
| 穿透按钮防误触 | hover 停留 500ms 才现身 | buttons.rs |
| hover 底色 | 大多**瞬时**，不过度动画 | 全局 |

### 3.4 帧率纪律

- busy 位聚合决定帧调度：有动 → 60fps；静止但有图表 → 每 0.25px 重绘一帧；音乐区静止 → 250ms 一帧；完全静止 → 零重绘。
- 对我们的映射：Tauri 后端数据推送节流，前端 rAF 仅在有活动动画时跑，静止停循环。

### 3.5 尊重系统

- `reduced_motion()` 读 Windows 辅助功能"动画效果"（SPI_GETCLIENTAREAANIMATION），禁用位移只留淡入——对应我们已有的 `prefers-reduced-motion` 三层降级。

---

## 4. 设置窗口

### 4.1 架构与手感参数

- 独立 OS 窗口（1300×760，min 1080×560，≤工作区 90%），原生边框 + Mica 背景；**全窗禁用 IME**（快捷键录制需要原始 VK）；单例 + 关闭后保活 10s（再开近乎瞬时）。
- IA：左导航 200px（项高 40、图标 16）+ 中卡片列 400px + **右实时预览**（布局页=真实屏幕排布；外观页=桌面截图上渲染真实面板）。
- 行模板：行高 60、label 14px、hint 13px 可换行（**行内 hint 替代 tooltip**）；导航项左竖条 3px 选中指示。
- 动效手感：分段选择 thumb 滑动 240ms `c-b(0.16,1,0.3,1)`（**换行选项间也滑动**）；开关翻转 160ms + 按住向运动方向拉伸（rubber 手感）；页签下划线**先导后随**（lead 200ms / trail 360ms）；换语言在动画播完才换文案（`relabel_at = now + SLIDE`）。

### 4.2 生效模型（最大亮点）

- **改即存，无确定/取消按钮**：每次变更即时 `save()`。
- **三路合并**：`merged(held, base, mine)` 逐键三路合并——本窗没改的键保留 held（别的窗口/进程的并发修改原样站立）；有专门测试 `a_change_made_elsewhere_stands`。
- **滑块两段提交**：拖动中只推实时预览不落库，松手才写单字段——重型预览零落库成本。
- **异步开关以系统回报为准**：乐观翻 UI，每帧轮询真实结果回填，永不假装成功；进行中按钮一律禁用且不推命中区。
- **深链 + 延迟切换**：托盘/悬浮窗菜单可直达指定设置页；"别处改了设置"延迟到指针空闲才应用。

### 4.3 持久化三层救捞

- 单 JSON（%APPDATA%），写 >16MB 拒绝；提权安全写入（不跟随符号链接）。
- 读三层防御：①序列化往返回路校验；②整份坏 → 备份 + journal 留痕；③半坏 → 以默认值为基底按 JSON 路径逐字段嫁接，丢的路径另存备份。
- **无版本号迁移**：按字段形状探测（读到旧字段形状就地转换）；加载后统一 clamp（interval 250..10000 等）。
- 每个失败模式都有同名测试。

### 4.4 键盘与状态反馈

- **焦点环只在 keyboard=true（用过键盘）后显示**——鼠标用户零视觉噪音，键盘用户全功能可达。CSS `:focus-visible` 零成本等价。
- 状态反馈内联化：复制→"已复制"2s；发送成功带编号；失败给替代路径（"可复制后附在反馈中"+重试钮）。
- 危险操作：行内明示后果 + 确认，能内联说明就不弹模态；"受限不可用"的开关 hint 可点击，弹出原因与修复指引。

### 4.5 我们可移植的十件事

1. live-apply + diff 写回：全局 settings store 唯一事实源，改动先 diff 再写，天然并发安全（我们有草稿模型，需统一心智）。
2. 滑块 oninput 预览 / pointerup 落库两段式。
3. 行组件数据驱动 `{label, hint?, control, visible?}`，页面=清单函数。
4. hint 行内说明优先于 tooltip；危险操作行内确认。
5. 异步开关乐观 UI + 系统回报回填 + 失败专属文案。
6. 状态反馈机（已复制 2s / 带编号 / 失败给替代路径）。
7. `:focus-visible` 焦点环 + Tab 环 + Esc 关窗 + 方向键微调。
8. 持久化三件套：单 JSON + 逐字段救捞 + 形状迁移。
9. 控件手感参数直接用作 token：thumb 240ms、开关 160ms、按钮 32px、行 60px、hint 13px。
10. 控件命中分区语义化（grip 拖拽 / 开关切换 / 整行开合并存不冲突）。

---

## 5. 贴边与触发

> 核心结论：**触发 = 「光标物理压在最外 1px + 继续向外推的 raw counts 在 250ms 窗口内累计过阈值」或「多屏接缝驻留 500ms」**；面板窗口 = 盖住整块显示器的无边框透明层，动画是绘制期内容平移——**窗口从不移动**。

### 5.1 进程模型

- 两个进程：主程序 + 提权 sensors 进程（watchdog：等待 20s / 心跳 5s / 卡死 30s）。边缘监听放提权进程的原因：**前台是提权窗口时，普通进程收不到 raw input、GetAsyncKeyState 恒读"抬起"**（UIPI）。sensors 把 `Pushed{edge, 点}` / `Buttons{按下}` 跨进程发给主程序；它挂了主程序降级自听。
- 主程序单线程拥有全部面板窗口并逐帧绘制；收起时 `WaitMessage()` 全挂起——无忙等、无 1ms 轮询。

### 5.2 触发模型（detector.rs，74 行零依赖）

- **只有 Left / Top / Right 三条热区边，没有 Bottom**（`settings.rs:184` `pub const EDGES: [Edge; 3]`）——底部是任务栏的地盘。每条边映射一个面板部件，边没配东西就完全不触发。
- **推入（相对设备：鼠标/触摸板）**：源码注释原话 "Reaching the edge is not enough: scroll bars and window borders live there. The pointer has to keep pushing into the edge after it has stopped moving."（到达边缘不够——滚动条就住在那——指针必须在停住后继续向外推。）
  - 压边判定：`Left: x<=monitor.left`、`Right: x>=monitor.right-1`、`Top: y<=monitor.top`——最外 1px，无距离带。
  - 压力累计：250ms 压力窗口（间隔超此清零重计）；`outward>0 && outward>=|along|` 才累计——**沿边滑动永不累计**（测试 `brushing_the_edge_does_not_open`：每 4ms 一次 push(1,12) 共 200 次永不触发）；向回推清零。
  - 阈值（raw mouse counts，非 px）：**Light 50 / Medium 120（默认）/ Firm 260**，再 clamp(20, 2000)。
- **驻留（绝对设备：笔/触屏/远程）**：绝对设备推不过边缘，改为**压边驻留 300ms**（DWELL）即开。
- **接缝（多屏正对边）**：指针会滑过去的边不响应推入，改为**驻留 500ms**（比边缘久："scroll bar may be beside the seam"）。带 overshoot 容差：刚跨屏冲过头 64 DIP / 1500ms 内回来仍算接缝驻留；Win11 22H2+ 指针死区缓动（注册表默认开启）整边视为 seam。接缝判定：另一屏正对且重叠。
- **防抖 armed**：触发一次即 disarm；**指针离边才 re-arm**——收回时鼠标还压在边内不会立刻重开（"推着不放是同一个手势，不是新手势"）。

### 5.3 触发输入链路（无钩子、无 DLL）

- `RegisterRawInputDevices{usUsagePage:1, usUsage:2, RIDEV_INPUTSINK}` → 专属线程上的**消息专用窗**（`HWND_MESSAGE`，类名 `"GlanceSink"`）收 `WM_INPUT`，`GetRawInputData` 取相对位移与按钮位——**不用 `WH_MOUSE_LL`**，源码注释点名理由（panel.rs:485）："Raw input 只读——不像鼠标钩子会延迟指针，且 Windows 永不静默断开它"。移植时同样别用低阶钩子：会被安全软件盯上、被系统节流、且真的延迟指针。
- 触发判定线程与 UI 线程**完全分离**：判定只碰整数与 `Instant`，从不做窗口/绘制调用；窗口操作一律排队到 UI 线程——因此**触发不依赖 webview 存活**。
- 按钮真值：raw `usButtonFlags` 位折叠实时维护；每 100ms 与 `GetAsyncKeyState` 对账，**连续 2 次不一致才采信系统值**（防安全桌面污染）。绝对指针兜底：100ms `GetCursorPos` 轮询，发现"有移动但没听到 raw motion"按驻留路径处理；**16ms 定时器按需挂载**（仅驻留/接缝计时期间，空闲即杀）。

### 5.4 防误触清单

按键按住不触发 · 光标隐藏（`CURSOR_SHOWING` 为假）不触发 · **角落两端各 48 DIP 排除带**（"标题栏按钮和任务栏的高度，它们拥有角落"）· 接缝改驻留 · 全屏游戏默认不触发（见 5.8）· UAC/锁屏（另一桌面接管输入）直接跳过。

### 5.5 面板几何

- **窗口 = 整块显示器**：无边框透明层盖满 monitor（floating 时盖整个虚拟桌面），面板只是其中绘制的矩形；**滑出屏幕的部分被窗口边界自然裁掉**。非面板区域每帧切换 `WS_EX_TRANSPARENT` 点击穿透。
- 贴边坐标用 **monitor rect + inset**（Paper 0 / Fluent 12 / Glass 10 DIP）；**沿边坐标用 work rect 避开任务栏**：锚点（触发点或屏幕中心）→ 沿边居中 → clamp 进工作区。每（屏,边）一个独立 Panel 实例。
- **reach（在面板上的判定区）**= panel rect ± LEAVE_TOLERANCE 10 DIP，贴边侧延伸到屏幕边，并 ∪ 推入点 ±10（修"从边端推入、指针从未进面板被误判离开"）∪ 按钮条。

### 5.6 显隐状态机（Phase{Hidden, Open{entered, outside_since, dragging}, Closing}，不到 200 行）

- **entered 门**：指针从未到过面板（快捷键/托盘打开）时**永不因离开而关闭**。
- **定时关闭**：进入过 + 离开 reach 超 `close_delay`（**默认 200ms，上限 10s**）→ 关。
- **按下别处立即关**；**按下起于面板内则拖着走也不关**。
- **Closing 中再触发原地复活**（"出场路上被重新打开，就从当前位置捡起来"）。
- **按下别处立即关 / 按下面板内拖走不关**：两侧各记一个按压计数（`PRESSES`/`OURS`）并配对，菜单上的按压无论谁先被看见都不算"别处按压"——防"点自己菜单被当成点了别处"。
- **常驻永不自动收**：`stays() = pinned || floating || 并排模式开着 || 菜单打开`。
- Esc = 动态注册的全局热键（面板开着且未 pin 时收走全系统的 Esc）。
- **vsync 动画**：帧到期对齐 DwmGetCompositionTimingInfo 的下一刷新（高精度 waitable timer）；**首帧先画完再 show 窗口**（"never shows empty"）；桌面截图在窗口隐藏时截。

### 5.7 拖拽 / 吸附 / 撕下

- 起拖阈值 `MOVE_FROM` 8 DIP（撕下 widget 4 DIP）——位移小于它算"点按"不算拖。
- 拖动跟随 8ms 一帧；面板边缘 6 DIP 缩放带、角落 20（同时吃两边）、尺寸夹在 `(0.75, 2.0)` 倍。
- **松手吸附**：距任一**已配置**边 ≤ **24 DIP** → 吸附该边（锚点=松手中心）；否则转 floating：位置存 **0..1 归一化 share + 中心物理 px + locked**，重启恢复。
- **撕下为桌面组件**（tear_off 开启）：按住某条 lane 的标题带撕单模块，否则撕整个面板；撕下的 widget 贴边模型——松手距工作区边 < **36 DIP** 贴上（两条边都近则贴进角落）、拖离 > **52 DIP** 才脱离、近边 **64 DIP** 内先显示落点提示、**甩出屏幕自动收起**（托盘可召回）。
- **显示器热插拔 / DPI 变更**：`WM_DISPLAYCHANGE`/`WM_DPICHANGED` → scale 差 >0.001 或换屏 → 收起后按新屏重开；拖动中延迟到松手。拓扑不缓存，每次指针事件都重查，所以拔插后无需重建状态。

### 5.8 全屏 / 游戏

- 检测：独占全屏 = `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN`（一行）；无边框铺满 + 持续 present 帧（帧间隙容忍 3s）也算。
- **默认 `OverFullscreen::Shortcut`：全屏时推入/接缝都不开面板，只留快捷键**——"游戏里的推边多半是误触"。
- 关闭动画完成后 `bring_back(game)` 恢复游戏前台/还原最小化（"受保护的游戏窗口拒绝别的进程 ShowWindow，要用 SC_RESTORE"）。

### 5.9 Tauri v2 移植要点

1. **触发器**：`RegisterRawInputDevices` + `RIDEV_INPUTSINK` 后台窗（windows-rs 几十行）+ detector.rs 逐行移植（74 行零依赖），阈值 50/120/260、250ms、300ms 直接复用。**不需要提权进程**——除非要求在 UAC/提权窗前也能触发。
2. **窗口动画路线**：(a) 推荐——窗口盖整屏（或整条边），HTML 内容用 CSS transform 做滑入，WebView2 合成器自己插帧，最顺滑；非面板区 `set_ignore_cursor_events` 切换穿透。(b) 窗口只占面板大小逐帧 `set_position`——有 IPC/调度延迟，不推荐。对照我们的痛点：peek 把手窗跨边销毁重建实测 619–794ms，路线 (a) 天然消除。
3. **状态机照搬**：entered 门、reach ±10、close_delay 200ms、按下别处立即关、Closing 复活、armed 防抖——这套手感全在小状态机里，代码 <200 行。
4. **几何**：Tauri 不暴露 work area——自调 `GetMonitorInfoW`；沿边 clamp 进工作区、inset、每（屏,边）一实例、热插拔重排（scale 差>0.001）都值得照抄。
5. **全屏**：QUNS 一行检测；默认只留快捷键（glance 的默认值就是踩坑后的结论）。
6. **接缝**：仅多显示器需要；单屏整个跳过。判定依据：另一屏正对边落在同一直线且沿线投影有非空重叠 → 该段是接缝；Win11 22H2+（build ≥ 22557 且 `HKCU\Control Panel\Cursors\CursorDeadzoneJumpingSetting≠0`，缓存 1s）开启"显示器之间缓动指针"时，指针会越过接缝继续走，**整条边都按接缝处理**（否则多屏用户会觉得"推边缘没反应"）。接缝段：zone 8 DIP + 静默抖动 8 DIP + 越缝宽限 64 DIP/1500ms，驻留 500ms 呼出，一次驻留只呼一次。
7. **必须写的 Win32 原生代码**（Tauri 配置层不暴露的）：`GetMonitorInfoW().rcWork`（工作区）· `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`（每屏缩放）· `EnumDisplayDevicesW().DeviceID`（按屏独立配置）· `SHQueryUserNotificationState()`（全屏）· **`WS_EX_NOACTIVATE`（+ 可选 `WS_EX_TOOLWINDOW`）——面板不抢焦点**，需在 Rust 里 `SetWindowLongPtr`；`RegisterHotKey`（`MOD_NOREPEAT`）。
8. **判定放 Rust、渲染放前端**：命中区/reach 在 Rust 侧算（DIP × scale → 物理 px），Svelte 只按 `panel/reach/rack` 四个矩形做 hover 与点击——避免前端算错缩放。
9. **帧率**：Web 端 CSS/rAF 天然 vsync，无需 DWM 时序那套；关键是**收起后停一切定时器**（glance：WaitMessage 挂起，16ms 定时器按需挂载）。

---

## 6. 对照我们的现状：差距清单

### 6.1 视觉

| # | 差距 | glance 参照 | 我们现状 |
|---|---|---|---|
| V1 | 仅暗色一套 token | 深/浅双套 + 壁纸亮度自动 | `tokens.css` 无 light 分组、无 prefers-color-scheme |
| V2 | 字体 4.6MB 未裁剪 | latin 子集 ~140KB + 中文系统回退 | MiSans VF 56 分片全量 |
| V3 | 无图标体系 | Lucide 单色线性 46 枚 | 字符按钮 ↻⚙⤢✕ |
| V4 | 玻璃壳三处手写漂移 | 单一 skins 模块 | App/Settings/TrendWindow 各写一份 blur16 |
| V5 | 无材质档位 | Glass/Acrylic/Paper 整套 | 高不透明假玻璃 rgba .92 |
| V6 | Settings.svelte 第二套视觉语言 | 单一 theme 数据源 | rt-* 外来色 + 越界字号 |

### 6.2 动效

| # | 差距 | glance 参照 | 我们现状 |
|---|---|---|---|
| M1 | 第二套动效语言 | 全项目两组曲线 | Settings rt-*（.32/.28/.16s 过冲曲线）不走 token |
| M2 | 魔法数未接线 | 常数全部集中 | 420/260/180ms 组件内重复，`--tum-dur-*` 已有未引用 |
| M3 | 数值跳变硬切 | 数字是核心视觉 | tabular-nums 只防抖动不补间 |
| M4 | 把手跟随瞬移 | 触发→reveal 全程可中断续走 | set_position 直跳 |
| M5 | 拖拽落位跳变 | 松手飞回槽位 200ms | pill-drag-settled 直接跳位 |
| M6 | 页签/窗口切换硬切 | 布局形变 morph 260ms | 设置窗开关无动效、内容硬切 |

### 6.3 设置窗口

| # | 差距 | glance 参照 | 我们现状 |
|---|---|---|---|
| S1 | 关闭即保存 | live-apply + 三路合并 | ✕ 无条件保存，无未保存确认 |
| S2 | 即时项/草稿项混排 | 心智统一（全部改即存） | 同页两种生效模型 |
| S3 | 无搜索/命令面板 | 无搜索但全键盘可达 | 无搜索也无全键盘 |
| S4 | 位置不持久化 | 窗口几何全持久化 | 每次居中 |
| S5 | 凭据/cardOrder 不在 dirty 检测 | dirty 语义完整 | hasDraft 缺 credsDirty/localStorage 项 |
| S6 | 导出无入口 | 深链 + 托盘直达 | 后端 export.rs 在但无 UI |

### 6.4 贴边

| # | 差距 | glance 参照 | 我们现状 |
|---|---|---|---|
| D1 | Bottom 边缺失的定位 | **glance 同样只有 Left/Top/Right 三边**（settings.rs:184 `EDGES`）——不是缺陷是共识，我们应把 Bottom 从"未修"改为明确的设计决策 | dock.rs:172-179 自注"任务栏异常未修" |
| D2 | 无 DPI/热插拔监听 | WM_DISPLAYCHANGE/WM_DPICHANGED → scale 差>0.001 或换屏 → 收起后按新屏重开 | 无 ScaleFactorChanged 监听，靠下次 Moved 自愈 |
| D3 | along 绝对偏移 | floating 用 0..1 归一化 share + 物理中心持久化，重启恢复 | 换矮屏靠夹回兜底，无比例锚定 |
| D4 | 无"有意识的推"判定 | raw counts 压力累计（250ms 窗口、默认阈值 120）+ 绝对设备驻留 300ms | 鼠标悬停 reveal + 320ms 延迟——任何停留都呼出 |
| D5 | 无 armed 防抖 | 收回时指针仍压边内→不重开，**指针离边才 re-arm** | 需核对（scheduleDockPill 320ms 可能重触发） |
| D6 | 无 entered 门 | 指针从未到过面板（快捷键/托盘打开）→永不因离开关闭 | 需核对 |
| D7 | 无拖动吸附语义 | 松手距已配置边 ≤24 DIP 吸附 + anchor=松手中心 + floating 归一化持久化 | 松手才判边（16 物理 px snap margin） |

---

## 7. 落地路线图

### P0 快赢（参数级，1–2 天）

1. **动效 token 收口**：删 Settings rt-* 第二套曲线，全部接 `--tum-ease-*`/`--tum-dur-*`；组件内 420/260/180 魔法数替换为 var()。
2. **微交互手感对齐 glance**：开关 160ms + 按住拉伸；分段选择 thumb 240ms 滑动；按钮 hover 底色 6%/4%；穿透/悬浮按钮 500ms LINGER 防误触。
3. **数字补间**：核心数值（token 用量/费用）用 `tweened` 滚动 + tabular-nums，上升瞬时、下落限速（借频谱柱"显示值"思想）。
4. **把手/落位动画**：`pill-drag-settled` 后 200ms easeOutCubic 飞回槽位（FLIP），替代跳位；把手跟随用 rAF 指数趋近 τ=60ms。
5. **设置窗关闭拦截**：hasDraft>0 时 ✕ 弹"未保存更改"行内确认（保存/放弃/取消）。
6. **`:focus-visible` 全局焦点环** + Esc 关窗 + Tab 环核对。
7. **图标体系**：引入 lucide-svelte，替换字符按钮（单色线性、stroke 2、随文字色）。

### P1 体验升级（1–2 周）

1. **浅色主题 + 自动切换**：token 分组 light/dark；Rust 侧取壁纸算 Rec.709 均值 ≤0.5 判暗色（glance `is_dark(Backdrop)` 同款），存偏好（Light/Dark/Follow）。
2. **设置窗搜索**（数据驱动行清单后免费获得）+ 导航图标 + "恢复默认" + 导出入口（export.rs）。
3. **窗口位置持久化**：settings/trend/tool 三窗位置记忆（复用 persist_placement 思路）。
4. **DPI/热插拔**：监听 ScaleFactorChanged + 显示器增删 → 重贴边 + along 比例锚定（along 存为工作区宽度比例）。
5. **修复 Bottom 边**（任务栏交互异常根因排查）。
6. **live-apply 心智统一**：即时项分节明示（"立即生效"分组标题），或全部改三路合并 live-apply；即时项写失败回滚 UI。
7. **状态反馈机**：复制/保存/失败内联文案规范化（含失败替代路径）。
8. **字体裁剪**：MiSans latin 子集 + 中文系统回退，4.6MB → 预计 <1MB。

### P2 结构升级（有余力时）

1. **贴边触发升级**（对照 §5.9）：Rust 侧注册 raw input（`RegisterRawInputDevices` + `RIDEV_INPUTSINK`，几十行）+ 移植 glance detector.rs（74 行零依赖）：压力累计（250ms 窗口、阈值 50/120/260）、绝对设备驻留 300ms、armed 防抖、entered 门、close_delay 200ms、按下别处立即关；多屏才加 seam 驻留 500ms；全屏 `QUNS_RUNNING_D3D_FULL_SCREEN` 一行检测，默认只留快捷键。
2. **窗口动画路线抉择**：仿 glance"窗口盖整条边 + 内容 CSS transform 滑入 + `set_ignore_cursor_events` 穿透"，窗口永不逐帧移动——直接消除我们 peek 把手窗跨边销毁重建 619–794ms 的痛点；同步解决窗口级入场/退场动画。
3. **自适应磨砂**：透明窗 backdrop-filter 采不到桌面是硬约束（App.svelte:1694 自注）——路线：窗口级 vibrancy（window-vibrancy acrylic/mica）+ 前端只放 tint 与 inset 高光；frost 按"内容密度"而非屏幕采样近似。
4. **布局 morph**：面板列数/模式切换用 Svelte keyed each + FLIP 260ms 四次幂 easeOut，文字容器锁宽高比防压字。
5. **帧率纪律**：Rust 推送节流与前端 rAF 调度统一（busy 聚合），收起后停一切定时器（glance：WaitMessage 挂起）。

### 明确不学的（防过度工程）

- 纯 Rust 手绘 UI 路线（我们是 Web 栈，学参数不学实现）。
- 位移图玻璃弯折（CSS 无解，SVG feDisplacementMap 性能代价高；用"边缘更亮更透"暗示即可）。
- 三皮肤整套系统（先做 light/dark + 材质档位）。
- 歌词物理弹簧等专项动效（与我们场景无关，但参数表留档）。

---

## 附录 A：glance 全部常数速查表

### 动效
| 名称 | 值 | 出处 |
|---|---|---|
| OPEN | 260ms, c-b(0.16,1,0.3,1) | panel.rs:97-98 |
| OPEN_FADE | 140ms linear | panel.rs:99 |
| CLOSE | 180ms, c-b(0.4,0,1,1) | panel.rs:100 |
| PLAIN_FADE | 120ms | panel.rs:101 |
| SCROLL_EASE | 0.06s（τ） | panel.rs:102 |
| MORPH/SETTLE | 260ms, 1-(1-t)⁴ | widget.rs:349, morph.rs:468 |
| thumb EASE | τ=40ms（exp25） | thumb.rs |
| thumb SLIDE | 220ms cubicOut | thumb.rs |
| SHIFT（重排） | 200ms cubicOut | thumb.rs |
| GLIDE（歌词） | 420ms cubicOut | playing.rs:72 |
| 歌词弹簧 | (142, 28, 0.82) 子步 1/240s | music.rs:729 |
| SPECTRUM_FALL | ≤1.7/s 线性 | music.rs:263 |
| CONTROLS_FADE | 180ms | music.rs:265 |
| LINGER | 500ms | buttons.rs |
| 设置 SLIDE/FLIP/GLIDE/SETTLE | 240/160/200/180ms | settings_window.rs |
| 页签下划线 | lead 200ms / trail 360ms | settings_window.rs:4239 |

### 视觉
| 名称 | 值 |
|---|---|
| Ink 阶梯 | 100% / 55% / 34%→0（wash 渐隐） |
| hover | 暗白6% / 亮黑4% |
| figure | wght 600–620, wdth 66%, tracking -0.01~-0.035em, line-height 0.84 |
| 玻璃 | blur 9px, saturate 1.3, FROST_BLUR=9, BEND=16, RIM=40 |
| Acrylic | blur 30px, saturate 1.25, luminosity 0.96(d)/0.85(l) |
| frost | CALM=0.03, BUSY=0.14, CONTRAST_SPAN=0.35, glass_clear 0.16(d)/0.08(l) |
| rim stops | α 0.75/0.2/0.05/0.08/0.22, 宽 1.25 DIP |
| inset 三件套 | 顶 (1,2,白.5) / 底 (-1,2,白.12) / 全周 (0,18,白.07) |
| 阴影（Paper 深） | 0 18px 40px -12px .6 + 0 2px 6px .3 |
| 度量 | 列 356 / 标签 96 / 行 16 / 表行 22 / 按钮高 32 |
| 圆角 | 24 / 16 / 8 / 4 / 胶囊 min(r, h/2) |
| 壁纸判暗 | Rec.709 均值 ≤ 0.5 |
| 图标 | Lucide stroke 2, 尺寸吸附偶数像素 |

### 触发/边缘（完整；几何常数单位均为 DIP，运行时乘 `scale`=DPI/96 转物理像素）
| 名称 | 值 | 出处 |
|---|---|---|
| 压力阈值 | Light 50 / Medium 120（默认）/ Firm 260 raw counts，clamp(20,2000) | settings.rs:628, push.rs:33 |
| PRESSURE_WINDOW | 250ms（间隔超此清零重计；往回拉立刻清零） | detector.rs:12 |
| DWELL（绝对设备驻留） | 300ms；且仅在"自上次轮询无任何 raw 鼠标包"时生效 | detector.rs:14, push.rs:86 |
| 热区边 | 仅 Left/Top/Right，无 Bottom | settings.rs:184 |
| 触发深度 | 1 物理像素（指针须真压在屏幕边框那一行） | mouse.rs:127-129 |
| 接缝 | 驻留 500ms / 静容差 8 DIP / zone 8 DIP / overshoot 64 DIP / 跨屏窗 1500ms | panel.rs:72-76 |
| Ease-cursor 判定 | build ≥ 22557 且 `HKCU\Control Panel\Cursors\CursorDeadzoneJumpingSetting≠0`（缓存 1s） | screens.rs:17-19, 66-81 |
| CORNER_EXCLUSION | 两端各 48 DIP 排除带（标题栏按钮高 / 任务栏高） | mouse.rs:17 |
| LEAVE_TOLERANCE | 10 DIP（并上：贴边侧延伸到屏边 ∪ 推入点 ±10 ∪ 按钮架） | panel.rs:65, 3952-3981 |
| close_delay | 默认 200ms，上限 10s；点别处则不等它 | settings.rs:857/977, panel.rs:2477 |
| DOCK 吸附 | 距已配置边 ≤24 DIP，anchor=松手中心 | panel.rs:1697, 4015-4022 |
| 拖动 | 跟随 8ms；缩放带 6 / 角 20 / 起拖 8 DIP（widget 4）/ 尺寸 0.75–2.0 倍 / 按钮架内缩 6 DIP | panel.rs:102/106/1685-1693, 3908 |
| 撕下 widget | 贴上 36 DIP / 脱离 52 DIP / 落点提示 64 DIP | widget.rs:201-202, 361 |
| 轮询 | 驻留 16ms / 兜底 100ms / 设备上报 50ms | panel.rs:82/87, touches.rs |
| 全屏 | QUNS_RUNNING_D3D_FULL_SCREEN；present 帧间隙 3s；默认 OverFullscreen::Shortcut | panel.rs:4111, 197, 601-610 |
| 动画 | 同 §3.2（260/140/180/120ms） | panel.rs:98-106 |

## 附录 B：研究方法与信源

- 全部结论摘自 commit `6974ed4b07af25765777b2b9ea4f4350d7763de7` 的源码（raw.githubusercontent.com 逐文件抓取）。
- 关键文件与体量：`src/panel.rs`(221KB) `src/widget.rs`(226KB) `src/ui/view.rs`(381KB) `src/ui/settings_window.rs`(296KB) `src/ui/beside.rs`(150KB) `src/ui/music.rs`(203KB) `src/settings.rs`(62KB) 等。
- 我方对照数据：`src/styles/tokens.css`、`src/App.svelte`、`src/Settings.svelte`(3882 行)、`src-tauri/src/{ipc.rs,dock.rs,lib.rs,settings.rs}`（行号以 2026-10-23 工作区为准）。
