/**
 * 帧耗时埋点（方案 .scratch/glance-bench/issues/01）。
 *
 * ## 测的到底是什么（别被名字骗了）
 *
 * 三条通道，量纲各不相同，**不可互相替代**：
 *
 * - `markIpc`      量「发请求 → 拿到数据」。含网络等待。
 * - `markFrameStart` 量「数据就绪 → 下一帧**开始**」。
 *   **不含**该帧的 style/layout/paint/composite 成本——rAF 回调在提交之前就跑完了。
 *   所以 33ms 阈值的语义是「更新到帧开始的延迟」，不是「这一帧花了 33ms」。
 * - `observeLongFrames` 走 `PerformanceObserver({type:'long-animation-frame'})`，
 *   **这才是真正的帧成本**，与 Glance 的 `SLOW_FRAME` 语义对等。
 *   浏览器不支持该 entry type 时自动降级（不抛错、不占位）。
 *
 * 后续「用数据支撑这条动效该多长」这类论证**只能用 longFrames 通道**，
 * frameStart 只能回答「更新有多久才被画出来」。
 *
 * ## 两个数据污染源（都是实测踩到的）
 *
 * 1. **窗口隐藏**：本应用常驻托盘，窗口会长时间不可见。Chromium 在隐藏期把 rAF
 *    节流到 1Hz 甚至停摆，而后台推送仍在触发测量——唤醒后第一个 rAF 会记进一条
 *    几秒到几分钟的样本，永久污染 `max`。故隐藏期不测，且超上限的样本不计入。
 * 2. **同帧合并**：两次数据更新落在同一帧时，用户等的是**最早那一次**的等待时间，
 *    不是后一次。取最早起点才不失真，被合并掉的计入 `dropped`。
 *
 * @see 方案 .scratch/glance-bench/spec.md
 */

import type { PerfSnapshot, PerfSpan, PerfSources } from "./perf-types";

export type { PerfSnapshot, PerfSpan, PerfSources } from "./perf-types";

/** frameStart 通道的慢阈值（ms）。语义见文件头。 */
export const PERF_SLOW_MS = 33;

/** 单条样本的上限（ms）。超过则判为不可靠，不计入 `samples` 而计入 `unreliable`。
 *  兜住「可见但帧调度被严重阻塞」的极端情况。 */
export const PERF_SAMPLE_MAX_MS = 1000;

/** 环形缓冲容量：只保留最近这么多个样本。 */
export const PERF_CAPACITY = 120;

/** p95 在样本数低于此值时没有意义（小 n 下 p95 必然等于 max）。 */
export const PERF_MIN_N_FOR_P95 = 20;

/** 时钟与 rAF 的注入点。单测替换它，从而不依赖真实时钟与帧调度。 */
export const perfClock = {
  now: (): number => performance.now(),
  raf: (cb: () => void): void => {
    requestAnimationFrame(() => cb());
  },
};

function newStore(): { buf: Float64Array; len: number; cursor: number } {
  return { buf: new Float64Array(PERF_CAPACITY), len: 0, cursor: 0 };
}

type Store = ReturnType<typeof newStore>;

const ipc = newStore();
const frame = newStore();
const longFrames = newStore();

/** 因「同帧合并」被丢弃的测量数。门槛判定要求它为 0。 */
let dropped = 0;
/** 因超过 {@link PERF_SAMPLE_MAX_MS} 或窗口不可见而未计入的测量数。 */
let unreliable = 0;
/** `markIpc` 中失败的调用数。与成功样本分开计数，否则错误期的超时会抬高 p95。 */
let ipcFails = 0;

/** 未决 frameStart 测量的**最早**起点；null 表示没有。 */
let pendingFrame: number | null = null;

/**
 * 热路径零分配：覆写 `Float64Array` + 整数游标推进，**不**用 `push`/`shift`，
 * 也不为每个样本建对象——测性能的东西不能在热路径造垃圾。
 *
 * 注：游标回绕不改变读出正确性，因为 {@link stats} 会排序，物理顺序无关，
 * 缓冲始终装着最近 `min(count, CAPACITY)` 个值。
 * （这一点有过一次误报：用 1..200 顺序写入，读出确为 81..200。见 perf-ringbuf.test.ts。）
 */
function record(store: Store, value: number): void {
  store.buf[store.cursor] = value;
  store.cursor = (store.cursor + 1) % PERF_CAPACITY;
  if (store.len < PERF_CAPACITY) store.len++;
}

