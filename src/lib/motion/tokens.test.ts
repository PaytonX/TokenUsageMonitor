// src/lib/motion/tokens.test.ts
// durMs() 单位防呆测试：CSS 时长值只认 "ms"/"s" 后缀，无单位/垃圾/≤0 必须
// 落 fallback，不能被 parseFloat 静默放行（历史隐患：parseFloat("0.2s")
// 得 0.2，数值上恰好能骗过 `> 0` 守卫）。
// vitest 走 vite.config.ts、无独立 test 配置，默认 node 环境本就没有
// window/document/getComputedStyle；这里统一用 vi.stubGlobal 显式搭出
// 浏览器三件套，不依赖「恰好跑在哪个环境」。
import { afterEach, describe, expect, it, vi } from "vitest";
import { durMs } from "./tokens";

/** 模拟浏览器环境：durMs 依次触碰 window、document.documentElement、getComputedStyle。 */
function stubBrowser(raw: string): void {
  vi.stubGlobal("window", {});
  vi.stubGlobal("document", { documentElement: {} });
  vi.stubGlobal("getComputedStyle", () => ({ getPropertyValue: () => raw }));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("durMs", () => {
  it("ms 值按毫秒原值解析", () => {
    stubBrowser("120ms");
    expect(durMs("--tum-dur-x", 999)).toBe(120);
  });

  it("s 值 ×1000 换算成毫秒", () => {
    // 回归锚点：修复前 parseFloat("0.25s") 得 0.25 且能通过守卫，
    // 动画会静默快 1000 倍；修复后必须得 250。
    stubBrowser("0.25s");
    expect(durMs("--tum-dur-x", 999)).toBe(250);
  });

  it("垃圾输入落 fallback", () => {
    stubBrowser("not-a-duration");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
  });

  it("无单位/空串也落 fallback（不认裸数值）", () => {
    // 修复前 parseFloat("120") 会放行裸数值；修复后无单位一律不认，
    // 防止 tokens.css 漏写单位时被静默当毫秒用。
    stubBrowser("120");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
    stubBrowser("");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
  });

  it("≤0 的值落 fallback", () => {
    stubBrowser("0ms");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
    stubBrowser("-5ms");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
    stubBrowser("0s");
    expect(durMs("--tum-dur-x", 999)).toBe(999);
  });

  it("无 window（SSR/测试环境）落 fallback", () => {
    // vi.stubGlobal("window", undefined) 之后 typeof window === "undefined"，
    // 显式钉住早退分支。
    vi.stubGlobal("window", undefined);
    expect(durMs("--tum-dur-x", 999)).toBe(999);
  });
});
