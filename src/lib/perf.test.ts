/**
 * perf.ts 单测（方案 .scratch/glance-bench/issues/01）。
 *
 * 全部通过替换 `perfClock` 与 `document.visibilityState` 来控制时钟与可见性，
 * 不依赖真实时间与真实帧调度。
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  PERF_MIN_N_FOR_P95,
  PERF_SAMPLE_MAX_MS,
  PERF_SLOW_MS,
  SPAN_LOG_CAPACITY,
  loafSelfCheck,
  markEnd,
  markSpan,
  markIpc,
  markFrameStart,
  perfClock,
  perfReset,
  perfSnapshot,
  spanDroppedCount,
  spanList,
} from "./perf";

/** 可推进的假时钟 + 手动触发帧的假 rAF。 */
function installClock() {
  let t = 0;
  let queued: Array<() => void> = [];
  perfClock.now = () => t;
  perfClock.raf = (cb) => {
    queued.push(cb);
  };
  return {
    advance(ms: number) {
      t += ms;
    },
    /** 推进时钟后跑一帧（把队列里积压的回调一次结清）。 */
    frame(ms = 0) {
      t += ms;
      const batch = queued;
      queued = [];
      for (const cb of batch) cb();
    },
    /** 是否有未结算的帧回调。 */
    hasPending: () => queued.length > 0,
  };
}

/** 伪造 document.visibilityState（node 环境没有 document）。 */
function setVisibility(state: "visible" | "hidden") {
  (globalThis as Record<string, unknown>).document = { visibilityState: state };
}

const realNow = perfClock.now;
const realRaf = perfClock.raf;

beforeEach(() => {
  perfReset();
  setVisibility("visible");
});

afterEach(() => {
  perfClock.now = realNow;
  perfClock.raf = realRaf;
  delete (globalThis as Record<string, unknown>).document;
});

