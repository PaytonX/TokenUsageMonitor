# 日志约定

后端统一用 `tracing`。本文件只约定两件事：**target 命名**和**什么时候该打什么级别**。

## 怎么打开日志

subscriber 在 `src-tauri/src/lib.rs` 的 `run()` 里初始化，过滤器取自环境变量 `RUST_LOG`：

```rust
EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"))
```

也就是说 **不设 `RUST_LOG` 时就是 `info`**。后果要清楚：
`ErrorDeduper` 抑制重复错误时打的那条 `trace!`（`tum.notify`）在默认配置下**看不到**，
它只在主动调级时出现。这是有意为之——它每 300 秒一次，info 级别下纯属噪音。

PowerShell 里临时打开某个子系统：

```powershell
$env:RUST_LOG = "info,tum.poll=debug"
npm run tauri dev
```

只想看轮询、其余全静：

```powershell
$env:RUST_LOG = "warn,tum.poll=trace"
```

## target 约定

target 一律 `tum.<子系统>`，与代码里的模块**不对应**，而是与"出问题时你会去查哪一块"对应。
这么分是因为本项目里同一个模块横跨多个关注点：`scheduler.rs` 同时管轮询、燃烧率和设置唤醒，
按文件分组会让 `RUST_LOG=tum.scheduler=trace` 变成一把抓不出重点的钝器。

| target | 覆盖什么 | 排查场景 |
| --- | --- | --- |
| `tum.poll` | `scheduler.rs` 的轮询与轮询失败 | 某个账号不更新、请求在发但没回 |
| `tum.provider` | 各 provider 的 API 解析 | 接口改了格式、返回体形状变化 |
| `tum.window` | Tauri 窗口创建 / 位置 / 置顶 | 窗口弹不出来、位置跳变 |
| `tum.notify` | 阈值通知与 `ErrorDeduper` 抑制 | 通知不弹、错误提示刷屏 |
| `tum.exchange` | 汇率拉取与回退 | 金额显示不对 |
| `tum.hub` | 局域网同步（计划中） | 两台机器对不上 |
| `tum.local` | 本地工具扫描（计划中） | 本地用量扫不到 |
| `tum.burn` | 燃烧率计算（计划中） | 燃烧率异常跳变 |

标了「计划中」的三项目前没有调用点。占位是为了让 target 空间先稳定下来：
新日志一律从这张表里取名字，不要临时发明 `tum.ipc`、`tum.tokio` 之类的。

## 级别

| 级别 | 用于 | 例子 |
| --- | --- | --- |
| `error!` | 需要人介入、状态可能已经错了 | 凭据写进 keyring 失败 |
| `warn!` | 某次操作失败但流程继续、结果可接受 | 单个 provider 轮询失败、汇率回退 |
| `info!` | 少见但值得留痕的状态迁移 | 启动时读到的配置摘要 |
| `debug!` | 排查单次请求需要、量大但可预期的细节 | 热力图聚合出多少个格子 |
| `trace!` | 高频、按设计会重复的事件 | 去重器抑制了重复错误 |

判据是**频率 × 可行动性**：高频且无需动作的用 `trace`，低频但会让人疑惑的用 `warn`。
拿不准时先问"这条日志出现一百次，我会不会想关掉它"。

## 字段

优先用结构化字段而不是把值拼进消息字符串——过滤和聚合靠的是前者：

```rust
// 好：能按 provider 过滤
tracing::warn!(target: "tum.poll", provider = %id, error = %e, "poll failed");

// 差：只能全文搜索
tracing::warn!(target: "tum.poll", "poll failed for {id}: {e}");
```

约定：`provider` 恒为 provider id（不是展示名，展示名会变、会重复）；
`error` 恒为 `Display` 串；数量用 `=`，字符串用 `%=`。

## 不打的东西

- **token、key、cookie**。哪怕是 `trace!` 也不打。凭据只能经 keyring 走，见 `signing.rs`。
- **完整的 API 响应体**。`volcengine.rs` 只记 `body_chars` 长度，不记内容——
  响应里可能带账号标识，而且出了长度信息基本都用不上。
- **每轮的窗口数值**。那是快照的职责，日志只关心"这次请求发生了什么"。
