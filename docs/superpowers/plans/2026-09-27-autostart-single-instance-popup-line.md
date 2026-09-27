# 开机自启 + 单例模式 + 弹窗上边线修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 TokenUsageMonitor 增加开机自启设置项与单例模式，并消除 trend/tools/settings 弹窗上边缘的黑/白描边。

**Architecture:** 自启与单例均采用 Tauri 2 官方插件：tauri-plugin-autostart 写 HKCU Run 键（仅当前用户，默认关闭）；tauri-plugin-single-instance 注册在 builder 链最前，第二实例启动时回调在第一实例内执行（聚焦 dashboard），随后第二实例自动退出。autostart 状态持久化于现有 config.toml（SettingsStore），替换遗留字段 autostart_hint_shown（serde 默认忽略未知字段，旧配置天然兼容）。弹窗上边线根因：三个窗口 builder 缺 `.shadow(false)`（透明无边框窗口默认带 1px 系统描边），且只在创建时调用一次 `disable_corner_artifacts`（Win11 会在尺寸变化后重画 DWM 边框），复用 dashboard 现成的 Resized 重设模式修复。

**Tech Stack:** Tauri 2（tauri-plugin-autostart v2 / tauri-plugin-single-instance v2）、Rust（tokio / serde / toml）、Svelte 5（$state runes）、TypeScript

**设计文档:** `docs/superpowers/specs/2026-09-27-autostart-single-instance-popup-line-design.md`

