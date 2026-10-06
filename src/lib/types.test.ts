import { describe, expect, it } from "vitest";
import { hexToRgb } from "./types";

// hexToRgb 的 rgb()/rgba() 形态由 design-token 迁移引入（兜底灰 rgb(138,143,152)），
// 下游（TrendWindow segStyle、App badgeStyle、ProgressRing）依赖「同一颜色两种
// 写法解析结果一致」。这里锁定实际行为，防止回归。
describe("hexToRgb", () => {
  it('#RRGGBB → "r,g,b"（逗号分隔、无空格）', () => {
    expect(hexToRgb("#12b76a")).toBe("18,183,106");
  });

  it("十六进制大小写不敏感；首尾空白先 trim", () => {
    expect(hexToRgb("#12B76A")).toBe("18,183,106");
    expect(hexToRgb("  #12b76a  ")).toBe("18,183,106");
  });

  it("rgb(r,g,b)：紧凑与带空格写法同值", () => {
    expect(hexToRgb("rgb(138,143,152)")).toBe("138,143,152");
    expect(hexToRgb("rgb(138, 143, 152)")).toBe("138,143,152");
  });

  it("rgba(...) 只取三元组，丢弃 alpha", () => {
    expect(hexToRgb("rgba(18, 183, 106, 0.5)")).toBe("18,183,106");
    // dimOthers 压暗输出的形态（trend-data.ts:176）。
    expect(hexToRgb("rgba(138,143,152,0.22)")).toBe("138,143,152");
  });

  it("非法输入返回 null（含 3 位简写、缺 #、非颜色字符串）", () => {
    for (const bad of ["", "#fff", "#12345", "5b8cff", "red", "rgb()", "#gggggg"]) {
      expect(hexToRgb(bad), `input=${bad}`).toBeNull();
    }
  });

  it("通道范围不做校验：rgb(300,1,1) 原样返回 300（现状行为）", () => {
    // 实现：\d{1,3} 未设 0-255 上限。现有调用方都把三元组插进 rgba(...,a)，
    // 浏览器 CSS 会对超界通道钳制，故锁定现状、不在此断言期望的钳制行为。
    expect(hexToRgb("rgb(300,1,1)")).toBe("300,1,1");
  });
});