describe("perf: ipc 通道", () => {
  it("记录一次往返的耗时", async () => {
    const c = installClock();
    const p = markIpc(async () => {
      c.advance(12);
      return "ok";
    });
    c.frame();
    expect(await p).toBe("ok");
    const { ipc } = perfSnapshot();
    expect(ipc.samples).toEqual([12]);
    expect(ipc.fails).toBe(0);
  });

  it("失败路径同样计入耗时，但单独累加 fails", async () => {
    const c = installClock();
    await expect(
      markIpc(async () => {
        c.advance(7);
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    const { ipc } = perfSnapshot();
    // 失败的请求同样消耗了用户的等待。
    expect(ipc.samples).toEqual([7]);
    // 但读侧必须能知道这批样本里混了错误，否则错误期的超时会静默抬高 p95。
    expect(ipc.fails).toBe(1);
  });

  it("异常在计时落样本之后原样抛出", async () => {
    const c = installClock();
    await expect(markIpc(async () => {
      c.advance(1);
      throw new TypeError("bad shape");
    })).rejects.toThrow(TypeError);
  });
});

describe("perf: frameStart 通道的语义边界", () => {
  it("50ms 的 IPC 等待不出现在 frame 样本里", async () => {
    const c = installClock();
    // 一次完整轮询：IPC 花 50ms，拿到数据后到下一帧花 5ms。
    // 两步必须分开 await —— markIpc 内部要等一次微任务才落样本，
    // 若把两步串在同一个 async IIFE 里再统一 frame()，第二次测量还没入队就帧跑了，会死锁。
    await markIpc(async () => {
      c.advance(50);
    });
    const painted = markFrameStart();
    c.frame(5);
    await painted;

    const { ipc, frame } = perfSnapshot();
    expect(ipc.samples).toEqual([50]);
    expect(frame.samples).toEqual([5]);
  });

  it("只取第一个 rAF：一次测量只结算一个样本", async () => {
    const c = installClock();
    const p = markFrameStart();
    c.frame(4);
    await p;
    c.frame(4);
    expect(perfSnapshot().frame.samples).toHaveLength(1);
  });
});

describe("perf: 同帧合并取最早起点", () => {
  it("两次更新落在同一帧时，记录最早那次，并累加 dropped", async () => {
    const c = installClock();
    const a = markFrameStart(); // start = 0
    c.advance(5);
    const b = markFrameStart(); // start = 5，但被合并进 a
    c.frame(11); // 帧在 t = 16 开始
    await Promise.all([a, b]);

    const { frame } = perfSnapshot();
    // 用户等的是最早那次更新到画面出现的 16ms，不是 11ms。
    // 取后一次会系统性偏小，抬高不了门槛的公信力。
    expect(frame.samples).toEqual([16]);
    expect(frame.dropped).toBe(1);
  });

  it("dropped 归零后不该出现跨帧交叉样本", async () => {
    const c = installClock();
    // 连续两帧各一次测量：互不干扰，各自结算。
    const a = markFrameStart();
    c.frame(3);
    const b = markFrameStart();
    c.frame(7);
    await Promise.all([a, b]);

    const { frame } = perfSnapshot();
    expect(frame.samples).toEqual([3, 7]);
    expect(frame.dropped).toBe(0);
  });
});

describe("perf: 窗口隐藏时不测", () => {
  it("visibilityState=hidden 时不发起测量，计入 unreliable", async () => {
    setVisibility("hidden");
    const c = installClock();
    await markFrameStart();
    // 根本没有向帧队列登记。
    expect(c.hasPending()).toBe(false);
    const { frame } = perfSnapshot();
    expect(frame.samples).toHaveLength(0);
    // 托盘常驻导致窗口可能长时间不可见；这些测量不能进样本池。
    expect(frame.unreliable).toBe(1);
  });

  it("超出单样本上限的样本不计入，转入 unreliable", async () => {
    const c = installClock();
    const p = markFrameStart();
    c.frame(PERF_SAMPLE_MAX_MS + 500);
    await p;
    const { frame } = perfSnapshot();
    // 没有上限时，rAF 被节流后唤醒会记进一条分钟级的样本，永久污染 max。
    expect(frame.samples).toHaveLength(0);
    expect(frame.unreliable).toBe(1);
  });
});

describe("perf: 统计量", () => {
  it("p50 / p95 / max 在人为构造的样本上取值正确（最近秩）", async () => {
    const c = installClock();
    // 递增值 1..10 → p50 = 5，p95 = 10（最近秩 ceil(0.95*10)-1 = 9）
    for (let i = 1; i <= 10; i++) {
      const p = markFrameStart();
      c.frame(i);
      await p;
    }
    const { frame } = perfSnapshot();
    expect(frame.samples).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    expect(frame.p50).toBe(5);
    expect(frame.p95).toBe(10);
    expect(frame.max).toBe(10);
    expect(frame.n).toBe(10);
    expect(frame.slow).toBe(0);
  });

  it("样本不足时 p95Meaningful 为 false，避免调用方把 max 当 p95", async () => {
    const c = installClock();
    const p = markFrameStart();
    c.frame(5);
    await p;
    const { frame } = perfSnapshot();
    expect(frame.n).toBe(1);
    // n=1 时 p95 必然等于 max，拿它做结论没有意义。
    expect(frame.p95).toBe(frame.max);
    expect(frame.p95Meaningful).toBe(false);

    for (let i = 0; i < PERF_MIN_N_FOR_P95 - 1; i++) {
      const q = markFrameStart();
      c.frame(1);
      await q;
    }
    expect(perfSnapshot().frame.p95Meaningful).toBe(true);
  });

  it("达到阈值的样本计入 slow", async () => {
    const c = installClock();
    for (const ms of [PERF_SLOW_MS - 1, PERF_SLOW_MS, PERF_SLOW_MS + 20]) {
      const p = markFrameStart();
      c.frame(ms);
      await p;
    }
    expect(perfSnapshot().frame.slow).toBe(2);
  });

  it("无样本时全为 0，不返回 NaN", () => {
    const snap = perfSnapshot();
    for (const span of [snap.ipc, snap.frame, snap.longFrames]) {
      expect(span.samples).toEqual([]);
      expect(span.p50).toBe(0);
      expect(span.p95).toBe(0);
      expect(span.max).toBe(0);
      expect(span.n).toBe(0);
    }
  });
});

describe("perf: 时间线探针（issues/05 §1.2）", () => {
  beforeEach(() => {
    perfReset();
    installClock();
  });
  afterEach(() => {
    perfClock.now = realNow;
    perfClock.raf = realRaf;
  });

  it("markSpan/markEnd 记录区间的起止", () => {
    const c = installClock();
    const id = markSpan("dockPill.peekLead");
    c.advance(50);
    markEnd(id);
    const s = spanList().find((x) => x.id === id);
    expect(s).toBeDefined();
    expect(s!.label).toBe("dockPill.peekLead");
    expect(s!.end! - s!.start).toBe(50);
  });

  it("未闭合的区间 end 为 null（早退路径必须也能闭合）", () => {
    const id = markSpan("never-closed");
    const s = spanList().find((x) => x.id === id);
    expect(s!.end).toBeNull();
    markEnd(id);
    expect(spanList().find((x) => x.id === id)!.end).not.toBeNull();
  });

  it("markEnd 对重复闭合是幂等的（取第一次）", () => {
    const c = installClock();
    const id = markSpan("x");
    c.advance(10);
    markEnd(id);
    const first = spanList().find((s) => s.id === id)!.end;
    c.advance(999);
    markEnd(id);
    expect(spanList().find((s) => s.id === id)!.end).toBe(first);
  });

  it("markEnd 对 -1（生产）空操作，不抛", () => {
    expect(() => markEnd(-1)).not.toThrow();
  });

  it("perfReset 清空时间线", () => {
    markSpan("a");
    markSpan("b");
    expect(spanList().length).toBeGreaterThan(0);
    perfReset();
    expect(spanList()).toEqual([]);
  });

  it("环形上限：长期驻留进程不无界增长", () => {
    // 断言引用导出的容量常量，而不是写死数字——写死的话，改容量就会被
    // 误当成「放松了这条不变量」，而这条不变量恰恰是它要守的东西。
    for (let i = 0; i < SPAN_LOG_CAPACITY + 50; i++) {
      const id = markSpan(`s${i}`);
      markEnd(id);
    }
    expect(spanList().length).toBeLessThanOrEqual(SPAN_LOG_CAPACITY);
    // 最旧的被丢掉，最新的还在
    const last = SPAN_LOG_CAPACITY + 49;
    expect(spanList().some((s) => s.label === `s${last}`)).toBe(true);
    expect(spanList().some((s) => s.label === "s0")).toBe(false);
    // 被淘汰的条数要单独报出来：「日志里没有」与「被淘汰了」不能读成同一件事。
    expect(spanDroppedCount()).toBe(50);
  });

  it("spanList 返回副本，改它不影响内部状态", () => {
    markSpan("a");
    const list = spanList();
    list.length = 0;
    expect(spanList().length).toBeGreaterThan(0);
  });

  it("loafSelfCheck 返回布尔（不支持时为 false，判读表据此降级）", () => {
    expect(typeof loafSelfCheck()).toBe("boolean");
  });
});

describe("perf: 重置", () => {
  it("perfReset 清空三条通道与全部计数器", async () => {
    const c = installClock();
    await markIpc(async () => {
      c.advance(3);
    });
    const p = markFrameStart();
    const b = markFrameStart();
    c.frame(2);
    await Promise.all([p, b]);
    expect(perfSnapshot().frame.dropped).toBe(1);

    perfReset();
    const snap = perfSnapshot();
    expect(snap.ipc.samples).toHaveLength(0);
    expect(snap.frame.samples).toHaveLength(0);
    expect(snap.longFrames.samples).toHaveLength(0);
    expect(snap.frame.dropped).toBe(0);
    expect(snap.frame.unreliable).toBe(0);
    expect(snap.ipc.fails).toBe(0);
  });
});
