import { describe, expect, it } from "vitest";
import { highlightKeyForKind, ledgerSeriesForKind } from "./calendar-linkage";

describe("ledgerSeriesForKind", () => {
  it("普通 kind 映射到跨工具 provider key（不是工具 id）", () => {
    // 回归：723e360 引入 model-provider.ts 后，下拉改用 provider key，
    // 但卡片点击仍写工具 id（minimax-code），导致 p:minimax-code 永不匹配。
    expect(ledgerSeriesForKind("minimax", "minimax-1756-0")).toEqual({
      mode: "cross-tool",
      providerKey: "minimax",
    });
    expect(ledgerSeriesForKind("deepseek", "deepseek-1756-1")).toEqual({
      mode: "cross-tool",
      providerKey: "deepseek",
    });
  });

  it("带后缀的子模式归一化后再判定（火山 API 按量）", () => {
    // instance_id 形如 volcengine_api-<epoch>-<n>，split("-")[0] 得到
    // "volcengine_api"，直接查集合会漏判、掉进兜底分支显示"全部工具"。
    expect(ledgerSeriesForKind("volcengine_api", "volcengine_api-1-0")).toEqual({
      mode: "account-daily",
      providerId: "volcengine_api-1-0",
    });
    expect(ledgerSeriesForKind("kimi_global", "kimi_global-1-0")).toEqual({
      mode: "cross-tool",
      providerKey: "kimi",
    });
  });

  it("有服务端日账的 kind 走 account-daily", () => {
    expect(ledgerSeriesForKind("volcengine", "volcengine-9-0")).toEqual({
      mode: "account-daily",
      providerId: "volcengine-9-0",
    });
  });

  it("未登记的 kind 返回 null（诚实：不猜）", () => {
    expect(ledgerSeriesForKind("some_unknown_thing", "x-1-0")).toBeNull();
  });
});

describe("highlightKeyForKind", () => {
  it("跨工具 kind 返回高亮 key", () => {
    expect(highlightKeyForKind("minimax")).toBe("minimax");
    expect(highlightKeyForKind("minimax_api")).toBe("minimax");
  });

  it("账户日账 kind 不可高亮（单位不同，混算即错）", () => {
    // 火山是 AFP、OpenAI 是 USD，与本机工具 token 不同量纲。
    expect(highlightKeyForKind("volcengine")).toBeNull();
    expect(highlightKeyForKind("openai")).toBeNull();
    expect(highlightKeyForKind("xai")).toBeNull();
  });
});
