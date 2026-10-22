/**
 * perf-drive 单测（方案 .scratch/glance-bench/issues/05）。
 *
 * 驱动器本身**不碰**状态机，它只决定「什么时候调什么」。所以这里要钉住的是
 * **时序与计数**：
 * - 一轮 = 唤出 + 收起，且收起前必须等满宽限期（与真实「指针离开」一致）；
 * - warmup 轮不计入统计，但会先跑（冷启动不进统计）；
 * - 轮次之间按 intervalMs 间隔；
 * - 出错时已完成的轮数照实上报，不吞异常。
 *
 * 时间全部靠注入的假 sleep 推进，不依赖真实时钟。
 */
import { describe, expect, it } from "vitest";
import { drivePeekCycles, type PerfDriveHooks } from "./perf-drive";

/** 记录调用顺序的假驱动器。 */
function makeHooks(
  opts?: { ready?: boolean; failDockAt?: number },
): { calls: string[]; hooks: PerfDriveHooks } {
  const calls: string[] = [];
  let dockCount = 0;
  const hooks: PerfDriveHooks = {
    async reveal() {
      calls.push("reveal");
      return true;
    },
    async dock() {
      dockCount += 1;
      calls.push("dock");
      if (opts?.failDockAt === dockCount) throw new Error("dock 炸了");
    },
    ready: () => opts?.ready ?? true,
  };
  return { calls, hooks };
}

/** 假 sleep：记录时长后立即 resolve。 */
function fakeSleep(log: number[]): (ms: number) => Promise<void> {
  return (ms) => {
    log.push(ms);
    return Promise.resolve();
  };
}

const BASE = {
  dockDelayMs: 320,
  slideMs: 180,
  intervalMs: 1000,
};

describe("perf-drive: 一轮 = 唤出 + 收起", () => {
  it("先唤出，等满宽限期，再收起", async () => {
    const { calls, hooks } = makeHooks();
    const slept: number[] = [];
    const res = await drivePeekCycles({
      ...BASE,
      rounds: 1,
      hooks,
      sleep: fakeSleep(slept),
    });
    // warmup 一轮 + 测量 1 轮。
    expect(calls).toEqual(["reveal", "dock", "reveal", "dock"]);
    // 收起前的那段等待必须是宽限期本身，而不是「唤出后立刻收起」。
    expect(slept[0]).toBe(320);
    expect(res.rounds).toBe(1);
    expect(res.error).toBeUndefined();
  });

  it("warmup 轮不计入统计，但确实先跑了一轮", async () => {
    const { calls, hooks } = makeHooks();
    const res = await drivePeekCycles({
      ...BASE,
      rounds: 3,
      hooks,
      sleep: fakeSleep([]),
    });
    // warmup 1 轮 + 测量 3 轮。
    expect(calls.filter((c) => c === "reveal")).toHaveLength(4);
    expect(calls.filter((c) => c === "dock")).toHaveLength(4);
    expect(res.rounds).toBe(3);
  });

  it("warmup 之后清空采样：冷启动不进统计", async () => {
    const { calls, hooks } = makeHooks();
    let resets = 0;
    const res = await drivePeekCycles({
      ...BASE,
      rounds: 2,
      hooks,
      sleep: fakeSleep([]),
      resetStats: () => {
        resets += 1;
        calls.push("reset");
      },
    });
    expect(resets).toBe(1);
    // 清空必须发生在 warmup 之后、测量轮之前。
    expect(calls.indexOf("reset")).toBe(2);
    expect(res.rounds).toBe(2);
  });

  it("轮次之间按 intervalMs 间隔，最后一轮之后不间隔", async () => {
    const { hooks } = makeHooks();
    const slept: number[] = [];
    await drivePeekCycles({
      ...BASE,
      rounds: 3,
      hooks,
      sleep: fakeSleep(slept),
    });
    // 3 轮 + 1 warmup：每轮 [宽限, 滑出]，轮间 2 次 [间隔]。
    expect(slept).toEqual([
      320, 180,
      320, 180, 1000,
      320, 180, 1000,
      320, 180,
    ]);
  });
});

describe("perf-drive: 失败处理", () => {
  it("异常被捕获，已完成的轮数照实上报", async () => {
    // 第 2 次 dock 失败：warmup 那次成功，第 1 个测量轮的收起炸了。
    const { hooks } = makeHooks({ failDockAt: 2 });
    const res = await drivePeekCycles({
      ...BASE,
      rounds: 5,
      hooks,
      sleep: fakeSleep([]),
    });
    expect(res.error).toContain("dock 炸了");
    // 炸掉的那一轮**不算完成**（收起没走完，数据是半截的），
    // 所以完成轮数是 0 —— 宁可少报也不让半截数据混进统计。
    expect(res.rounds).toBe(0);
  });

  it("守卫不成立（就绪超时）时如实报错，且一轮都不跑", async () => {
    const { calls, hooks } = makeHooks({ ready: false });
    const res = await drivePeekCycles({
      ...BASE,
      rounds: 2,
      hooks,
      sleep: fakeSleep([]),
      readyTimeoutMs: 100,
    });
    expect(res.error).toContain("等待就绪超时");
    expect(res.rounds).toBe(0);
    expect(calls).toEqual([]);
  });
});