**执行者须知（本机工具约束）:**
- Edit/Write 工具对 `D:\` 路径会拒绝：所有写入/替换改用「工作树内编辑 → PowerShell 复制回 D 盘」的流程完成，替换必须断言命中次数，次数不符立即中止并报告。
- 每次修改前先 Read 目标区域，以内容为准（行号基于提交 a6efc34，可能漂移）。
- 提交时只 add 本计划涉及文件；工作区中 `src/lib/components/ProviderLogo.svelte` 仅为行尾符差异，`release/`、`docs/mockups/` 为既有未跟踪项，均不得带入。

**文件影响面:**

| 文件 | 动作 | 内容 |
| --- | --- | --- |
| `src-tauri/Cargo.toml` | 修改 | +2 插件依赖 |
| `src-tauri/src/settings.rs` | 修改 | autostart 字段替换遗留字段 + Default + 测试 |
| `src-tauri/src/lib.rs` | 修改 | 注册两个插件 + setup 自愈 + invoke_handler |
| `src-tauri/src/ipc.rs` | 修改 | set_autostart 命令 + save_settings 对账 + 三弹窗边线修复 |
| `src/lib/types.ts` | 修改 | Settings 接口字段替换 |
| `src/Settings.svelte` | 修改 | 状态/读取/保存/开关行 |
| `docs/superpowers/plans/2026-09-27-autostart-single-instance-popup-line.md` | 新建 | 本计划 |

---

### Task 1: settings.rs — autostart 字段（TDD）

**Files:**
- Modify: `src-tauri/src/settings.rs`（字段 34–37、Default 146、既有测试 304–319、测试模块末尾）

- [ ] **Step 1: 写失败测试**

在 `mod settings_defaults_tests` 内、`new_fields_round_trip` 测试的右花括号（367 行）之后、模块右花括号之前插入：

```rust
    #[test]
    fn autostart_field_parses_and_round_trips() {
        let raw = "\
enabled_providers = []
poll_interval_seconds = 60
autostart = true
";
        let parsed: Settings = toml::from_str(raw).expect("parse config with autostart");
        assert!(parsed.autostart);

        let dumped = toml::to_string(&parsed).expect("serialize settings");
        let reparsed: Settings = toml::from_str(&dumped).expect("reparse dumped settings");
        assert!(reparsed.autostart);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test autostart_field_parses_and_round_trips`（cwd: `src-tauri`）
Expected: 编译失败，错误指向 `parsed.autostart`（`no field 'autostart'`）——字段尚不存在，即 TDD 的"红"。

- [ ] **Step 3: 替换字段定义**

将（34–37 行）：

```rust
    /// Whether to autostart on system boot (informational; the user must add
    /// a shortcut to shell:startup themselves for now).
    #[serde(default)]
    pub autostart_hint_shown: bool,
```

替换为：

```rust
    /// Whether to launch automatically at system boot (Windows: HKCU Run key,
    /// managed via tauri-plugin-autostart).
    #[serde(default)]
    pub autostart: bool,
```

- [ ] **Step 4: 更新 Default 实现**

146 行 `autostart_hint_shown: false,` → `autostart: false,`

- [ ] **Step 5: 强化既有兼容测试**

在 `old_toml_without_new_fields_gets_defaults` 的末尾断言（318 行 `assert!(parsed.proxy_url.is_none());` 之后）追加一行：

```rust
        assert!(!parsed.autostart);
```

说明：raw TOML 中的 `autostart_hint_shown = false` 行（311 行）保留不动——它模拟旧版 config.toml 含已删除字段，serde 默认忽略未知字段；该测试同时守护「旧配置可加载」与「新字段缺省 false」。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test autostart`（cwd: `src-tauri`）
Expected: `autostart_field_parses_and_round_trips ... ok`、`old_toml_without_new_fields_gets_defaults ... ok`，0 failed。
（此时 lib.rs/ipc.rs 尚未引用新字段，crate 可独立编译。）

---

### Task 2: Cargo.toml — 两个插件依赖

**Files:**
- Modify: `src-tauri/Cargo.toml`（24 行 `tauri-plugin-window-state = "2"` 之后）

- [ ] **Step 1: 插入依赖**

在 `tauri-plugin-window-state = "2"` 之后插入两行：

```toml
tauri-plugin-autostart = "2"
tauri-plugin-single-instance = "2"
```

- [ ] **Step 2: 验证依赖可解析**

Run（cwd: `src-tauri`，两条分别执行）:

```
cargo tree -p tauri-plugin-autostart --depth 0
cargo tree -p tauri-plugin-single-instance --depth 0
```

Expected: 各输出一行形如 `tauri-plugin-autostart v2.x.y` / `tauri-plugin-single-instance v2.x.y`，无解析错误。

---

### Task 3: lib.rs — 单例插件（builder 链最前）

**Files:**
- Modify: `src-tauri/src/lib.rs`（230–231 行）

- [ ] **Step 1: 插入 single-instance 插件**

将：

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
```

替换为：

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二实例启动时本回调在第一实例内执行，第二实例进程随后由插件
            // 自动退出；面板此前收进托盘也能被唤起。
            if let Some(dash) = app.get_webview_window("dashboard") {
                let _ = dash.unminimize();
                let _ = dash.show();
                let _ = dash.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
```

说明：`get_webview_window` 来自 `tauri::Manager`。lib.rs setup 已使用 `app.path()`，说明 `Manager` 已在作用域；若编译报 method not found，在文件头部 `use tauri::{...}` 中补 `Manager`。

- [ ] **Step 2: 编译验证**

Run: `cargo check`（cwd: `src-tauri`）
Expected: 编译通过（首次会编译两个新插件及其依赖，耗时数分钟属正常）。

---

### Task 4: lib.rs — autostart 插件 + 启动自愈

**Files:**
- Modify: `src-tauri/src/lib.rs`（235 行 process 插件之后；256–257 行 settings_store 创建之后）

- [ ] **Step 1: 注册 autostart 插件**

在 `.plugin(tauri_plugin_process::init())` 之后插入：

```rust
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
```

- [ ] **Step 2: setup 钩子加自愈逻辑**

在：

```rust
            let settings_store = settings::SettingsStore::new(data_dir)
                .expect("loading settings store");
```

之后插入：

```rust

            // 自启自愈：config.toml 声明要自启但注册表项缺失（用户手动删除、
            // 换路径安装等）时重新写入，保证设置与系统状态最终一致。
            {
                use tauri_plugin_autostart::ManagerExt;
                let s = settings_store.read_blocking();
                if s.autostart {
                    let autolaunch = app.autolaunch();
                    if !autolaunch.is_enabled().unwrap_or(false) {
                        let _ = autolaunch.enable();
                    }
                }
            }
```

- [ ] **Step 3: 编译验证**

Run: `cargo check`（cwd: `src-tauri`）
Expected: 编译通过。

---

### Task 5: set_autostart 命令 + save_settings 对账 + 命令注册

**Files:**
- Modify: `src-tauri/src/ipc.rs`（832 行旧值捕获、843 行镜像 flags 之后、928 行 save_settings 结束后）
- Modify: `src-tauri/src/lib.rs`（622 行 invoke_handler 列表）

- [ ] **Step 1: save_settings 捕获旧值**

将：

```rust
    let old_proxy_url = state.settings.get().await.proxy_url.clone();
```

替换为：

```rust
    let old_settings = state.settings.get().await;
    let old_proxy_url = old_settings.proxy_url.clone();
    let old_autostart = old_settings.autostart;
```

- [ ] **Step 2: save_settings 对账自启**

在：

```rust
    state.edge_snap.store(new_settings.edge_snap, std::sync::atomic::Ordering::SeqCst);
```

之后插入：

```rust

    // 自启开关与注册表即时对账：仅当该字段实际变化时才触碰 HKCU Run 键，
    // 其余保存走快速路径。
    if old_autostart != new_settings.autostart {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        if new_settings.autostart {
            autolaunch.enable().map_err(|e| e.to_string())?;
        } else {
            autolaunch.disable().map_err(|e| e.to_string())?;
        }
    }
```

- [ ] **Step 3: 新增 set_autostart 命令**

在 save_settings 函数结束（928 行 `Ok(())` + `}`）之后插入：

```rust

/// 独立切换开机自启：更新注册表（tauri-plugin-autostart）并持久化到
/// config.toml。常规设置页保存走 save_settings 的对账路径；本命令作为
/// 专用入口保留（托盘菜单等场景复用）。
#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| e.to_string())?;
    } else {
        autolaunch.disable().map_err(|e| e.to_string())?;
    }
    let mut settings = state.settings.get().await;
    settings.autostart = enabled;
    state.settings.save(settings).await.map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 4: invoke_handler 注册**

在 `ipc::refresh_exchange_rates,` 之后插入一行：

```rust
            ipc::set_autostart,
```

- [ ] **Step 5: 编译验证**

Run: `cargo check`（cwd: `src-tauri`）
Expected: 编译通过。

---

### Task 6: ipc.rs — 三弹窗上边线修复

**Files:**
- Modify: `src-tauri/src/ipc.rs`（open_settings builder 705–711、open_trend_window builder 753–759、open_tool_window builder 789–795）

两处编辑各自在文件中恰好出现 **3 次**（settings/trend/tools 各一次），用全局替换并断言命中数 = 3。peek 把手窗口不改动（已有 `.shadow(false)`，且其 `disable_corner_artifacts(&built)` 调用变量名不同，不会被误伤）。

- [ ] **Step 1: 三处 builder 补 `.shadow(false)`**

定位片段（3 处相同）：

```rust
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
```

替换为：

```rust
    .always_on_top(false)
    .skip_taskbar(false)
    .shadow(false)
    .center()
```

断言：替换命中 3 次。若命中数 ≠ 3，立即中止并报告。

- [ ] **Step 2: 三处补 Resized 重设 DWM**

定位片段（3 处相同）：

```rust
    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
```

替换为：

```rust
    #[cfg(windows)]
    crate::dwm_corner::disable_corner_artifacts(&window);
    #[cfg(windows)]
    {
        // Win11 在窗口尺寸变化后会重画 DWM 边框/圆角，放大后上边缘会重新
        // 出现 1px 描边；复用 dashboard 的模式，每次 Resized 后重设。
        let win_for_event = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Resized(_)) {
                crate::dwm_corner::disable_corner_artifacts(&win_for_event);
            }
        });
    }
