/**
 * 额度类用量轨道（百分比制 Provider）。
 *
 * 背景：OpenCode Go、MiniMax Token Plan 这类订阅报的是「已用额度百分比」，
 * 没有 token、也没有绝对金额。它们的日值现在也写进了统一账本
 * （`kind='provider'`、`unit='percent'`），但**不能**和 token 相加，也不能
 * 走 token 日历 —— 所以统计页需要一条独立的轨道（`QuotaLane`）来呈现。
 *
 * 轨迹口径：账本里 percent 行的日值 = 当日消耗的额度百分点（由 provider 侧按
 * 窗口差分累计而来，与 token 无关）。本模块只做纯聚合，不读后端。
 *
 * ## 多个 API Key 怎么统一成「账户整体用量」
 *
 * 一个 API Key 在 TUM 里就是一个账户（账户 id = 账本 source）。同一个
 * OpenCode 订阅下可以签发多个 Key，它们读的是服务端同一份额度：
 *
 * 1. **同一订阅的多个 Key 必须去重**，只算一份。判据是订阅指纹：把各窗口的
 *    `reset_at` 拼起来 —— 同一订阅的所有 Key 看到的是同一批重置时刻。没有
 *    reset_at 可用时退回用账户 id，宁可按"两个订阅"算，也不把两份额度当成一份。
 * 2. **不同订阅并列时取均值，不相加**。N 个订阅的总容量是 N×100%，"整体已用
 *    X%"按容量归一 = 各订阅百分比的均值；相加只会得到 200% 这种没有意义的数。
 *
 * 这两条决定了本模块的聚合结果：`group.total` 是容量归一的整体用量，
 * `group.keys` / `group.subscriptions` 告诉你这个数字背后有几个 Key、几份额度。
 */
import type { UsageDailyRow } from "./api";
import type { UsageSnapshot, UsageUnit } from "./types";
import { providerShortName } from "./types";

/** 账本里算作「额度轨道」的单位。 */
export function isPercentUnit(unit: UsageUnit | string | undefined | null): boolean {
  return unit === "percent";
}

export interface QuotaDay {
  /** YYYY-MM-DD（本地日）。 */
  date: string;
  /** 当日消耗的额度百分点。 */
  value: number;
}

/** 一条账户级额度序列（= 一个 API Key 的观测）。 */
export interface QuotaSeries {
  /** 账户 id，同时也是账本 source。 */
  source: string;
  /** 订阅指纹：同一订阅的多个 Key 共享同一个值。 */
  subscription: string;
  kind: string;
  /** 区间内逐日；缺数据的日期补 0。 */
  days: QuotaDay[];
  total: number;
  peak: number;
}

/** 一个 kind 的整体视图（多 Key / 多订阅合并后的结果）。 */
export interface QuotaGroup {
  kind: string;
  /** 展示名（如 "OpenCode"）。 */
  label: string;
  /** 参与的 Key 数。 */
  keys: number;
  /** 去重后的订阅份数，也就是容量倍数。 */
  subscriptions: number;
  /**
   * 容量归一的逐日值：同一订阅只算一次，再对订阅取均值。
   * 单订阅时它就是那条订阅自己的序列。
   */
  days: QuotaDay[];
  /** 区间累计（容量归一）。 */
  total: number;
  /** 本自然月累计（容量归一）：月度限额制 Provider 的「本期已用」。 */
  monthTotal: number;
  /** 单日峰值（容量归一）。 */
  peak: number;
  /** 账户明细，按累计用量降序。 */
  series: QuotaSeries[];
}

/**
 * 订阅指纹。同一订阅签发的多个 Key 在服务端看到同一批窗口重置时刻，因此
 * 用各窗口 `reset_at` 组合作为指纹；一个都没有时无法证明是同一份额度，
 * 退回用账户 id（= 当成独立订阅）。
 */
export function subscriptionFingerprint(snapshot: UsageSnapshot): string {
  const windows = snapshot.windows ?? {};
  const resets = [windows.five_hour, windows.weekly, windows.monthly]
    .map((w) => (w?.reset_at ?? "").trim())
    .join("|");
  const kind = snapshot.provider_id.split("-")[0];
  if (resets.replace(/\|/g, "") === "") return `${kind}#${snapshot.provider_id}`;
  return `${kind}@${resets}`;
}

/** 「YYYY-MM-DD」→ 日期；非法输入返回 null。 */
function parseDay(date: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  if (!m) return null;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return Number.isNaN(d.getTime()) ? null : d;
}

function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

/**
 * 生成以 `end` 结尾、共 `days` 天的日期序列（升序）。区间必须连续，
 * 这样折线/迷你条才不会被"没数据的哪天"隐形跳过。
 */
