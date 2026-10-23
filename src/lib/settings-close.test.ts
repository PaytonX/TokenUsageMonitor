/**
 * settings-close 单测（方案 .scratch/glance-bench/issues/02b）。
 *
 * 钉住「关闭设置窗时到底该问还是该关」这一条判定：它决定用户每次点 ✕
 * 会不会被多问一句，也决定二次 ✕ 能不能真的关掉窗口。
 */
import { describe, expect, it } from "vitest";
import { resolveCloseAction } from "./settings-close";

describe("settings-close: resolveCloseAction", () => {
  it("无草稿 + 未确认 → 直接关（不该出现二次确认）", () => {
    expect(resolveCloseAction({ hasDraft: false, confirming: false })).toBe("close");
  });

  it("有草稿 + 未确认 → 弹确认条", () => {
    expect(resolveCloseAction({ hasDraft: true, confirming: false })).toBe("open-confirm");
  });

  it("有草稿 + 已确认 → 直接关（二次 ✕ 就是「坚持关闭」）", () => {
    expect(resolveCloseAction({ hasDraft: true, confirming: true })).toBe("close");
  });

  it("无草稿 + 已确认 → 直接关（条子已失效，不该吞掉这次关闭）", () => {
    expect(resolveCloseAction({ hasDraft: false, confirming: true })).toBe("close");
  });
});
