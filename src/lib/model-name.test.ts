import { describe, expect, it } from "vitest";
import {
  canonicalModelKey,
  dominantDisplayName,
  isPlaceholderModelName,
  stripModelPrefix,
} from "./model-name";

describe("stripModelPrefix", () => {
  it("剥掉 provider/ 前缀", () => {
    expect(stripModelPrefix("minimax/MiniMax-M3")).toBe("MiniMax-M3");
    expect(stripModelPrefix("openai/gpt-5")).toBe("gpt-5");
  });
  it("无前缀原样返回", () => {
    expect(stripModelPrefix("GLM-5.3-Flash")).toBe("GLM-5.3-Flash");
    expect(stripModelPrefix("")).toBe("");
  });
});

describe("canonicalModelKey", () => {
  // 回归：本机 zcode 写 GLM-5.3-Flash、路由台账写 glm-5.3-flash，
  // 不归并大小写就会被拆成两个模型条目。
  it("同一模型的大小写变体归到同一个键", () => {
    expect(canonicalModelKey("GLM-5.3-Flash")).toBe(
      canonicalModelKey("glm-5.3-flash"),
    );
    expect(canonicalModelKey("MiniMax-M3")).toBe(canonicalModelKey("minimax-m3"));
    expect(canonicalModelKey("Auto")).toBe(canonicalModelKey("auto"));
  });
  // 回归：真机账本里 minimax/MiniMax-M3.1-Flash（35.5 亿）与
  // MiniMax-M3.1-Flash（0.9 亿）被拆成两条，同一模型排名被对半分。
  it("同一模型的 provider/ 前缀变体归到同一个键", () => {
    expect(canonicalModelKey("minimax/MiniMax-M3.1-Flash")).toBe(
      canonicalModelKey("MiniMax-M3.1-Flash"),
    );
    expect(canonicalModelKey("minimax/MiniMax-M3")).toBe(
      canonicalModelKey("MiniMax-M3"),
    );
    // 前缀 + 大小写同时不同也要归一。
    expect(canonicalModelKey(" minimax/GLM-5.3-Flash ")).toBe(
      canonicalModelKey("glm-5.3-flash"),
    );
  });
  it("不同模型不被合并（不做模糊匹配）", () => {
    expect(canonicalModelKey("GLM-5.3")).not.toBe(canonicalModelKey("GLM-5.3-Flash"));
    expect(canonicalModelKey("deepseek-v4-flash")).not.toBe(
      canonicalModelKey("deepseek-v4-flash-260425"),
    );
    // 版本号是模型身份的一部分：v4 与 v4.1、M3 与 M3.1 都是不同模型。
    expect(canonicalModelKey("DeepSeek V4 Flash")).not.toBe(
      canonicalModelKey("DeepSeek V4.1 Flash"),
    );
    expect(canonicalModelKey("MiniMax-M3")).not.toBe(
      canonicalModelKey("MiniMax-M3.1-Flash"),
    );
    // -Preview 是另一个模型，不能因为主干名相同就并进来。
    expect(canonicalModelKey("minimax/MiniMax-M3.1-Flash-Preview")).not.toBe(
      canonicalModelKey("MiniMax-M3.1-Flash"),
    );
  });
  it("忽略首尾空白", () => {
    expect(canonicalModelKey("  GLM-5.3  ")).toBe("glm-5.3");
  });
});

describe("isPlaceholderModelName", () => {
  it("识别路由占位名", () => {
    expect(isPlaceholderModelName("__via_router__")).toBe(true);
    expect(isPlaceholderModelName("auto")).toBe(true);
    expect(isPlaceholderModelName("Auto")).toBe(true);
  });
  it("真模型（含名字里带 auto 的）不误判", () => {
    expect(isPlaceholderModelName("MiniMax-M3.1-Flash")).toBe(false);
    expect(isPlaceholderModelName("auto-router-v2")).toBe(false);
    expect(isPlaceholderModelName("未标记模型")).toBe(false);
    expect(isPlaceholderModelName("")).toBe(false);
  });
});

describe("dominantDisplayName", () => {
  it("取用量最大的写法", () => {
    const v = new Map([
      ["GLM-5.3-Flash", 1_643_542_486],
      ["glm-5.3-flash", 55_128_329],
    ]);
    expect(dominantDisplayName(v)).toBe("GLM-5.3-Flash");
  });
  it("展示名剥掉 provider/ 前缀（与归并键解耦，但前缀不进 UI）", () => {
    const v = new Map([
      ["minimax/MiniMax-M3.1-Flash", 3_549_717_420],
      ["MiniMax-M3.1-Flash", 92_351_368],
    ]);
    expect(dominantDisplayName(v)).toBe("MiniMax-M3.1-Flash");
  });
  it("并列时取字典序最小，保证结果稳定", () => {
    expect(dominantDisplayName(new Map([["b-model", 10], ["a-model", 10]]))).toBe(
      "a-model",
    );
    expect(dominantDisplayName(new Map([["a-model", 10], ["b-model", 10]]))).toBe(
      "a-model",
    );
  });
  it("只有一个写法时原样返回", () => {
    expect(dominantDisplayName(new Map([["gemini-3.5-flash", 1]]))).toBe(
      "gemini-3.5-flash",
    );
  });
  it("空表返回空串而不是崩", () => {
    expect(dominantDisplayName(new Map())).toBe("");
  });
});
