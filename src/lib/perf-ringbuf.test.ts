/**
 * 环形缓冲读出是否错位——用可辨识的样本值直接判定，不靠推断。
 *
 * 争议点：有人认为 `record` 用游标覆写、`stats` 用 `subarray(0, len)` 读，
 * 游标回绕后读到的是「错位的陈旧数据」。
 *
 * 判定方法：写入 1..200（值 = 序号，可辨识）。缓冲容量 120 时，
 * 正确的读出应当是**最近 120 个**，即 81..200。
 * 若确实错位/陈旧，min 会落在 81 之外。
 *
 * 注意 `stats` 内部会 `sort()`，所以物理顺序不影响 p50/p95/max/slow ——
 * 只有「集合内容」错了才会导致本用例失败。
 */
import { beforeEach, describe, expect, it } from "vitest";
import {
  PERF_CAPACITY,
  markFrameStart,
  perfClock,
  perfReset,
  perfSnapshot,
} from "./perf";

let t = 0;
let queued: Array<() => void> = [];

beforeEach(() => {
  perfReset();
  t = 0;
  queued = [];
  perfClock.now = () => t;
  perfClock.raf = (cb) => {
    queued.push(cb);
  };
});

describe("环形缓冲读出", () => {
  it("写入 1..200 后读出的是最近 120 个（81..200），不是错位数据", async () => {
    expect(PERF_CAPACITY).toBe(120);
    for (let v = 1; v <= 200; v++) {
      const p = markFrameStart();
      t += v;
      const batch = queued;
      queued = [];
      for (const cb of batch) cb();
      await p;
    }

    const { frame } = perfSnapshot();
    expect(frame.samples).toHaveLength(PERF_CAPACITY);
    // 最近 120 个 = 81..200，两端都可辨识。
    expect(frame.samples[0]).toBe(81);
    expect(frame.samples[PERF_CAPACITY - 1]).toBe(200);
    expect(frame.max).toBe(200);
    // 若读出的是「上一轮尾部残留」，min 会小于 81。
    expect(frame.samples[0]).toBeGreaterThanOrEqual(81);
  });

  it("多次回绕后 min 仍等于 CAPACITY+1（陈旧样本已被完全冲掉）", async () => {
    for (let v = 1; v <= 500; v++) {
      const p = markFrameStart();
      t += v;
      const batch = queued;
      queued = [];
      for (const cb of batch) cb();
      await p;
    }
    const { frame } = perfSnapshot();
    // 最近 120 个 = 381..500。
    expect(frame.samples[0]).toBe(381);
    expect(frame.samples[PERF_CAPACITY - 1]).toBe(500);
  });
});
