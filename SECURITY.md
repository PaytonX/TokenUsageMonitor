# 安全策略 / Security Policy

## 报告漏洞 / Reporting a vulnerability

请使用 GitHub 的**私密漏洞报告**（仓库 Security 标签页 →
"Report a vulnerability"）提交安全问题，**不要**直接开公开 Issue。

请在报告中包含：影响的组件（如 router / hub / 本地扫描）、复现步骤、
影响评估。我们会在 7 天内初步回应。

Please use GitHub's private vulnerability reporting (Security tab →
"Report a vulnerability") instead of opening a public issue. Include the
affected component, reproduction steps and impact assessment. We aim to
respond within 7 days.

## 支持的版本 / Supported versions

| 版本 | 支持状态 |
|---|---|
| 最新 Release | ✅ 安全修复 |
| 更早版本 | ❌ 请升级 |

## 安全设计说明

- **凭证存储**：Provider API Key / AK·SK 存入 Windows 凭据管理器
  （`keyring` crate，DPAPI 保护），keyring service 名为
  `TokenUsageMonitor`。Codex 凭证例外：来自本机 `~/.codex/auth.json`，
  每次轮询现读，不复制不落盘。
- **密钥生成**：hub 共享密钥与路由 token 均由 `getrandom`
  （Windows 下即 BCryptGenRandom）生成，128-bit 熵。
- **路由代理的凭据隔离**：TokenRouter 只把客户端请求头按白名单透传
  （content-type / accept / user-agent / anthropic-version / beta），
  客户端的 `Authorization` / `x-api-key` **绝不转发给上游**；上游凭据
  由路由器从候选账户注入。
- **更新检查的 URL 白名单**：应用内"打开仓库 / Releases"走
  `open_url` 命令，仅放行 `https://github.com`（无显式端口），其余
  URL 一律拒绝。
- **日志**：tracing 输出到 stderr，约定不打印任何 token / key /
  cookie / 完整响应体（见 `docs/logging.md`）。

## 已知权衡（如实披露）

- **hub 共享密钥与路由 token 明文存于 config.toml**（与 Provider Key 的
  DPAPI 保护不同层级）。请不要分享该文件；文件的访问控制依赖操作系统的
  用户目录权限。
- **多端同步走局域网明文 HTTP**（无 TLS），鉴权为 Bearer 共享密钥。
  若显式清空 `hub_token`，hub 将放行所有 `/ingest` / `/devices` 请求
  （仅限能连到 `0.0.0.0:43210` 的对端）。跨不可信网络请勿开启同步。
- **路由代理的 `/health` 端点无鉴权**（仅本机 `127.0.0.1` 可达），暴露
  监听端口与启停状态，不含敏感数据。
- **Tauri CSP 当前为 `null`**（见 `src-tauri/tauri.conf.json`）。前端
  不加载任何远程资源，攻击面主要在本地，但这不是纵深防御的最优状态，
  后续版本计划收紧。
- **本机扫描范围**：会读取 7 种 AI 工具会话日志的 usage 字段（不读对话
  内容），Windows 上默认包含 WSL 发行版内的同名目录。

## 数据外发边界

应用出网请求的完整清单见 [README 隐私与数据说明](README.md#隐私与数据说明)。
除该清单所列外，应用不发出任何其他网络请求。
