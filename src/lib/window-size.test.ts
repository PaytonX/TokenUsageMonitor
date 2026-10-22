import { describe, expect, it } from "vitest";
import {
  DASHBOARD_SIZE,
  PILL_COLLAPSED_SIZE,
  PILL_MINI_GAP,
  PILL_MINI_ROW,
  createSizeWriteQueue,
  pillExpandedHeight,
  shouldApplySizeWrite,
  type WindowMode,
  type WindowSize,
} from "./window-size";

describe("shouldApplySizeWrite", () => {
  it("同模式的写要落地", () => {
    expect(shouldApplySizeWrite("compact", "compact")).toBe(true);
    expect(shouldApplySizeWrite("dashboard", "dashboard")).toBe(true);
  });

  it("跨模式的写必须跳过：那笔尺寸属于旧模式", () => {
    // 竞态现场：胶囊展开的「168×150」晚于 Rust 的「切全窗态 400×680」落地。
    // 若不跳过，全窗态窗口会被改回 168 宽，dashboard 就被塞进胶囊宽度的
    // 窗口里（标签栏中文竖排、标题被裁、下拉框出界）。
    expect(shouldApplySizeWrite("compact", "dashboard")).toBe(false);
    expect(shouldApplySizeWrite("dashboard", "compact")).toBe(false);
  });
});

describe("pillExpandedHeight", () => {
  it("没有明细行时等于收起态高度", () => {
    // 0 行时算式给 60（比收起态的 56 高 4px）——那是明细区上下内边距的
    // 预留，行数靠 CSS 折叠。锁住这个值是为了让「行数 → 高度」不再散落
    // 在调用点上。
    expect(pillExpandedHeight(0)).toBe(60);
  });

  it("每多一行加 30px（行高 24 + 间距 6）", () => {
    expect(pillExpandedHeight(1)).toBe(90);
    expect(pillExpandedHeight(2)).toBe(120);
    expect(pillExpandedHeight(3)).toBe(150);
  });

  it("行高与间距的常量别被改坏", () => {
    expect(PILL_MINI_ROW).toBe(24);
    expect(PILL_MINI_GAP).toBe(6);
  });
});

describe("窗口尺寸常量", () => {
  it("与 Rust set_window_mode 保持一致", () => {
    // 改这几个数必须同步改 Rust ipc.rs::set_window_mode，否则会出现
    // 「内容按 A 尺寸排、窗口按 B 尺寸」的错位。
    expect(PILL_COLLAPSED_SIZE).toEqual({ width: 168, height: 56 });
    expect(DASHBOARD_SIZE).toEqual({ width: 400, height: 680 });
  });
});

describe("createSizeWriteQueue · 竞态本身", () => {
  /** 造一个队列：记录真正落地的尺寸；`current` 可随时改，模拟模式变化。 */
  function makeQueue() {
    const applied: WindowSize[] = [];
    const state = { current: "compact" as WindowMode, switching: false };
    const queue = createSizeWriteQueue({
      shouldApply: (forMode) =>
        !state.switching && shouldApplySizeWrite(forMode, state.current),
      apply: async (size) => {
        applied.push(size);
      },
    });
    return { applied, state, queue };
  }

  it("同模式的写要落地", async () => {
    const { applied, queue } = makeQueue();
    await queue.enqueue("compact", { width: 168, height: 150 });
    expect(applied).toEqual([{ width: 168, height: 150 }]);
  });

  it("模式在落地前变了 → 这笔写必须被跳过", async () => {
    // 这就是用户截图那次的竞态：胶囊展开的「168×150」在途，模式切成了
    // dashboard，那笔写晚到一步落地 → 全窗态窗口被改回 168 宽，
    // dashboard 就被塞进了胶囊宽度的窗口（中文标签竖排成一列）。
    const { applied, state, queue } = makeQueue();
    const pending = queue.enqueue("compact", { width: 168, height: 150 });
    state.current = "dashboard"; // 模式在这笔写落地之前就切了
    await pending;
    expect(applied).toEqual([]);
  });

  it("切模式进行中 → 任何写都作废", async () => {
    // 切模式期间 `mode` 还没翻（要等 Rust 改完尺寸才翻），这时冒出来的
    // 胶囊尺寸写看到「模式还是旧的」，以为自己也有效——必须由这个闸门拦住。
    const { applied, state, queue } = makeQueue();
    state.switching = true;
    await queue.enqueue("compact", { width: 168, height: 150 });
    expect(applied).toEqual([]);
  });

  it("切模式收尾补写能把尺寸纠正回来", async () => {
    const { applied, state, queue } = makeQueue();
    state.switching = true;
    await queue.enqueue("compact", { width: 168, height: 150 });
    state.switching = false;
    state.current = "dashboard";
    await queue.enqueue("dashboard", DASHBOARD_SIZE);
    expect(applied).toEqual([DASHBOARD_SIZE]);
  });

  it("写是串行的：落地顺序 = 排队顺序", async () => {
    const order: string[] = [];
    const queue = createSizeWriteQueue({
      shouldApply: () => true,
      apply: async (size) => {
        order.push(`${size.width}x${size.height}`);
      },
    });
    await Promise.all([
      queue.enqueue("compact", { width: 1, height: 1 }),
      queue.enqueue("compact", { width: 2, height: 2 }),
      queue.enqueue("compact", { width: 3, height: 3 }),
    ]);
    expect(order).toEqual(["1x1", "2x2", "3x3"]);
  });

  it("一笔写失败不会毒死队列", async () => {
    const applied: WindowSize[] = [];
    const queue = createSizeWriteQueue({
      shouldApply: () => true,
      apply: async (size) => {
        if (size.width === 1) throw new Error("boom");
        applied.push(size);
      },
    });
    await expect(queue.enqueue("compact", { width: 1, height: 1 })).rejects.toThrow(
      "boom",
    );
    await queue.enqueue("compact", { width: 2, height: 2 });
    expect(applied).toEqual([{ width: 2, height: 2 }]);
  });

  it("drain 等到已排队的写全部落地", async () => {
    const applied: WindowSize[] = [];
    const queue = createSizeWriteQueue({
      shouldApply: () => true,
      apply: async (size) => {
        applied.push(size);
      },
    });
    void queue.enqueue("compact", { width: 168, height: 150 });
    await queue.drain();
    expect(applied).toEqual([{ width: 168, height: 150 }]);
  });
});
