/**
 * 总量页卡片拖拽排序的纯计算部分。
 *
 * 为什么把这两步拆出来：拖拽手感的问题几乎都出在「落点怎么算」和「松手后
 * 怎么重排」上，而这两件事与 DOM 无关。放在这里可以用单测钉住边界（拖到
 * 最前/最后、拖到自己身上、落点越界），组件里只负责读矩形和画线。
 */

/** 参与落点计算的一个槽位：只关心纵向范围。 */
export interface SlotExtent {
  /** 视口坐标下的上边界（getBoundingClientRect().top）。 */
  top: number;
  height: number;
}

/**
 * 指针 y → 插入下标（在**排除被拖项之后**的列表里）。
 *
 * 判据是「指针越过槽位中线就算到它后面」，而不是「离哪个中心更近」：
 * 前者在拖过一整个槽位时只翻转一次，不会在中线附近来回抖。等于中线时
 * 保持在前（`>` 而非 `>=`），让落点稳定偏上，符合从上往下拖的直觉。
 */
export function dropIndexFor(rects: readonly SlotExtent[], y: number): number {
  let index = 0;
  for (const r of rects) {
    if (y > r.top + r.height / 2) index += 1;
  }
  return index;
}

/**
 * 把 `draggedId` 从 `order` 里取出，插入到 `dropIndex` 位置。
 *
 * `dropIndex` 的坐标系与 {@link dropIndexFor} 一致：**已经排除被拖项**。
 * 越界下标会被夹到 `[0, rest.length]`，所以「拖到列表末端」和「指针离开
 * 列表」都不需要调用方另行处理。返回新数组，不修改入参。
 */
export function applyDropIndex(
  order: readonly string[],
  draggedId: string,
  dropIndex: number,
): string[] {
  const rest = order.filter((id) => id !== draggedId);
  const index = Math.max(0, Math.min(Math.floor(dropIndex), rest.length));
  rest.splice(index, 0, draggedId);
  return rest;
}