```

断言：命中 3 次。注意 settings 窗口该行之后还有 `let _ = window.show();` 等语句，仅替换锚定片段本身，不动后续语句。

- [ ] **Step 3: 编译验证**

Run: `cargo check`（cwd: `src-tauri`）
Expected: 编译通过。

---

### Task 7: types.ts — Settings 接口字段替换

**Files:**
- Modify: `src/lib/types.ts`（246 行）

- [ ] **Step 1: 字段替换**

将：

```ts
  autostart_hint_shown: boolean;
```

替换为：

```ts
  /** Whether to launch automatically at system boot (Windows HKCU Run key). */
  autostart: boolean;
```

---

### Task 8: Settings.svelte — 开机自启开关

**Files:**
- Modify: `src/Settings.svelte`（状态 68 行后、refreshAll 158 行后、persistSettings 427 行后、模板 988–989 行间）

- [ ] **Step 1: 状态声明**

在 `let edgeSnap = $state(true);` 之后插入：

```ts
  let autostart = $state(false);
```

- [ ] **Step 2: refreshAll 回填**

在 `edgeSnap = s.edge_snap ?? true;` 之后插入：

```ts
      autostart = s.autostart ?? false;
```

- [ ] **Step 3: persistSettings 保存**

在 `edge_snap: edgeSnap,` 之后插入：

```ts
        autostart,
