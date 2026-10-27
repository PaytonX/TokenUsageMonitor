import { describe, expect, it } from "vitest";
import { formatResetIn } from "./reset-time";

const NOW = Date.UTC(2026, 9, 27, 12, 0, 0);

function at(offsetMs: number): string {
  return new Date(NOW + offsetMs).toISOString();
}

describe("formatResetIn", () => {
  it("超过一天用 d + 两位小时", () => {
    expect(formatResetIn(at(4 * 86_400_000 + 6 * 3_600_000), NOW)).toBe("4d 06h");
  });

  it("一天内用 h + 两位分钟", () => {
    expect(formatResetIn(at(2 * 3_600_000 + 10 * 60_000), NOW)).toBe("2h 10m");
  });

  it("一小时内用分钟", () => {
    expect(formatResetIn(at(45 * 60_000), NOW)).toBe("45m");
  });

  it("不足一分钟用秒", () => {
    expect(formatResetIn(at(12_000), NOW)).toBe("12s");
  });

  it("已过期返回 0m（下一次轮询会带回新窗口）", () => {
    expect(formatResetIn(at(-60_000), NOW)).toBe("0m");
  });

  it("缺失 / 非法时间返回破折号，不抛错", () => {
    expect(formatResetIn(undefined, NOW)).toBe("—");
    expect(formatResetIn(null, NOW)).toBe("—");
    expect(formatResetIn("not a date", NOW)).toBe("—");
  });
});
