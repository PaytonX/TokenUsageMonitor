// 经 TokenRouter 路由的 token，在按模型/按 Provider 归因时的**替换**规则。
//
// ## 背景
//
// 工具侧的日志看不到实际承载模型：路由不改写客户端可见的模型名，MiniMax Code
// 只认得自定义 Provider 占位名 `custom_provider:*`，Codex 只认得 `LocalRouter`。
// 所以后端把这类行打成哨兵 [`ROUTED_MODEL`]，真实模型由 TokenRouter 自己的
// 记账行（`kind='router'`，`model` 为实际打到上游的模型）给出。
//
// 本模块负责把哨兵行的归因**替换**成路由台账里的真实模型，而不是靠前缀猜。
//
// ## 为什么是「替换」而不是「相加」
//
// 工具扫描器和路由台账记的是同一批 token（工具请求经 43211 转发）。直接相加
// 就是双计。`buildLedgerBreakdown` 里的「分模型合计 == 来源总量」是既有回归
// 护栏，所以替换必须**恰好消费掉哨兵那么多量**：多了按顺序截断，少了把差额
// 回退到工具兜底归因——两边都不丢量、都不虚增。
//
// ## 关联键的局限
//
// 两侧没有共同的 request id，唯一可用的关联键是**日期**。同日既走直连又走
// 路由是允许的：直连行本来就不带哨兵，不参与替换，因此只有哨兵那部分会被
// 动。代价是路由台账若还包含**未被本机扫描**的客户端（curl、指向它的其他
// 工具），那部分会被一并计入——此时截断会按行顺序丢弃尾部，属于已知偏差。

import type { UsageDailyRow } from "./api";

/** 「经 TokenRouter」哨兵模型名。镜像 `src-tauri/src/local/mod.rs::ROUTED_MODEL`。 */
export const ROUTED_MODEL = "__via_router__";

/**
 * 哨兵的展示名。模型页按模型名聚合，直接渲染会露出内部标识。
 *
 * 只有**路由台账覆盖不足**（差额进了 fallback）时才会以模型身份出现；正常情况
 * 下这些 token 已被替换成真实模型。口径与后端 `UNCLASSIFIED_MODEL`（"未标记模型"
 * 直接原样展示）一致，属展示层映射而非数据层标签。
 */
export const ROUTED_MODEL_DISPLAY = "经本地路由";

/** TokenRouter 自记账的 `usage_daily.kind`。镜像后端 `ROUTER_LEDGER_KIND`。 */
export const ROUTER_LEDGER_KIND = "router";

/** 一段可归属到具体模型的 token。 */
export interface ModelSlice {
  model: string;
  total: number;
}

export interface RoutedAttribution {
  /** 能用路由台账里真实模型解释的部分，合计 ≤ `sentinelTotal`。 */
  slices: ModelSlice[];
  /**
   * 路由台账覆盖不足（或缺失）而无法解释的量——调用方须回退到工具兜底归因，
   * 否则按模型合计会小于来源总量，回归护栏 `分模型合计 == 来源总量` 会破。
   */
  fallback: number;
}

/**
 * 选出替换哨兵行的归因切片。
 *
 * 按行顺序消费路由台账的量直到等于 `sentinelTotal`：台账多于哨兵（还包含了
 * 非本机工具客户端）时截断，少于则差额进 `fallback`。模型名为空的台账行
 * 同样计入 `fallback`——无模型可归因，如实回退而不是硬塞给某个 Provider。
 */
export function routedAttribution(
  sentinelTotal: number,
  routerRows: UsageDailyRow[],
): RoutedAttribution {
  if (sentinelTotal <= 0) return { slices: [], fallback: 0 };

  const slices: ModelSlice[] = [];
  let remaining = sentinelTotal;
  for (const r of routerRows) {
    if (remaining <= 0) break;
    if (r.total <= 0) continue;
    const take = Math.min(remaining, r.total);
    if (!r.model) {
      remaining -= take;
      continue;
    }
    const hit = slices.find((s) => s.model === r.model);
    if (hit) hit.total += take;
    else slices.push({ model: r.model, total: take });
    remaining -= take;
  }
  return { slices, fallback: remaining > 0 ? remaining : 0 };
}

/** 按日期归集路由台账行（跨多个 `router:<account>` 来源求和）。 */
export function collectRouterRowsByDate(rows: UsageDailyRow[]): Map<string, UsageDailyRow[]> {
  const byDate = new Map<string, UsageDailyRow[]>();
  for (const r of rows) {
    if (r.kind !== ROUTER_LEDGER_KIND) continue;
    const list = byDate.get(r.date);
    if (list) list.push(r);
    else byDate.set(r.date, [r]);
  }
  return byDate;
}
