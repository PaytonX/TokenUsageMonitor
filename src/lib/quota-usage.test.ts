import { describe, expect, it } from "vitest";
import type { UsageDailyRow } from "./api";
import type { UsageSnapshot } from "./types";
import {
  buildQuotaSeries,
  dayWindow,
  groupQuotaSeries,
  isPercentUnit,
  subscriptionFingerprint,
} from "./quota-usage";

function row(
  source: string,
  date: string,
  total: number,
  unit = "percent",
  kind: UsageDailyRow["kind"] = "provider",
): UsageDailyRow {
  return {
    source,
    kind,
    date,
    provider: "",
    model: "",
    input: 0,
    cache_read: 0,
    output: 0,
    total,
    unit,
    cost: null,
    currency: null,
    cost_estimated: false,
  };
}

function snapshot(id: string, resets: (string | undefined)[]): UsageSnapshot {
  const w = (reset_at?: string) => ({
    used: 10,
    quota: 100,
    unit: "percent" as const,
    reset_at,
    over_quota: false,
  });
  return {
    provider_id: id,
    provider_display_name: id,
    timestamp: "2026-10-24T00:00:00Z",
    windows: {
      five_hour: w(resets[0]),
      weekly: w(resets[1]),
      monthly: w(resets[2]),
    },
  };
}

describe("isPercentUnit", () => {
  it("只认 percent —— 额度轨道不靠 provider 名单，而靠单位", () => {
    expect(isPercentUnit("percent")).toBe(true);
    expect(isPercentUnit("tokens")).toBe(false);
    expect(isPercentUnit("")).toBe(false);
    expect(isPercentUnit(undefined)).toBe(false);
  });
});

describe("dayWindow", () => {
  it("生成连续的升序日期，末尾就是区间最后一天", () => {
    expect(dayWindow(3, "2026-10-24")).toEqual([
      "2026-10-22",
      "2026-10-23",
      "2026-10-24",
    ]);
  });

  it("跨月也连续（不跳过没数据的日子）", () => {
    expect(dayWindow(2, "2026-11-01")).toEqual(["2026-10-31", "2026-11-01"]);
  });
});

describe("subscriptionFingerprint", () => {
  it("同订阅的多个 Key 指纹相同（各窗口 reset_at 一致）", () => {
    const a = snapshot("opencode-a", [
      "2026-10-24T05:00:00Z",
      "2026-10-26T00:00:00Z",
      "2026-11-01T00:00:00Z",
    ]);
    const b = snapshot("opencode-b", [
      "2026-10-24T05:00:00Z",
      "2026-10-26T00:00:00Z",
      "2026-11-01T00:00:00Z",
    ]);
    expect(subscriptionFingerprint(a)).toBe(subscriptionFingerprint(b));
  });

  it("不同订阅指纹不同（重置时刻不同）", () => {
    const a = snapshot("opencode-a", ["2026-10-24T05:00:00Z"]);
    const b = snapshot("opencode-b", ["2026-10-24T09:00:00Z"]);
    expect(subscriptionFingerprint(a)).not.toBe(subscriptionFingerprint(b));
  });

  it("一个 reset_at 都没有时退回账户 id：宁可按独立订阅算", () => {
    const a = snapshot("opencode-a", [undefined, undefined, undefined]);
    const b = snapshot("opencode-b", [undefined, undefined, undefined]);
    expect(subscriptionFingerprint(a)).not.toBe(subscriptionFingerprint(b));
    expect(subscriptionFingerprint(a)).toContain("opencode-a");
  });
});

describe("buildQuotaSeries", () => {
  const kindOf = (s: string) => s.split("-")[0];

  it("只取 percent 行：token 行与其它单位都不进额度轨道", () => {
    const series = buildQuotaSeries(
      [
        row("opencode", "2026-10-24", 3.5),
        row("deepseek", "2026-10-24", 1200, "tokens"),
        row("kimi", "2026-10-24", 2.2, "cny"),
      ],
      { kindOf, days: 2, end: "2026-10-24" },
    );
    expect(series.map((s) => s.source)).toEqual(["opencode"]);
    expect(series[0].total).toBeCloseTo(3.5, 6);
  });

  it("区间外的日子被丢掉，缺数据的日子补 0（序列始终连续）", () => {
    const series = buildQuotaSeries(
      [
        row("opencode", "2026-10-20", 9.9), // 区间外
        row("opencode", "2026-10-24", 1.5),
      ],
      { kindOf, days: 3, end: "2026-10-24" },
    );
    expect(series[0].days).toEqual([
      { date: "2026-10-22", value: 0 },
      { date: "2026-10-23", value: 0 },
      { date: "2026-10-24", value: 1.5 },
    ]);
    expect(series[0].peak).toBeCloseTo(1.5, 6);
  });
});

describe("groupQuotaSeries", () => {
  const kindOf = (s: string) => s.split("-")[0];

  it("同一订阅的多个 Key 只算一份额度（否则同一份额度被报成两倍）", () => {
    const series = buildQuotaSeries(
      [
        row("opencode-a", "2026-10-24", 4),
        row("opencode-b", "2026-10-24", 4),
      ],
      {
        kindOf,
        subscriptionOf: () => "opencode@samesub",
        days: 1,
        end: "2026-10-24",
      },
    );
    const [group] = groupQuotaSeries(series);
    expect(group.keys).toBe(2);
    expect(group.subscriptions).toBe(1);
    expect(group.total).toBeCloseTo(4, 6); // 不是 8
    expect(group.days[0].value).toBeCloseTo(4, 6);
  });

  it("不同订阅并列时取均值（容量归一），不相加", () => {
    const series = buildQuotaSeries(
      [
        row("opencode-a", "2026-10-24", 4),
        row("opencode-b", "2026-10-24", 2),
      ],
      {
        kindOf,
        subscriptionOf: (s) => `sub:${s}`,
        days: 1,
        end: "2026-10-24",
      },
    );
    const [group] = groupQuotaSeries(series);
    expect(group.subscriptions).toBe(2);
    expect(group.total).toBeCloseTo(3, 6); // (4+2)/2，不是 6
    expect(group.monthTotal).toBeCloseTo(3, 6);
  });

  it("本自然月累计只算当月，跨月的部分不算进「本期已用」", () => {
    const series = buildQuotaSeries(
      [
        row("opencode", "2026-09-30", 5),
        row("opencode", "2026-10-01", 1),
        row("opencode", "2026-10-02", 2),
      ],
      { kindOf, days: 3, end: "2026-10-02" },
    );
    const [group] = groupQuotaSeries(series);
    expect(group.total).toBeCloseTo(8, 6);
    expect(group.monthTotal).toBeCloseTo(3, 6);
  });

  it("不同 kind 各自成组，按累计降序", () => {
    const series = buildQuotaSeries(
      [
        row("minimax", "2026-10-24", 9),
        row("opencode", "2026-10-24", 2),
      ],
      { kindOf, days: 1, end: "2026-10-24" },
    );
    const groups = groupQuotaSeries(series);
    expect(groups.map((g) => g.kind)).toEqual(["minimax", "opencode"]);
    expect(groups[1].label).toBe("OpenCode");
  });
});