```

- [ ] **Step 4: 模板开关行**

在「行为」section 内、「靠边吸附」behavior-row 结束的 `</div>` 之后、该 section 的 `</div>` 之前插入：

```svelte
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">开机自启</span>
                <span class="behavior-hint">开机后自动运行本程序（写入当前用户注册表，点保存后生效）。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={autostart} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
```

- [ ] **Step 5: 前端类型检查**

Run: `npm run build`（cwd: 仓库根）
Expected: vite build + svelte-check 通过，无类型错误（`autostart_hint_shown` 已无任何引用点）。

---

### Task 9: 全量验证

- [ ] **Step 1: Rust 测试**

Run: `cargo test`（cwd: `src-tauri`）
Expected: 基线 127 passed + 新增 1 条 = **128 passed，0 failed**。

- [ ] **Step 2: 完整打包**

Run: `npm run tauri:build`（cwd: 仓库根）
Expected: 前端构建 + Rust release 编译 + NSIS/MSI 打包全部成功（产物位于 `D:\cargo-target\release\bundle\`）。

- [ ] **Step 3: 手动验证清单（需真机，报告时列出待办）**

1. 双开：再次启动 exe → 第二实例自动退出，已运行面板被唤起（若在托盘则还原并聚焦）。
2. 自启：设置页打开「开机自启」→ 保存 → 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 出现本程序键值；关闭 → 保存 → 键值消失。
3. 弹窗：打开趋势/工具/设置窗口并拉大尺寸 → 上边缘无黑/白线。

---

### Task 10: 提交（需用户确认后执行）

- [ ] **Step 1: 暂存本计划文件**

```powershell
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/settings.rs src-tauri/src/lib.rs src-tauri/src/ipc.rs src/lib/types.ts src/Settings.svelte docs/superpowers/specs/2026-09-27-autostart-single-instance-popup-line-design.md docs/superpowers/plans/2026-09-27-autostart-single-instance-popup-line.md
```

（`Cargo.lock` 因新增依赖会更新，一并提交。不得 add：`src/lib/components/ProviderLogo.svelte`、`release/`、`docs/mockups/`。）

- [ ] **Step 2: 提交**

提交信息（中文 + OMC trailers）：

```
feat: 开机自启设置项、单例模式，修复弹窗上边缘描边

- 新增 tauri-plugin-autostart：设置页「行为」区可开关自启（默认关），save_settings 变更对账注册表，启动时自愈补写
- 新增 tauri-plugin-single-instance（注册于 builder 链最前）：重复启动聚焦已运行面板
- settings: autostart 字段替换遗留 autostart_hint_shown，旧 config.toml 兼容（serde 忽略未知字段）
- ipc: trend/tools/settings 窗口补 shadow(false) 并在 Resized 后重设 DWM 属性，消除放大后上边缘 1px 描边

Constraint: 单例回调聚焦 dashboard；自启默认关闭
Rejected: 手写 winreg / CreateMutex（路径与引号维护成本高、无法唤起已有实例）
Confidence: 高（cargo test 全绿、tauri:build 通过）
Scope-risk: 低（peek 把手窗口未改动）
Not-tested: 真机重启自启、双开唤起（需手动验证）
```

---

## 自审记录（writing-plans Self-Review）

1. **Spec 覆盖**：spec 各节——自启插件与设置项（Task 2/4/5/7/8）、set_autostart 命令（Task 5）、save_settings 对账（Task 5）、启动自愈（Task 4）、单例（Task 3）、边线修复（Task 6）、旧配置兼容测试（Task 1）——全部有对应任务；capabilities 不改（插件仅 Rust 侧调用，无前端 invoke 插件命令）。
2. **占位符扫描**：所有步骤含完整代码/命令/预期输出，无 TBD/TODO。
3. **类型一致性**：`autostart: bool` 贯穿 settings.rs ↔ types.ts ↔ Settings.svelte ↔ save_settings 对账；`set_autostart` 命令名与 invoke_handler 注册一致；`MacosLauncher::LaunchAgent`、`ManagerExt` 全路径与 v2 插件 API 一致。
