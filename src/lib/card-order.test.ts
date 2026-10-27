import { describe, expect, it } from "vitest";
import { applyDropIndex, dropIndexFor } from "./card-order";

/** 三个 40px 高的槽位，从 y=100 开始。 */
const slots = [
  { top: 100, height: 40 },
  { top: 148, height: 40 },
  { top: 196, height: 40 },
];

describe("dropIndexFor", () => {
  it("指针在各槽位中线之前 → 插到该槽位前面", () => {
    expect(dropIndexFor(slots, 100)).toBe(0);
    expect(dropIndexFor(slots, 119)).toBe(0);
    expect(dropIndexFor(slots, 150)).toBe(1);
    expect(dropIndexFor(slots, 198)).toBe(2);
  });

  it("越过每个槽位中线后下标加一；末端返回 length", () => {
    // 120 正好是第一个槽位的中线：保持在前，避免边界抖动。
    expect(dropIndexFor(slots, 120)).toBe(0);
    expect(dropIndexFor(slots, 121)).toBe(1);
    expect(dropIndexFor(slots, 169)).toBe(2);
    expect(dropIndexFor(slots, 300)).toBe(3);
  });

  it("空列表返回 0（拖到空容器也安全）", () => {
    expect(dropIndexFor([], 500)).toBe(0);
  });
});

describe("applyDropIndex", () => {
  const order = ["a", "b", "c", "d"];

  it("向下拖：从前往后插到目标之后", () => {
    // b 拖到 c 后面（排除 b 后列表是 a,c,d，落点 2）。
    expect(applyDropIndex(order, "b", 2)).toEqual(["a", "c", "b", "d"]);
  });

  it("向上拖：插到目标之前", () => {
    expect(applyDropIndex(order, "d", 1)).toEqual(["a", "d", "b", "c"]);
  });

  it("拖到最前 / 最后", () => {
    expect(applyDropIndex(order, "c", 0)).toEqual(["c", "a", "b", "d"]);
    expect(applyDropIndex(order, "a", 3)).toEqual(["b", "c", "d", "a"]);
  });

  it("落点越界被夹住，不会丢项", () => {
    expect(applyDropIndex(order, "b", 99)).toEqual(["a", "c", "d", "b"]);
    expect(applyDropIndex(order, "b", -5)).toEqual(["b", "a", "c", "d"]);
  });

  it("原地拖（落点等于原位置）结果不变，且不修改入参", () => {
    const copy = [...order];
    expect(applyDropIndex(order, "b", 1)).toEqual(order);
    expect(order).toEqual(copy);
  });

  it("id 不在列表里时只在落点插入，不移除任何项", () => {
    expect(applyDropIndex(order, "x", 2)).toEqual(["a", "b", "x", "c", "d"]);
  });
});
