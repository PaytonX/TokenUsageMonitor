/**
 * pill-hover-state 单测（方案 .scratch/glance-bench/issues/03）。
 *
 * 这几条用例对应的是「人工验收最容易翻车」的场景——原先它们只能靠人眼重放，
 * 回归时没人会记得逐条重跑。结构对齐 glance `detector.rs` 自己的单测写法。
 */
import { describe, expect, it } from "vitest";
import {
  HOVER_GRACE_MS,
  initialHoverState,
  reduceHover,
  runHover,
  type HoverEvent,
  type HoverState,
} from "./pill-hover-state";

/** 先把胶囊置为可交互（展开态），再跑其余事件。 */
function interactive(): HoverEvent[] {
  return [{ type: "structure", visible: true, at: 0 }];
}

describe("pill-hover: 基本显隐", () => {
  it("胶囊可交互 + 指针进入 → 显示", () => {
    const s = runHover([...interactive(), { type: "enter", zone: "pill", at: 10 }]);
    expect(s.visible).toBe(true);
  });

  it("胶囊不可交互时指针进入也不显示", () => {
    const s = runHover([{ type: "enter", zone: "pill", at: 10 }]);
    expect(s.visible).toBe(false);
  });

  it("离开后经宽限才收起", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "leave", at: 100 },
      { type: "tick", at: 100 + HOVER_GRACE_MS - 1 },
    ]);
    expect(s.visible).toBe(true);
  });
});

describe("pill-hover: 从左划到右（最易翻车处）", () => {
  it("指针依次经过 pill → gap → strip，全程不隐藏", () => {
    // 三个区域连扫：胶囊、按钮与胶囊之间的缝隙、按钮条。
    // 漏掉 gap 的话，指针横穿那一格时按钮条会闪一下。
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "leave", at: 1 },
      { type: "enter", zone: "gap", at: 2 },
      { type: "leave", at: 3 },
      { type: "enter", zone: "strip", at: 4 },
    ]);
    expect(s.visible).toBe(true);
    expect(s.hideAt).toBeNull();
  });

  it("缝隙停留超过宽限也应保持可见（它仍是交互区的一部分）", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "gap", at: 0 },
      { type: "tick", at: 10_000 },
    ]);
    // gap 被算作「在内」，所以 tick 不该把它收起。
    expect(s.visible).toBe(true);
  });

  it("扫掠 200 次不产生隐藏事件", () => {
    const ev: HoverEvent[] = [...interactive()];
    for (let i = 0; i < 200; i++) {
      const zone = (["pill", "gap", "strip"] as const)[i % 3];
      ev.push({ type: "enter", zone, at: i * 2 });
      ev.push({ type: "leave", at: i * 2 + 1 });
      ev.push({ type: "enter", zone, at: i * 2 + 2 });
    }
    const s = runHover(ev);
    expect(s.visible).toBe(true);
  });
});

describe("pill-hover: 拖拽冻结", () => {
  it("冻结期间 leave 不启动计时", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "drag", active: true, at: 10 },
      { type: "leave", at: 20 },
      { type: "tick", at: 10_000 },
    ]);
    // 拖到一半被收起判定拽回屏幕边缘是已知故障（App.svelte:616-617）。
    expect(s.visible).toBe(true);
  });

  it("冻结期间 enter 不让按钮条自己冒出来", () => {
    const s = runHover([
      ...interactive(),
      { type: "drag", active: true, at: 0 },
      { type: "enter", zone: "strip", at: 5 },
    ]);
    expect(s.visible).toBe(false);
  });

  it("冻结会作废在途的收起计时，解冻后宁可多显示一会儿", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "leave", at: 10 },
      { type: "drag", active: true, at: 20 },
    ]);
    expect(s.hideAt).toBeNull();
    expect(s.frozen).toBe(true);
  });
});

describe("pill-hover: 结构性变化", () => {
  it("胶囊收起时立即隐藏并清掉计时", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "leave", at: 10 },
      { type: "structure", visible: false, at: 20 },
    ]);
    expect(s.visible).toBe(false);
    expect(s.hideAt).toBeNull();
  });

  it("dock→reveal→dock 连续三轮不残留可见态", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "structure", visible: false, at: 10 },
      { type: "enter", zone: "pill", at: 20 },
      { type: "structure", visible: true, at: 30 },
      { type: "enter", zone: "pill", at: 40 },
      { type: "structure", visible: false, at: 50 },
    ]);
    expect(s.visible).toBe(false);
  });
});

describe("pill-hover: 状态是不可变的", () => {
  it("reduceHover 不修改传入的初态", () => {
    const start = initialHoverState();
    const before = { ...start };
    reduceHover([...interactive(), { type: "enter", zone: "pill", at: 0 }], start);
    expect(start).toEqual(before);
  });

  it("同一串事件跑两次结果一致（纯函数）", () => {
    const ev: HoverEvent[] = [
      ...interactive(),
      { type: "enter", zone: "pill", at: 0 },
      { type: "leave", at: 5 },
      { type: "tick", at: 5 + HOVER_GRACE_MS },
      { type: "enter", zone: "strip", at: 300 },
    ];
    const a: HoverState = runHover(ev);
    const b: HoverState = runHover(ev);
    expect(a).toEqual(b);
  });
});
