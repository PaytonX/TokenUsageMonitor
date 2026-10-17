/**
 * pill-hover-state 单测（方案 .scratch/glance-bench/issues/03）。
 *
 * 每条用例对应一个真实场景或一个已踩过的坑；原先它们只能靠人眼重放，
 * 回归时没人会记得逐条重跑。结构对齐 glance `detector.rs` 自己的单测写法。
 */
import { describe, expect, it } from "vitest";
import {
  HOVER_GRACE_MS,
  initialHoverState,
  reduceHover,
  runHover,
  type HoverEvent,
} from "./pill-hover-state";

/** 先把胶囊置为可交互（展开态），再跑其余事件。 */
function interactive(): HoverEvent[] {
  return [{ type: "structure", interactive: true, at: 0 }];
}

describe("pill-hover: 基本显隐", () => {
  it("可交互 + 指针进入 → 显示", () => {
    expect(runHover([...interactive(), { type: "enter", at: 10 }]).visible).toBe(true);
  });

  it("不可交互时指针进入也不显示", () => {
    expect(runHover([{ type: "enter", at: 10 }]).visible).toBe(false);
  });

  it("离开后经宽限才收起", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "leave", at: 100 },
      { type: "tick", at: 100 + HOVER_GRACE_MS - 1 },
    ]);
    expect(s.visible).toBe(true);
  });
});

describe("pill-hover: 拖拽冻结不得吃掉在途的宽限", () => {
  it("指针离开后在宽限内开始拖拽：解冻后按钮仍会按剩余宽限收起", () => {
    // 已踩过的坑：drag active 把 hideAt 清成 null，于是 pushHover 不再重挂
    // 计时器；drag inactive 也不重挂。按钮就永久卡在可见态，直到用户再次
    // 划过胶囊才自愈。
    const leaveAt = 100;
    const frozenAt = leaveAt + 20; // 宽限期内开始拖拽
    const prefix: HoverEvent[] = [
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "leave", at: leaveAt },
      { type: "drag", active: true, pointerInside: false, at: frozenAt },
      { type: "drag", active: false, pointerInside: false, at: 400 },
    ];
    // 解冻后 hideAt 必须仍在 —— 否则这条路径必卡。
    const frozen = runHover(prefix);
    expect(frozen.hideAt).toBe(leaveAt + HOVER_GRACE_MS);
    // 且宽限到期后确实收起。
    const after = reduceHover([{ type: "tick", at: leaveAt + HOVER_GRACE_MS }], frozen);
    expect(after.visible).toBe(false);
  });

  it("冻结期间 tick 不消费宽限：解冻后余量还在", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "leave", at: 0 },
      { type: "drag", active: true, pointerInside: false, at: 10 },
      { type: "tick", at: 10_000 },
    ]);
    expect(s.visible).toBe(true);
    expect(s.hideAt).toBe(HOVER_GRACE_MS);
  });
});

describe("pill-hover: 解冻对账", () => {
  it("冻结期进来的 enter 被丢弃，解冻时按指针位置补回", () => {
    // 拖拽期间指针离开又回到胶囊上——这些 enter 全被 break 掉了。
    // 松手时指针仍在胶囊上，按钮必须出现，否则「拖完停在胶囊上准备关闭」
    // 这个最自然的序列会失灵。
    const s = runHover([
      ...interactive(),
      { type: "drag", active: true, pointerInside: true, at: 0 },
      { type: "enter", at: 10 },
      { type: "drag", active: false, pointerInside: true, at: 20 },
    ]);
    expect(s.visible).toBe(true);
  });

  it("解冻时指针已不在胶囊上：不得凭空显示", () => {
    const s = runHover([
      ...interactive(),
      { type: "drag", active: true, pointerInside: true, at: 0 },
      { type: "enter", at: 10 },
      { type: "drag", active: false, pointerInside: false, at: 20 },
    ]);
    expect(s.visible).toBe(false);
  });

  it("冻结期间 enter 不让按钮自己冒出来", () => {
    const s = runHover([
      ...interactive(),
      { type: "drag", active: true, pointerInside: false, at: 0 },
      { type: "enter", at: 5 },
    ]);
    expect(s.visible).toBe(false);
  });
});

describe("pill-hover: 结构性变化", () => {
  it("胶囊不可交互时立即隐藏并清掉计时", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "leave", at: 10 },
      { type: "structure", interactive: false, at: 20 },
    ]);
    expect(s.visible).toBe(false);
    expect(s.hideAt).toBeNull();
  });

  it("dock→reveal→dock 连续三轮不残留可见态", () => {
    const s = runHover([
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "structure", interactive: false, at: 10 },
      { type: "enter", at: 20 },
      { type: "structure", interactive: true, at: 30 },
      { type: "enter", at: 40 },
      { type: "structure", interactive: false, at: 50 },
    ]);
    expect(s.visible).toBe(false);
  });
});

describe("pill-hover: 状态是不可变的", () => {
  it("reduceHover 不修改传入的初态", () => {
    const start = initialHoverState();
    const before = { ...start };
    reduceHover([...interactive(), { type: "enter", at: 0 }], start);
    expect(start).toEqual(before);
  });

  it("同一串事件跑两次结果一致（纯函数）", () => {
    const ev: HoverEvent[] = [
      ...interactive(),
      { type: "enter", at: 0 },
      { type: "leave", at: 5 },
      { type: "tick", at: 5 + HOVER_GRACE_MS },
      { type: "enter", at: 300 },
    ];
    expect(runHover(ev)).toEqual(runHover(ev));
  });

  it("连续高频 enter 不会饿死 tick（宽限期能正常到期）", () => {
    // 每次 enter 都会把 hideAt 清掉——这正是防抖所需的；只要 enter 之间
    // 存在 leave，tick 就仍然有机会到期。这里断言「一直 enter」不会卡住。
    const ev: HoverEvent[] = [...interactive()];
    for (let i = 0; i < 100; i++) ev.push({ type: "enter", at: i * 10 });
    const s = runHover(ev);
    expect(s.visible).toBe(true);
    expect(s.hideAt).toBeNull();
  });
});
