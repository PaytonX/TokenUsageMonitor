// 账本 `provider` 列（上游计费账户）→ provider key 的归因测试。
//
// 背景：趋势页曾把「经 OpenCode Go 调用的 deepseek-v4.1-flash」全部算进
// DeepSeek（实测 879 条消息 / 77.7M tokens）。根因不是模型名规则写错，而是
// 归因只看了模型名、账本里记着的账户被彻底忽略。这些用例把「账户优先」钉住。
import { describe, expect, it } from "vitest";
import {
  providerForLedgerRow,
  providerKeyForAccount,
  providerLabel,
} from "./model-provider";

describe("providerKeyForAccount", () => {
  it("认得各客户端上报的账户别名", () => {
    expect(providerKeyForAccount("opencode-go")).toBe("opencode");
    expect(providerKeyForAccount("  OpenCode-Zen ")).toBe("opencode");
    expect(providerKeyForAccount("ark-cn")).toBe("volcengine");
    expect(providerKeyForAccount("volcanoagentplan")).toBe("volcengine");
    expect(providerKeyForAccount("minimax-cn")).toBe("minimax");
    expect(providerKeyForAccount("moonshot")).toBe("kimi");
  });

  it("本机路由账户不是厂商，返回 null 交给路由台账归因", () => {
    // 若把 local-router 当厂商，经路由的 token 会凭空多出一个 Provider 层，
    // 路由台账的真实模型替换也会被这层短路。
    expect(providerKeyForAccount("local-router")).toBeNull();
    expect(providerKeyForAccount("TokenRouter")).toBeNull();
    // 变体写法：实例名里拼了承载厂商，前缀兜底会把它认成 minimax 账户。
    expect(providerKeyForAccount("local-router-minimax")).toBeNull();
  });

  it("没记账户 / 认不出的账户都返回 null，不硬凑 other", () => {
    // "other" 是归因结论；由账户名认不出就下结论，会把未知量吞进兜底桶，
    // 让模型名兜底（比如单 Provider 工具）彻底失效。
    expect(providerKeyForAccount("")).toBeNull();
    expect(providerKeyForAccount(undefined)).toBeNull();
    expect(providerKeyForAccount("someone-new")).toBeNull();
  });
});

describe("providerForLedgerRow", () => {
  it("账户优先：经 OpenCode Go 的 DeepSeek 模型算 OpenCode", () => {
    expect(
      providerForLedgerRow({
        model: "deepseek-v4.1-flash",
        source: "deepseek-harness",
        provider: "opencode-go",
      }),
    ).toBe("opencode");
  });

  it("没记账户时退回模型名（老日志/旧版本写入口行为不变）", () => {
    expect(
      providerForLedgerRow({
        model: "deepseek-v4.1-flash",
        source: "deepseek-harness",
        provider: "",
      }),
    ).toBe("deepseek");
    expect(providerForLedgerRow({ model: "MiniMax-M3" })).toBe("minimax");
  });

  it("账户记的是本机路由时仍按模型名兜底", () => {
    expect(
      providerForLedgerRow({
        model: "__via_router__",
        source: "codex",
        provider: "local-router",
      }),
    ).toBe("other");
  });
});

describe("providerLabel", () => {
  it("OpenCode 有稳定展示名", () => {
    expect(providerLabel("opencode")).toBe("OpenCode");
  });
});
