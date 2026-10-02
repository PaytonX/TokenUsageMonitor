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

  it("注册表覆盖全部可产出 p: key 的 kind（回归：漏一个就少一张卡片能联动）", () => {
    // providerForModel 能产出的 key 里，除合成兜底桶 "other"（无对应卡片）外，
    // 每个都必须能反查到 kind，否则该 Provider 的卡片点了没反应。
    // 曾经的真实漏项：spark 有品牌登记、有 PREFIX_RULES，却不在注册表里。
    const pKeys = [
      "anthropic", "deepseek", "doubao", "gemini", "glm", "kimi",
      "minimax", "openai", "qwen", "spark", "volcengine", "xai",
    ];
    for (const k of pKeys) {
      expect(ledgerSeriesForKind(k, `${k}-1-0`), `kind=${k}`).not.toBeNull();
    }
  });

  it("未知 kind 返回 null（诚实：不猜）", () => {
    // 单词未知 kind（不靠 normalizeKind 截断也能判空），与上面的多词兜底互不替代
    expect(ledgerSeriesForKind("zzz", "x-1-0")).toBeNull();
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
