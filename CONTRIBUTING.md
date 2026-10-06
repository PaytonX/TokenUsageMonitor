# 参与贡献 / Contributing

感谢关注 TokenUsageMonitor！欢迎 Issue 与 PR。

## 开发环境

构建前置条件与步骤见 [README 构建指南](README.md#构建指南)（英文版见
[README.en.md](README.en.md#build)）。要点：

- Windows 10/11，Node.js ≥ 20，Rust stable（GNU 工具链由
  `rust-toolchain.toml` 自动固定，无需 MSVC Build Tools）
- `npm install && npm run tauri:dev` 起开发环境

## 提交 PR 前必须通过

```bash
cd src-tauri && cargo test   # Rust 单元 + 集成测试（约 300 个）
npm test                     # vitest
npx svelte-check             # 前端类型检查，0 error 0 warning
```

- `cargo fmt` / `cargo clippy` 建议在本地先跑一遍
- 新功能请附单元测试；修复 bug 时先写能复现问题的测试再修

## 代码约定

- **注释与提交信息**：仓库现状以中文注释为主，新代码沿用即可；
  提交信息格式 `<type>(<scope>): 描述`（参考 `git log`）
- **日志**：tracing target 用 `tum.<子系统>` 命名，禁止打印 token /
  key / cookie / 完整响应体，约定详见 [docs/logging.md](docs/logging.md)
- **凭据**：任何源码、测试、示例中不得出现可用的凭据字面量；测试需要
  凭据时用运行时拼接的合成值
- **新增 Provider**：实现 `src-tauri/src/providers/mod.rs` 的
  `Provider` trait（`id()` / `display_name()` / `auth_kind()` /
  `fetch_usage()` → `UsageSnapshot`），再在预设目录注册
- **前端**：Svelte 5 runes 模式（`svelte.config.js` 强制），设置项
  统一收在 `src/Settings.svelte`

## 提交什么会被优先合并

- 新 Provider 支持（有对应官方用量 API 的）
- 新本机工具的会话日志解析（只读 usage 字段，不读对话内容）
- Bug 修复与测试补充
- 文档改进（中英双语同步更新）

## 安全问题

不要以公开 Issue 提交安全漏洞，走
[SECURITY.md](SECURITY.md) 里的私密漏洞报告渠道。
