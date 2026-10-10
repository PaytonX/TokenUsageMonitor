import { describe, expect, it } from "vitest";
import {
  canonicalModelKey,
  dominantDisplayName,
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
  it("不同模型不被合并（不做模糊匹配）", () => {
    expect(canonicalModelKey("GLM-5.3")).not.toBe(canonicalModelKey("GLM-5.3-Flash"));
    expect(canonicalModelKey("deepseek-v4-flash")).not.toBe(
      canonicalModelKey("deepseek-v4-flash-260425"),
    );
  });
  it("忽略首尾空白", () => {
    expect(canonicalModelKey("  GLM-5.3  ")).toBe("glm-5.3");
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