/** 最近秩分位数：`ceil(q * n) - 1`，n≥1 时恒在界内。小 n 下 p95 会等于 max，故另导出 n。 */
function statAt(sorted: number[], q: number): number {
  return sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)];
}

function stats(store: Store, fails: number, d: number, u: number): PerfSpan {
  const samples = Array.from(store.buf.subarray(0, store.len)).sort((a, b) => a - b);
  const base: PerfSpan = {
    samples,
    n: samples.length,
    p50: 0,
    p95: 0,
    max: 0,
    slow: 0,
    fails,
    dropped: d,
    unreliable: u,
    // 样本不足以谈 p95 时显式为 false，避免调用方拿 max 当 p95 用。
    p95Meaningful: samples.length >= PERF_MIN_N_FOR_P95,
  };
  if (samples.length === 0) return base;
  base.p50 = statAt(samples, 0.5);
  base.p95 = statAt(samples, 0.95);
  base.max = samples[samples.length - 1];
  base.slow = samples.reduce((n, v) => (v >= PERF_SLOW_MS ? n + 1 : n), 0);
  return base;
}

/** 文档是否可见。SSR / 单测里没有 document，此时按可见处理。 */
function visible(): boolean {
  return typeof document === "undefined" || document.visibilityState !== "hidden";
}

/**
 * 量一次 IPC 往返。
 *
 * 失败路径也计入 `samples`（一次失败的请求同样消耗了用户的等待），
 * 但**另外**累加 `fails`，好让读侧知道那批样本里混了错误——否则错误期的超时会
 * 静默抬高 p95，看不出是网络坏了还是渲染变慢。
 */
export async function markIpc<T>(fn: () => Promise<T>): Promise<T> {
  const t0 = perfClock.now();
  try {
    const out = await fn();
    record(ipc, perfClock.now() - t0);
    return out;
  } catch (err) {
    ipcFails++;
    record(ipc, perfClock.now() - t0);
    throw err;
  }
}

/**
 * 在数据写入 state 之后调用，量到「下一帧开始」为止。
 *
 * 三条约束，都来自实际踩到的坑：
 * 1. **只取第一个 rAF**。取第二个会系统性多算一整帧（约 16.7ms）。
 * 2. **同帧合并取最早起点**。两次更新落在同一帧时用户等的是最早那次的等待时间。
 *    被合并掉的计入 `dropped`（取后一次会系统性偏小，抬高不了门槛的公信力）。
 * 3. **窗口隐藏时不测**。托盘常驻导致窗口可能长时间不可见，rAF 被节流后
 *    唤醒时会记进一条分钟级的样本。超上限的样本同样计入 `unreliable`。
 *
 * ⚠️ 本函数**不含**该帧的绘制成本。要量帧成本用 {@link observeLongFrames}。
 */
export function markFrameStart(): Promise<void> {
  if (!visible()) {
    unreliable++;
    return Promise.resolve();
  }
  const start = perfClock.now();
  if (pendingFrame === null || start < pendingFrame) pendingFrame = start;
  const mine = pendingFrame;
  return new Promise<void>((resolve) => {
    perfClock.raf(() => {
      if (pendingFrame !== mine) {
        // 有更早的测量在等，本次已被合并进它。
        dropped++;
        resolve();
        return;
      }
      pendingFrame = null;
      const dt = perfClock.now() - mine;
      if (dt > PERF_SAMPLE_MAX_MS) {
        unreliable++;
      } else {
        record(frame, dt);
      }
      resolve();
    });
  });
}

/** 当前环境是否支持 `long-animation-frame`（Chromium 123+；WebView2 版本因机器而异）。 */
export function supportsLongFrames(): boolean {
  return (
    typeof PerformanceObserver !== "undefined" &&
    Array.isArray(PerformanceObserver.supportedEntryTypes) &&
    PerformanceObserver.supportedEntryTypes.includes("long-animation-frame")
  );
}

let longObserver: PerformanceObserver | null = null;

/**
 * 订阅真正的帧成本。这才是与 Glance `SLOW_FRAME` 语义对等的通道。
 *
 * 不支持时静默返回 false —— 调用方据此知道「没有帧成本数据」，
 * 而不是拿到一个假装精确的替代指标。
 */
