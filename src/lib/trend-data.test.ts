// 趋势数据源（`fetchProviderCrossToolSeries`）的归因测试。
//
// 这层是趋势页「Provider 用量」的唯一入口：它读账本、按 provider 分层，任何
// 一处归因写错都会直接显示在用户面前。三条不变式：
//   1) 来源总量行（model='')不参与分模型累加，否则整条趋势线翻倍；
//   2) 账本记了计费账户就按账户归因（经 OpenCode Go 的 DeepSeek 不算 DeepSeek）；
//   3) 路由哨兵行在没有路由台账时回落到兜底桶，而不是硬塞给某个厂商。
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UsageDailyRow } from "./api";

const mocks = vi.hoisted(() => ({
  history: vi.fn(),
  kinds: vi.fn(),
  usage: vi.fn(),
}));

vi.mock("./api", () => ({
  getUsageHistory: mocks.history,
  loadInstanceKinds: mocks.kinds,
  getUsage: mocks.usage,
}));

import { fetchProviderCrossToolSeries, fetchQuotaGroups, fetchToolSeries } from "./trend-data";

function row(
  over: Partial<UsageDailyRow> & { date: string; total: number },
): UsageDailyRow {
  return {
    source: "deepseek-harness",
    kind: "tool",
    model: "",
    provider: "",
    input: over.total,
    cache_read: 0,
    output: 0,
    unit: "tokens",
    cost: null,
    currency: null,
    cost_estimated: false,
    ...over,
  };
}

beforeEach(() => {
  mocks.history.mockReset();
  mocks.kinds.mockReset().mockResolvedValue(new Map());
  mocks.usage.mockReset().mockResolvedValue([]);
});

describe("fetchProviderCrossToolSeries", () => {
  it("账户优先：经 OpenCode Go 的 DeepSeek 模型记在 OpenCode 名下", async () => {
    mocks.history.mockResolvedValue({
      rows: [
        row({
          date: "2026-10-05",
          model: "deepseek-v4.1-flash",
          provider: "opencode-go",
          total: 700,
        }),
        row({ date: "2026-10-05", model: "deepseek-v4.1-flash", total: 300 }),
      ],
    });
    const { ids, series } = await fetchProviderCrossToolSeries(30);
    expect(series.opencode["2026-10-05"]).toBe(700);
    expect(series.deepseek["2026-10-05"]).toBe(300);
    expect(ids).toEqual(["opencode", "deepseek"]);
  });

  it("来源总量行不参与分模型累加（否则趋势值翻倍）", async () => {
    mocks.history.mockResolvedValue({
      rows: [
        row({ date: "2026-10-05", total: 1000 }), // 来源总量行
        row({ date: "2026-10-05", model: "MiniMax-M3", total: 600 }),
        row({ date: "2026-10-05", model: "deepseek-v4", total: 400 }),
      ],
    });
    const { series } = await fetchProviderCrossToolSeries(30);
    const sum = Object.values(series).reduce(
      (s, days) => s + Object.values(days).reduce((a, b) => a + b, 0),
      0,
    );
    expect(sum).toBe(1000);
  });

  it("路由哨兵行没有路由台账时按工具默认兜底，不硬塞给无关厂商", async () => {
    mocks.history.mockResolvedValue({
      rows: [
        row({
          date: "2026-10-05",
          model: "__via_router__",
          provider: "local-router",
          total: 250,
        }),
        row({
          source: "codex",
          date: "2026-10-05",
          model: "__via_router__",
          provider: "local-router",
          total: 50,
        }),
      ],
    });
    const { series } = await fetchProviderCrossToolSeries(30);
    // deepseek-harness 是单 Provider 工具，兜底规则把它记到 DeepSeek；codex
    // 没有单 Provider 默认，落 other。两者都不是"猜一个厂商"。
    expect(series.deepseek["2026-10-05"]).toBe(250);
    expect(series.other["2026-10-05"]).toBe(50);
  });

  it("区间内零用量的 Provider 不产生空序列", async () => {
    mocks.history.mockResolvedValue({
      rows: [row({ date: "2026-10-05", model: "MiniMax-M3", total: 10 })],
    });
    const { ids } = await fetchProviderCrossToolSeries(30);
    expect(ids).toEqual(["minimax"]);
  });
});

describe("fetchToolSeries", () => {
  it("只取工具总量行，按 source 分键", async () => {
    mocks.history.mockResolvedValue({
      rows: [
        row({ source: "hermes", date: "2026-10-05", total: 500 }),
        row({ source: "hermes", date: "2026-10-05", model: "deepseek-flash", total: 500 }),
        row({ source: "codex", date: "2026-10-05", total: 120 }),
      ],
    });
    const { ids, series } = await fetchToolSeries(30);
    expect(ids.sort()).toEqual(["codex", "hermes"]);
    expect(series.hermes["2026-10-05"]).toBe(500);
    expect(series.codex["2026-10-05"]).toBe(120);
  });
});

// 额度轨道的数据源：百分比行 → 按订阅去重的 kind 级序列。
// 它是「多个 API Key 如何统一成一个账户用量」这条规则的唯一实现，模式里
// 最容易错的两点：同订阅重复计入（三倍用量）与跨单位混算（percent 混进 token）。
describe("fetchQuotaGroups", () => {
  function percentRow(
    source: string,
    date: string,
    total: number,
  ): UsageDailyRow {
    return row({ source, date, total, kind: "provider", unit: "percent" });
  }

  function snapshot(id: string, reset: string) {
    const w = {
      used: 10,
      quota: 100,
      unit: "percent" as const,
      reset_at: reset,
      over_quota: false,
    };
    return {
      provider_id: id,
      provider_display_name: id,
      timestamp: "2026-10-24T00:00:00Z",
      windows: { five_hour: w, weekly: w, monthly: w },
    };
  }

  it("同一订阅的两个 Key 只算一份，不是两份相加", async () => {
    const day = new Date();
    const today = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
    // 两个 Key 看到同一批 reset_at = 同一份额度，服务端对两者报同一个 5%。
    mocks.usage.mockResolvedValue([
      snapshot("opencode-fresh", "2026-11-01T00:00:00Z"),
      snapshot("opencode-work", "2026-11-01T00:00:00Z"),
    ]);
    mocks.history.mockResolvedValue({
      rows: [
        percentRow("opencode-fresh", today, 5),
        percentRow("opencode-work", today, 5),
      ],
    });
    const [group] = await fetchQuotaGroups(30);
    expect(group.kind).toBe("opencode");
    expect(group.keys).toBe(2);
    expect(group.subscriptions).toBe(1);
    expect(group.monthTotal).toBe(5); // 去重后 5，不是 10
  });

  it("两份独立订阅按容量取均值，且忽略 token 行", async () => {
    const day = new Date();
    const today = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
    mocks.usage.mockResolvedValue([
      snapshot("opencode-a", "2026-11-01T00:00:00Z"),
      snapshot("opencode-b", "2026-11-08T00:00:00Z"),
    ]);
    mocks.history.mockResolvedValue({
      rows: [
        percentRow("opencode-a", today, 5),
        percentRow("opencode-b", today, 15),
        // token 行必须被忽略：额度是另一个量纲，混进来会污染百分比。
        row({ source: "opencode-a", date: today, total: 1_000_000 }),
      ],
    });
    const [group] = await fetchQuotaGroups(30);
    expect(group.subscriptions).toBe(2);
    expect(group.monthTotal).toBe(10); // (5 + 15) / 2
  });
});