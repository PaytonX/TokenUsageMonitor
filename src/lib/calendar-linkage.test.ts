import { describe, expect, it } from "vitest";
import {
  buildLedgerBreakdown,
  focusStats,
  highlightKeyForKind,
  ledgerSeriesForKind,
} from "./calendar-linkage";

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

/** 构造一行账本。model 为 "" 即来源总量行。 */
const row = (
  source: string,
  date: string,
  model: string,
  total: number,
) => ({
  source,
  kind: "tool" as const,
  date,
  model,
  input: total,
  cache_read: 0,
  output: 0,
  total,
  unit: "tokens",
  cost: null,
  currency: null,
  cost_estimated: false,
});

describe("buildLedgerBreakdown", () => {
  it("总量只取来源总量行，不与分模型行相加", () => {
    // persist_all_tools 每个工具每天写两类行。旧实现两类都累加 → 翻倍。
    const rows = [
      row("minimax-code", "2026-10-01", "", 1000),      // 来源总量行
      row("minimax-code", "2026-10-01", "MiniMax-M3", 600), // 分模型行
      row("minimax-code", "2026-10-01", "deepseek-v4", 400), // 分模型行
    ];
    const map = buildLedgerBreakdown(rows);
    const d = map.get("2026-10-01")!;
    expect(d.total).toBe(1000);            // 不是 2000
    expect(d.byProvider).toEqual({ minimax: 600, deepseek: 400 });
  });

  it("分模型合计与来源总量对账一致（回归护栏）", () => {
    const rows = [
      row("zcode", "2026-10-02", "", 900),
      row("zcode", "2026-10-02", "GLM-5.3", 500),
      row("zcode", "2026-10-02", "未标记模型", 400),
    ];
    const d = buildLedgerBreakdown(rows).get("2026-10-02")!;
    const sum = Object.values(d.byProvider).reduce((s, v) => s + v, 0);
    expect(sum).toBe(d.total);
  });

  it("无模型信息的行按工具兜底归因（MiniMax Code → MiniMax）", () => {
    const rows = [row("minimax-code", "2026-10-03", "未标记模型", 700)];
    const d = buildLedgerBreakdown(rows).get("2026-10-03")!;
    expect(d.byProvider).toEqual({ minimax: 700 });
  });

  it("provider 账本行不计入本机工具口径", () => {
    const rows = [
      { ...row("volcengine-1-0", "2026-10-01", "", 50), kind: "provider" as const },
    ];
    expect(buildLedgerBreakdown(rows).size).toBe(0);
  });

  it("按日期升序无关，map 按需累加", () => {
    const rows = [
      row("codex", "2026-10-05", "", 300),
      row("codex", "2026-10-05", "gpt-5", 300),
      row("claude-code", "2026-10-05", "", 200),
    ];
    const d = buildLedgerBreakdown(rows).get("2026-10-05")!;
    expect(d.total).toBe(500);
    expect(d.byProvider).toEqual({ openai: 300 });
  });
});

describe("focusStats", () => {
  const win = ["2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"];
  const byProvider = { minimax: 100, deepseek: 200 };
  const days: Record<string, { total: number; byProvider: Record<string, number> }> = {
    "2026-10-01": { total: 400, byProvider },
    "2026-10-02": { total: 200, byProvider: { deepseek: 200 } },
    "2026-10-03": { total: 300, byProvider: { minimax: 100 } },
    "2026-10-04": { total: 100, byProvider: {} },
  };
  const toMap = (rec: typeof days) => new Map(Object.entries(rec));

  it("统计该 provider 的活跃天数/合计/峰值/占比", () => {
    const s = focusStats(toMap(days), win, "minimax", 1000);
    expect(s).toEqual({ days: 2, sum: 200, peak: 100, share: 20 });
  });

  it("窗口内无该 provider 用量时全部为 0，占比 0", () => {
    const s = focusStats(toMap(days), win, "kimi", 1000);
    expect(s).toEqual({ days: 0, sum: 0, peak: 0, share: 0 });
  });

  it("总量为 0 时占比不产生 NaN", () => {
    const s = focusStats(toMap(days), win, "minimax", 0);
    expect(s.share).toBe(0);
  });
});