export function observeLongFrames(): boolean {
  if (longObserver || !supportsLongFrames()) return longObserver !== null;
  try {
    longObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        record(longFrames, entry.duration);
      }
    });
    longObserver.observe({ type: "long-animation-frame", buffered: true });
    return true;
  } catch {
    longObserver = null;
    return false;
  }
}

export function stopLongFrames(): void {
  longObserver?.disconnect();
  longObserver = null;
}

/** 读取三条通道的统计。 */
export function perfSnapshot(): PerfSources {
  return {
    ipc: stats(ipc, ipcFails, 0, 0),
    frame: stats(frame, 0, dropped, unreliable),
    longFrames: stats(longFrames, 0, 0, 0),
    longFramesSupported: supportsLongFrames(),
  };
}

// ───────────────────────────────────────────────────────────────────────────
// 时间线探针（方案 issues/05 §1.2）
//
// 与上面三条通道不同：**span 测的是宿主侧 wall-clock**，而 LoAF 测的是主窗
// 渲染进程的主线程。把手窗的创建发生在宿主事件循环线程，它可以占住宿主主线程
// 却**不必然产生主窗任何 LoAF 条目**。所以「贴边卡顿」到底是宿主开销还是渲染
// 掉帧，只有把 span 时长一并看才能判——只绑 LoAF 的判读表两个方向都会判错，
// 而且都带着测量数据背书。
// ───────────────────────────────────────────────────────────────────────────

export interface SpanRecord {
  id: number;
  label: string;
  start: number;
  /** null = 尚未闭合。早退路径必须也闭合，否则样本会被静默丢弃。 */
  end: number | null;
}

/**
 * 生产构建下**完全零开销**：不记录、不分配、不进数组。
 *
 * 原方案写「函数本身保留（零成本）」是错的——调用点是无条件的，长期驻留进程
 * 里每次贴边都会 push 对象，无界增长。
 */
const SPANS_ENABLED = !import.meta.env.PROD;
const spanLog: SpanRecord[] = [];
let spanSeq = 0;

/** 开始一个区间标记，返回 id（生产下恒为 -1，`markEnd` 对 -1 是空操作）。 */
export function markSpan(label: string): number {
  if (!SPANS_ENABLED) return -1;
  const id = spanSeq++;
  spanLog.push({ id, label, start: perfClock.now(), end: null });
  if (spanLog.length > 400) spanLog.splice(0, spanLog.length - 400);
  return id;
}

/** 闭合一个区间。对 -1（生产）与重复闭合都是空操作。 */
export function markEnd(id: number): void {
  if (!SPANS_ENABLED || id < 0) return;
  const s = spanLog.find((x) => x.id === id);
  if (s && s.end === null) s.end = perfClock.now();
}

/** 读出全部区间。 */
export function spanList(): SpanRecord[] {
  return SPANS_ENABLED ? spanLog.map((s) => ({ ...s })) : [];
}

/**
 * LoAF 能力自检（方案 §1.5）。
 *
 * 判读表依赖 `long-animation-frame`，但本机 WebView2 版本是否支持不能假设。
 * 返回 supported=false 时，调用方应把判读表里依赖 LoAF 的行标为不可用，
 * 退化为「只看 span wall-clock」。
 */
export function loafSelfCheck(): boolean {
  return supportsLongFrames();
}

/** 清空全部样本与计数器。仅供测试与显式重置。 */
export function perfReset(): void {
  ipc.len = 0;
  ipc.cursor = 0;
  frame.len = 0;
  frame.cursor = 0;
  longFrames.len = 0;
  longFrames.cursor = 0;
  dropped = 0;
  unreliable = 0;
  ipcFails = 0;
  pendingFrame = null;
  if (SPANS_ENABLED) spanLog.length = 0;
}

/**
 * 开发期把读数挂到 `window.__perf`，否则仪表盘**没有读数出口**：
 * 模块导出在 Vite 打包后不是全局，控制台敲 `perfSnapshot()` 只会得到
 * `is not defined`。只挂 dev 构建，生产包不带这个全局。
 *
 * 用法（需 `npm run tauri:dev` 起开发构建，再开 DevTools）：
 *   __perf.snapshot()  → PerfSnapshot
 *   __perf.reset()     → 清空重来
 */
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as Record<string, unknown>).__perf = {
    snapshot: perfSnapshot,
    reset: perfReset,
    markSpan,
    markEnd,
    spanList,
    loafSelfCheck,
  };
}
