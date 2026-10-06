---
name: Bug 报告
about: 报告应用行为异常
title: "[Bug] "
labels: bug
assignees: ""
---

**描述**
简要说明遇到了什么问题。

**复现步骤**
1. 打开 …
2. 点击 …
3. 出现 …

**期望行为**
正常应该发生什么。

**实际行为**
实际发生了什么（可附截图，注意打码凭据）。

**环境**
- Windows 版本（如 Win11 23H2）：
- 应用版本（设置 → 关于与诊断）：
- 是否启用 WSL / 多端同步 / TokenRouter：

**日志**
如方便，用 `RUST_LOG=debug` 启动后复现一次，附 stderr 输出
（确认不含任何 token/key 后再粘贴）。