export function dayWindow(days: number, end: string): string[] {
  const tail = parseDay(end) ?? new Date();
  const n = Math.max(1, Math.floor(days));
  const out: string[] = [];
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(tail.getTime() - i * 86_400_000);
    out.push(isoDay(d));
  }
  return out;
}

export interface BuildQuotaOptions {
  /** 账户 id → provider kind。 */
  kindOf: (source: string) => string | undefined;
  /** 账户 id → 订阅指纹。缺省时按独立订阅处理。 */
  subscriptionOf?: (source: string) => string | undefined;
  /** 区间天数。 */
  days: number;
  /** 区间最后一天（YYYY-MM-DD）。 */
  end: string;
}

/**
 * 把账本里 percent 行聚成账户级序列。
 *
 * 只认 `unit==='percent'` 的行：这是"额度轨道"的唯一判据，不依赖 provider
 * 名单，新接入的百分比制 Provider 自动就能进这条轨道。
 */
export function buildQuotaSeries(
  rows: readonly UsageDailyRow[],
  opts: BuildQuotaOptions,
): QuotaSeries[] {
  const window = dayWindow(opts.days, opts.end);
  const inWindow = new Set(window);
  const bySource = new Map<string, Map<string, number>>();

  for (const row of rows) {
    if (!isPercentUnit(row.unit)) continue;
    const date = row.date.trim().slice(0, 10);
    if (!inWindow.has(date)) continue;
    let perDay = bySource.get(row.source);
    if (!perDay) {
      perDay = new Map<string, number>();
      bySource.set(row.source, perDay);
    }
    // 同一 source+date 理论上只有一行；真出现多行（未来拆分模型）就求和 ——
    // 同一天同一账户的百分点是可加的。
    perDay.set(date, (perDay.get(date) ?? 0) + (Number(row.total) || 0));
  }

  const out: QuotaSeries[] = [];
  for (const [source, perDay] of bySource) {
    const kind = opts.kindOf(source) ?? source.split("-")[0];
    const days = window.map((date) => ({ date, value: perDay.get(date) ?? 0 }));
    const total = days.reduce((sum, d) => sum + d.value, 0);
    const peak = days.reduce((m, d) => Math.max(m, d.value), 0);
    out.push({
      source,
      subscription: opts.subscriptionOf?.(source) ?? `${kind}#${source}`,
      kind,
      days,
      total,
      peak,
    });
  }
  return out.sort((a, b) => b.total - a.total);
}

/** 对若干逐日序列取均值（长度一致时逐点平均）。 */
function meanDays(list: QuotaDay[][]): QuotaDay[] {
  if (list.length === 0) return [];
  const base = list[0];
  return base.map((cell, i) => {
    const sum = list.reduce((acc, days) => acc + (days[i]?.value ?? 0), 0);
    return { date: cell.date, value: sum / list.length };
  });
}

/**
 * 按 kind 合并成整体视图。
 *
 * 关键点：
 * - 同一订阅的多个 Key 只算一份（否则同一份额度被报成两倍）；
 * - 多订阅取均值而非相加（容量归一，见文件头）。
 */
export function groupQuotaSeries(series: readonly QuotaSeries[]): QuotaGroup[] {
  const byKind = new Map<string, QuotaSeries[]>();
  for (const s of series) {
    const list = byKind.get(s.kind);
    if (list) list.push(s);
    else byKind.set(s.kind, [s]);
  }

  const groups: QuotaGroup[] = [];
  for (const [kind, members] of byKind) {
    // 一个订阅只保留一条代表序列（取累计用量最高的那条 Key 作为代表，
    // 同一订阅内各 Key 的观测差异只来自轮询时刻，不代表不同额度）。
    const bySub = new Map<string, QuotaSeries>();
    for (const m of members) {
      const prev = bySub.get(m.subscription);
      if (!prev || m.total > prev.total) bySub.set(m.subscription, m);
    }
    const subscriptions = [...bySub.values()];
    const days = meanDays(subscriptions.map((s) => s.days));
    const total = days.reduce((sum, d) => sum + d.value, 0);
    // tsconfig 的 target/lib 是 ES2021，`Array.prototype.at` 属于 ES2022，
    // 用下标取末位（这里不是热路径，可读性差异无所谓）。
    const lastDay = days.length > 0 ? days[days.length - 1].date : "";
    const monthPrefix = lastDay.slice(0, 7);
    const monthTotal = days
      .filter((d) => d.date.startsWith(monthPrefix))
      .reduce((sum, d) => sum + d.value, 0);
    const peak = days.reduce((m, d) => Math.max(m, d.value), 0);
    groups.push({
      kind,
      label: providerShortName(kind, kind),
      keys: members.length,
      subscriptions: subscriptions.length,
      days,
      total,
      monthTotal,
      peak,
      series: [...members].sort((a, b) => b.total - a.total),
    });
  }
  return groups.sort((a, b) => b.total - a.total);
}
