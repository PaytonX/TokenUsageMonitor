# glance P0 之后的跟进清单

本轮（`feat/glance-p0-polish`，8 个代码 commit）把 glance 那套「细节决定质感」的做法
落到了参数级别的第一批：动效 token 收口、图标体系、按压与悬浮反馈、数字补间、
全局焦点环、设置关闭确认、贴边落位的指数趋近滑动，以及合并前终审揪出的
四处数据安全／协议缺口。

下面这些是实施与审查过程中发现、但**故意不放进本轮**的问题。它们不阻塞合并，
按「值得做但要看时机」排列；每条都写了当初为什么不修，避免以后重新争论。

已在本轮修掉的（留档备查，不必重做）：关闭路径忽略保存失败、`closeConfirm` 不复位、
丢弃失败无声、Alt+F4 绕过确认条、`set_pill_dragging` 不推进滑动代际、
`snap_to_edges` 不推进代际、Settings 专项门禁缺失、两个 `<select>` 的双层焦点环、
孤儿 token 注释说谎、design/plan 文档漂移。

---

## 1. 设置窗口

| 项 | 位置 | 为什么不本轮修 | 建议修法 |
| --- | --- | --- | --- |
| 取消确认后焦点不归位 | `src/Settings.svelte` 确认条 | 需要记住「确认条出现前的 activeElement」，收益小 | 打开确认条时记 `document.activeElement`，取消/关闭时 `.focus()` 回去 |
| Settings 窗无 Esc 关闭 | `src-tauri/src/ipc.rs` 的 `settings.html` | 前端 `svelte:window` 的 Esc 若直接接上，会与输入法组合键冲突 | 加 `isComposing` / `keyCode === 229` 守卫后绑定 `Esc → requestClose()` |
| `removeRoot` 的 icon-only 按钮套了 `.btn` 文本类 | `src/Settings.svelte` | 只是语义不精确，视觉无差 | 抽 `.btn--icon` 变体 |
| `Settings.svelte` 其余规则仍不参与 design-tokens lint | `src/lib/design-tokens.test.ts` | 只补了「禁用色 + 半像素字号」两类专项断言，其余规则一开就是几十条历史问题 | 逐规则启用（先 `FORBIDDEN_PX` 的圆角，再字号） |
| 页脚「保存并关闭」与确认条共用 `saving` 闸 | `src/Settings.svelte` | 已由 `handleClose` 单点守卫覆盖，无需再改 | — |

## 2. 动效与微交互

| 项 | 位置 | 为什么不本轮修 | 建议修法 |
| --- | --- | --- | --- |
| 按压深度未 token 化 | 11 处 `:active` 规则 | 数值集中在 0.94/0.9 两档，抽 token 不改变产物 | 加 `--tum-press-scale: 0.94` 与 `--tum-press-scale-close: 0.9` |
| `transition: all` 与显式段并存 | `Settings.svelte` 若干处 | 当前行为正确（同属性取末匹配），清理属重构 | 显式列出属性 |
| `.dev__refresh` 常态底误用 hover token | `src/lib/components/DevicePanel.svelte` | 视觉上只差 0.005 alpha | 换回 `--tum-surface` |
| hover 透明度无测试护栏 | `design-tokens.test.ts` | 现有 lint 只查 hex/rgba 字面量 | 断言 `--tum-surface-hover` < `--tum-surface-hover-raised` 且都 > `--tum-surface` |
| `docs/FRONTEND-DESIGN-STYLE.md` 仍写单档 0.085 | 该文档 :32 / :188 | 文档滞后于实现 | 改成两档 0.06 / 0.09 并补说明 |
| `atoms/index.ts` 末行无换行 | `src/lib/components/atoms/index.ts` | 既有状态 | 顺手加 |
| `TrendPanel.svelte` 注释仍讲 `⤢` 字形 | 该文件 :3 / :192 | 注释散文 | 改为「图标按钮」 |

## 3. 数字补间

| 项 | 位置 | 为什么不本轮修 | 建议修法 |
| --- | --- | --- | --- |
| `reset()` 只改显示值、不清动画态 | `src/lib/motion/num.ts` | 现有调用序列都是「reset 后先跟一步贴住新值」，实际行为正确；改语义要重新推演 10 个用例 | 要么在 `reset` 里 `falling = false`，要么在文档注释里写清「reset 后必须再 step 一次」 |
| `Stat` 的 `?? 25` 与 `num.ts` 的 `DEFAULTS.fallLimitPerSec = 40` 双默认 | `Stat.svelte` / `num.ts` | 为保原行为而保留；当前只有 `DevicePanel` 用 `animate` 且显式传 `25_000` | 统一成一个默认值，或强制 `animate` 模式必须传 `fallLimitPerSec` |
| 只接了 `DevicePanel` 的两处聚合数字 | 各面板的 Stat 调用点 | 其余调用点的原始 number 不在作用域内（已格式化成字符串），接线要改数据流 | 逐面板改造：把 `fmtTokens(x)` 挪进 `format` prop |
| 补间无「首帧从当前值续走」的视觉回归测试 | — | 需要组件渲染设施 | 若要测，用 `@testing-library/svelte` + 假 rAF |

## 4. 贴边与滑动落位

| 项 | 位置 | 为什么不本轮修 | 建议修法 |
| --- | --- | --- | --- |
| F6：模式切换瞬间的迟到写 | `src-tauri/src/ipc.rs` glide 线程 | 协作式取消 + `SWP_ASYNCWINDOWPOS` 队列，人手几乎不可命中；彻底封口要改线程生命周期 | 三选一：存 `JoinHandle` 并等待退出 / 几何改写前 `sleep(GLIDE_STEP_MS + 2ms)` / 改写后隔一帧补写一次 |
| `reposition_peek` 的滑动分支收益≈0 | `src-tauri/src/ipc.rs` | `along` 不变即早退；真有 gap 时每 `Moved` 重启会让把手抖 2~78px | `along` 变化走瞬时，或按 target 去重后再启动 |
| 每次 dock 落盘 2 次 | `persist_placement` 调用点 | 已挪出关键路径（后台串行写盘） | 合并成一次 |
| glide 步序列未抽纯函数 | `dock.rs` 用例 | 现有 4 个用例覆盖首步/收敛/吸附；其中 2 例断言等价 | 抽 `glide_positions(from, to)` 让 cap/步数/单调性可测 |
| `perf_probe` 的 `anchor_ms` 语义漂移 | `perf_probe.rs` | 从「已就位」变成「已派发」 | 文档注明，或改标 `anchor_dispatch_ms` |

## 5. 其它（跨窗口一致性）

- 三处手写玻璃壳（`blur(16px)`）仍各写各的，未收口成 token 组合。
- 字体仍是 4.6MB 的 MiSans 56 分片，未按 glance 的做法裁 latin 子集 + 系统中文回退。
- 没有「跟随系统深浅色」的实际切换路径（tokens.css 只有暗色一套）。
- `Settings.svelte` 3921 行、`App.svelte` 2200+ 行的巨石组件本身是下一轮的最大成本项。
