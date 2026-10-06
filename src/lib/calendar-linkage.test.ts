import { describe, expect, it } from "vitest";
import {
  buildLedgerBreakdown,
  buildUnifiedBreakdown,
  DIM_FACTOR,
  focusStats,
  hexToRgbTriplet,
  highlightKeyForKind,
  ledgerSeriesForKind,
  LEVEL_ALPHA,
  paintCell,
  providerKeyForKind,
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

  it("账户日账 kind 也可高亮——合并日历后它们都在日历里", () => {
    // 回归：日历可切换口径的年代，volcengine/openai/xai 因"单位不同"被排除在
    // 高亮之外。日历恒为合并口径后该前提消失——
    //   volcengine：服务端日明细本身就是 tokens（providers/volcengine.rs:471
    //               直接沿用 API 返回的 c.unit），已并入日历；
    //   openai/xai：USD 日账不进 token 日历，但本机工具归因有它们的层
    //               （gpt/o1/o3/o4 → openai，grok → xai）。
    // 若仍返回 null，火山卡片点击后切到趋势页会看到"整张日历全被压暗"。
    expect(highlightKeyForKind("volcengine")).toBe("volcengine");
    expect(highlightKeyForKind("openai")).toBe("openai");
    expect(highlightKeyForKind("xai")).toBe("xai");
  });

  it("未注册 kind 不可高亮（诚实：不猜）", () => {
    expect(highlightKeyForKind("zzz")).toBeNull();
    expect(highlightKeyForKind("some_unknown_thing")).toBeNull();
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

describe("providerKeyForKind", () => {
  it("忽略 instance_id 后缀与子模式后缀", () => {
    // 服务端行的 source 是 instance_id（providers/volcengine.rs:463 写
    // self.instance_id），调用方先 split("-")[0] 再查表。
    expect(providerKeyForKind("volcengine")).toBe("volcengine");
    expect(providerKeyForKind("volcengine_api")).toBe("volcengine");
    expect(providerKeyForKind("kimi_global")).toBe("kimi");
  });

  it("未注册 kind 返回 null", () => {
    expect(providerKeyForKind("zzz")).toBeNull();
  });
});

/** 构造一行服务端日账（kind='provider'），unit 决定它能否进 token 日历。 */
const providerRow = (
  source: string,
  date: string,
  total: number,
  unit = "tokens",
) => ({
  ...row(source, date, "", total),
  kind: "provider" as const,
  unit,
});

describe("buildUnifiedBreakdown", () => {
  it("服务端有当天数据时**替换**本地值，而不是相加（防双计）", () => {
    // 本机 zcode 归因到火山 300，服务端说当天火山用了 500。真相是 500，
    // 不是 800。若相加，日历总量会随"本机是否也扫到该调用"而漂移。
    const rows = [
      row("zcode", "2026-10-01", "", 300),
      row("zcode", "2026-10-01", "ark-seed-1.6", 300), // → volcengine 层
      providerRow("volcengine-9-0", "2026-10-01", 500),
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-01")!;
    expect(d.byProvider.volcengine).toBe(500);
    expect(d.total).toBe(500); // 300 本地总量 − 300 本地归因 + 500 官方
  });

  it("豆包(doubao)被并入火山账户的服务端口径，不再单列", () => {
    // 用户 2026-10-02 拍板：本机没有配过其它能提供 doubao 模型的 provider，
    // 所以 doubao 层就是火山账户的一部分，官方日账自然覆盖它。
    // 关键在"整账户替换"：ark- 与 doubao- 的本地值一起被官方值取代，
    // 而不是各自替换、导致官方值之外还留着 doubao 那份 → 总量虚高。
    const rows = [
      row("zcode", "2026-10-08", "", 1000),
      row("zcode", "2026-10-08", "ark-seed-1.6", 400),   // → volcengine
      row("zcode", "2026-10-08", "doubao-seed-1.6", 600), // → doubao，同账户
      providerRow("volcengine-9-0", "2026-10-08", 900),
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-08")!;
    expect(d.byProvider).toEqual({ volcengine: 900 }); // doubao 层被吸收掉
    expect(d.total).toBe(900); // 1000 − (400 + 600) + 900
  });

  it("服务端无数据时 doubao 仍按本机归因单列（不硬吞）", () => {
    // 吸收只在"官方有账"时发生。没有官方数据的那些天，doubao 自己的量不该丢。
    const rows = [
      row("zcode", "2026-10-09", "", 900),
      row("zcode", "2026-10-09", "ark-seed-1.6", 300),   // → volcengine
      row("zcode", "2026-10-09", "doubao-seed-1.6", 600), // → doubao
      providerRow("volcengine-9-0", "2026-10-08", 500),    // 只覆盖前一天
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-09")!;
    expect(d.byProvider).toEqual({ volcengine: 300, doubao: 600 });
    expect(d.total).toBe(900);
  });

  it("别名吸收只对有官方数据的账户生效，不牵连其它 provider", () => {
    // glm 没有服务端日账，它不能因为"火山吸收 doubao"而被顺带改掉。
    const rows = [
      row("zcode", "2026-10-11", "", 1000),
      row("zcode", "2026-10-11", "ark-seed-1.6", 400),
      row("zcode", "2026-10-11", "doubao-seed-1.6", 300),
      row("zcode", "2026-10-11", "GLM-5.3", 300),
      providerRow("volcengine-9-0", "2026-10-11", 500),
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-11")!;
    expect(d.byProvider).toEqual({ volcengine: 500, glm: 300 });
    expect(d.total).toBe(800);
  });

  it("服务端有数据但本地无该 provider 时直接计入", () => {
    const rows = [
      row("minimax-code", "2026-10-02", "", 1000),
      row("minimax-code", "2026-10-02", "MiniMax-M3", 1000),
      providerRow("volcengine-9-0", "2026-10-02", 700),
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-02")!;
    expect(d.byProvider.volcengine).toBe(700);
    expect(d.total).toBe(1700); // 1000 本机 + 700 官方（本地该层为 0）
  });

  it("服务端当天缺数据时保留本机归因（逐 provider 独立判定）", () => {
    // 合并是 per-(provider, date) 的，不是整源一刀切：有官方就用官方，
    // 没官方就还是本机的值。
    const rows = [
      row("zcode", "2026-10-03", "", 900),
      row("zcode", "2026-10-03", "GLM-5.3", 600), // → glm 层
      row("zcode", "2026-10-03", "ark-seed-1.6", 300), // → volcengine 层
      // 服务端只覆盖 10-02，10-03 缺失
      providerRow("volcengine-9-0", "2026-10-02", 500),
    ];
    const map = buildUnifiedBreakdown(rows);
    const d3 = map.get("2026-10-03")!;
    expect(d3.byProvider).toEqual({ glm: 600, volcengine: 300 });
    expect(d3.total).toBe(900);
  });

  it("非 token 单位的服务端行不并入（跨单位相加是错的）", () => {
    // OpenAI/xAI 的服务端日账是 USD、Kimi 是 CNY。它们不进 token 总量。
    const rows = [
      row("codex", "2026-10-04", "", 800),
      row("codex", "2026-10-04", "gpt-5", 800),
      providerRow("openai-1-0", "2026-10-04", 3.25, "usd"),
      providerRow("xai-1-0", "2026-10-04", 1.5, "usd"),
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-04")!;
    expect(d.total).toBe(800);
    expect(d.byProvider).toEqual({ openai: 800 }); // 本机归因层原样保留
  });

  it("本机当天完全没有工具行时，服务端数据也要把那天补进日历", () => {
    // 回归：只遍历 buildLedgerBreakdown 的结果会把这类日子整段丢掉。
    // 实测本机 GLM 有账 4 天、服务端 20 天，仅重叠 2 天——合并若不补洞，
    // 服务端那一侧的数据等于白拿，日历仍是"本机视角"。
    const rows = [providerRow("volcengine-9-0", "2026-09-01", 450)];
    const map = buildUnifiedBreakdown(rows);
    expect(map.size).toBe(1);
    const d = map.get("2026-09-01")!;
    expect(d.total).toBe(450);
    expect(d.byProvider).toEqual({ volcengine: 450 });
  });

  it("服务端补出的日子与本机已有日子共存，不互相污染", () => {
    const rows = [
      row("minimax-code", "2026-09-10", "", 700),
      row("minimax-code", "2026-09-10", "MiniMax-M3", 700),
      providerRow("volcengine-9-0", "2026-09-11", 300), // 本机无此日
    ];
    const map = buildUnifiedBreakdown(rows);
    expect(map.get("2026-09-10")).toEqual({
      total: 700,
      byProvider: { minimax: 700 },
    });
    expect(map.get("2026-09-11")).toEqual({
      total: 300,
      byProvider: { volcengine: 300 },
    });
  });

  it("无服务端行时与 buildLedgerBreakdown 完全等价", () => {
    const rows = [
      row("minimax-code", "2026-10-05", "", 1000),
      row("minimax-code", "2026-10-05", "MiniMax-M3", 700),
      row("minimax-code", "2026-10-05", "deepseek-v4", 300),
    ];
    const u = buildUnifiedBreakdown(rows).get("2026-10-05")!;
    const l = buildLedgerBreakdown(rows).get("2026-10-05")!;
    expect(u).toEqual(l);
  });

  it("多个服务端 provider 各自独立替换", () => {
    const rows = [
      row("zcode", "2026-10-06", "", 1000),
      row("zcode", "2026-10-06", "ark-seed-1.6", 400),   // volcengine
      row("zcode", "2026-10-06", "GLM-5.3", 600),        // glm
      providerRow("volcengine-9-0", "2026-10-06", 250),   // 官方更小
      providerRow("volcengine-1-0", "2026-10-06", 100),   // 同 key 另一账户
    ];
    const d = buildUnifiedBreakdown(rows).get("2026-10-06")!;
    // 同 provider 的多账户服务端行先合并再替换：250 + 100 = 350
    expect(d.byProvider.volcengine).toBe(350);
    expect(d.byProvider.glm).toBe(600);
    expect(d.total).toBe(600 + 350); // 1000 − 400 本地 + 350 官方
  });

  it("重复调用同一份 rows 结果稳定（纯函数）", () => {
    // 调用方可能对同一份 rows 反复算不同口径；若 buildUnifiedBreakdown 改到
    // 共享结构上，第二次就会把官方值再叠一遍。
    const rows = [
      row("zcode", "2026-10-07", "", 300),
      row("zcode", "2026-10-07", "ark-seed-1.6", 300),
      providerRow("volcengine-9-0", "2026-10-07", 500),
    ];
    expect(buildUnifiedBreakdown(rows).get("2026-10-07")!.total).toBe(500);
    expect(buildUnifiedBreakdown(rows).get("2026-10-07")!.total).toBe(500);
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

  it("分模型合计超过总量时占比夹到 100（迁移遗留 hermes 行）", () => {
    // hermes 迁移只写模型行不写总量行 → sum(byProvider) > total → 原算法会算出
    // 100% 以上的"占比"。这是数据层瑕疵，不该以百分比外溢到 UI。
    const s = focusStats(toMap(days), win, "minimax", 50);
    expect(s.share).toBe(100);
  });
});

const BRAND: [number, number, number] = [91, 140, 255]; // minimax #5b8cff

describe("paintCell · total 模式", () => {
  it("按总量分级取 alpha，level 4 带辉光", () => {
    const p = paintCell({ mode: "total", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.bg).toBe("rgba(76,194,255,0.94)");
    expect(p.shadow).toContain("0 0 4px");
  });

  it("level 0 = 浅灰空档（保持网格可见）", () => {
    expect(paintCell({ mode: "total", level: 0, isToday: false, focusValue: 0, brandRgb: BRAND }).bg)
      .toBe("rgba(76,194,255,0)");
  });

  it("今天叠加琥珀描边", () => {
    const p = paintCell({ mode: "total", level: 2, isToday: true, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toContain("var(--tum-amber)");
  });
});

describe("paintCell · highlight 模式", () => {
  it("焦点日用品牌色 + 亮描边，alpha 仍按总量分级（大小信息不丢）", () => {
    const p = paintCell({ mode: "highlight", level: 3, isToday: false, focusValue: 500, brandRgb: BRAND });
    expect(p.bg).toBe("rgba(91,140,255,0.66)");
    expect(p.shadow).toContain("rgba(91,140,255,0.95)");
  });

  it("非焦点日压暗到 DIM_FACTOR 倍，但仍是同一个色相", () => {
    const p = paintCell({ mode: "highlight", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.bg).toBe(`rgba(76,194,255,${LEVEL_ALPHA[4] * DIM_FACTOR})`);
    expect(p.shadow).not.toContain("inset 0 0 0 1px rgba(91,140,255");
  });

  it("非焦点日不再有 level-4 辉光（避免和焦点日抢注意力）", () => {
    const p = paintCell({ mode: "highlight", level: 4, isToday: false, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toBe("none");
  });

  it("焦点日若恰是今天，两个描边都在", () => {
    const p = paintCell({ mode: "highlight", level: 2, isToday: true, focusValue: 1, brandRgb: BRAND });
    expect(p.shadow).toContain("rgba(91,140,255,0.95)");
    expect(p.shadow).toContain("var(--tum-amber)");
  });

  it("非焦点日若恰是今天，保留今天描边（今天必须始终可辨）", () => {
    const p = paintCell({ mode: "highlight", level: 1, isToday: true, focusValue: 0, brandRgb: BRAND });
    expect(p.shadow).toContain("var(--tum-amber)");
  });

  it("压暗系数是严格小于 1 的正数（不能把非焦点日抹成不可见）", () => {
    expect(DIM_FACTOR).toBeGreaterThan(0);
    expect(DIM_FACTOR).toBeLessThan(1);
    expect(LEVEL_ALPHA[4] * DIM_FACTOR).toBeGreaterThan(0.1);
  });
});

describe("hexToRgbTriplet", () => {
  it("解析 6 位十六进制（大小写皆可）", () => {
    expect(hexToRgbTriplet("#5b8cff")).toEqual([91, 140, 255]);
    expect(hexToRgbTriplet("#5B8CFF")).toEqual([91, 140, 255]);
  });

  it("3 位简写不解析（避免与 6 位语义混淆）", () => {
    expect(hexToRgbTriplet("#fff")).toEqual([76, 194, 255]);
  });

  it("rgb()/rgba() 形态同样取三元组（brand-glyphs 兜底灰 rgb 写法）", () => {
    expect(hexToRgbTriplet("rgb(138,143,152)")).toEqual([138, 143, 152]);
    expect(hexToRgbTriplet("rgba(138, 143, 152, 0.22)")).toEqual([138, 143, 152]);
  });

  it("非法输入回落到强调色，不返回 NaN", () => {
    for (const bad of ["", "not-a-color", "#12345", "5b8cff", "#gggggg"]) {
      const rgb = hexToRgbTriplet(bad);
      expect(rgb, `input=${bad}`).toEqual([76, 194, 255]);
      expect(rgb.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)).toBe(true);
    }
  });
});
