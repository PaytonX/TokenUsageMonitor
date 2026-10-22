/**
 * 贴边「唤出 → 收起」无人值守驱动器（方案 .scratch/glance-bench/issues/05）。
 *
 * ## 为什么需要它
 *
 * 贴边卡顿是**偶发**的：手点二十次可能只复现一次。想把它变成可复现、可重复、
 * 可脚本化的实验，只有两条路——真人反复手点（不可控、样本带手抖），或者把这条
 * 交互链接到驱动上无人值守跑 N 轮。本模块是后者。
 *
 * ## 它走的是哪条路（**不改任何交互行为**）
 *
 * 驱动器不重写状态机，只像「按按钮」一样调用现有的 `revealPill()` / `dockPill()`：
 *
 * - 世代号（`peekGen` / `pillResizeGen`）由被调函数自己推进，驱动器**不碰**；
 * - `pillDragActive` 冻结机制原样生效（唤出守卫里会读它）；
 * - `revealPill()` / `dockPill()` 里已有的 span 探针照常打点，
 *   于是测出来的数据与手点**同源**。
 *
 * ## 与真人操作的差异（必须如实记进报告，别当成完全等价）
 *
 * 1. 真人 hover 把手有 60ms 防抖（peek 页的 `HOVER_DEBOUNCE_MS`），驱动器直接
 *    调 `revealPill()`，**不含**这段防抖。防抖只平移「唤出」的时刻，不改变
 *    唤出之后的时间线，对卡顿测量无影响。
 * 2. 真人离开胶囊后有 320ms 宽限（`scheduleDockPill` 的 `PILL_DOCK_DELAY_MS`），
 *    期间胶囊**仍保持唤出态**。驱动器**显式等满**这 320ms 再调 `dockPill()`，
 *    与真实时序一致——这是本模块唯一一处必须精确复刻的时序，否则
 *    `dockPill` 里两个 `setTimeout` 相对「唤出」的偏移全变了，数据失真。
 * 3. 真人下一次 hover 的时刻是随机的；驱动器用固定 `intervalMs`。
 * 4. 冷启动（WebView2 首次建 peek 窗口 + 运行时初始化）落在 warmup 轮，并在
 *    warmup 之后清空采样，**不计入统计**。
 *
 * @see 方案 .scratch/glance-bench/issues/05-dock-stutter.md
 */

import { PERF_PROBE_ENABLED } from "./perf";

/** 驱动器与 App.svelte 之间的接线。驱动器只通过这几个回调驱动现有交互。 */
export interface PerfDriveHooks {
  /**
   * 唤出。**必须**是 `onPeekHover` 回调体的同源实现：
   * `if (pillDocked && !pillCollapsing && !pillDragActive) await revealPill();`
   *
   * 返回 true = 这一轮真的唤出了；false = 守卫不成立（本轮没唤出）。
   */
  reveal(): Promise<boolean>;
  /** 收起。等价于 `dockPill()`（`scheduleDockPill` 的定时器到期效果）。 */
  dock(): Promise<void>;
  /** 唤出守卫当前是否成立。驱动器用它等状态就绪。 */
  ready(): boolean;
}

export interface PerfDriveOptions {
  /** 测量轮数（**不含** warmup 轮）。 */
  rounds: number;
  /** 每轮之间的间隔（ms）。 */
  intervalMs: number;
  /** 收起前的停留时长（ms）——须等于真实「指针离开」链路的宽限期。 */
  dockDelayMs: number;
  /** 收起动画时长（ms）——等 `dockPill` 的时间线走完，下一轮的守卫才可能成立。 */
  slideMs: number;
  hooks: PerfDriveHooks;
  /** warmup 轮之后清空探针采样，使冷启动不进入统计。 */
  resetStats?: () => void;
  /** 等就绪的最长时间（ms）。 */
  readyTimeoutMs?: number;
  /** 睡眠注入（单测用）。 */
  sleep?: (ms: number) => Promise<void>;
}

export interface PerfDriveResult {
  /** 实际完成的测量轮数（不含 warmup）。 */
  rounds: number;
  /** 失败原因（异常消息）；无异常时为 undefined。 */
  error?: string;
  /** 守卫不成立、本轮没有真正唤出的次数。正常流程下应为 0。 */
  blocked: number;
}

/** 等就绪的默认上限（ms）。 */
const DEFAULT_READY_TIMEOUT_MS = 3000;
/** 等就绪时的轮询间隔（ms）。 */
const READY_POLL_MS = 20;
/** 就绪等待失败时的提示。 */
const READY_TIMEOUT_MSG =
  "驱动器等待就绪超时：胶囊不在可唤出状态（未贴边 / 正在拖拽 / 非 compact 模式）";

function realSleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * 跑 N 轮「唤出 + 收起」。
 *
 * 流程：warmup 一轮（冷启动，不计入统计）→ 清空采样 → N 轮测量。
 * 任何一步抛错都会被捕获并作为 `error` 返回，已完成的轮数照实上报——
 * 半程失败的数据比「全有或全无」更有诊断价值。
 */
export async function drivePeekCycles(
  opts: PerfDriveOptions,
): Promise<PerfDriveResult> {
  // 探针未启用时驱动器不可达：生产构建里既不注册监听器（见 App.svelte），
  // 这里再挡一道，避免有人绕过入口直接调用。
  if (!PERF_PROBE_ENABLED) {
    return {
      rounds: 0,
      blocked: 0,
      error: "探针未启用（PERF_PROBE_ENABLED=false），驱动器不可用",
    };
  }
  const sleep = opts.sleep ?? realSleep;
  const readyTimeoutMs = opts.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS;
  let rounds = 0;
  let blocked = 0;

  /** 等唤出守卫成立（最多 readyTimeoutMs）。用轮询次数而非墙钟，便于注入假时钟。 */
  const waitReady = async (): Promise<boolean> => {
    const maxPolls = Math.max(1, Math.ceil(readyTimeoutMs / READY_POLL_MS));
    for (let i = 0; i < maxPolls; i++) {
      if (opts.hooks.ready()) return true;
      await sleep(READY_POLL_MS);
    }
    return opts.hooks.ready();
  };

  /** 一轮 = 唤出 + 收起。 */
  const runCycle = async (): Promise<boolean> => {
    // 唤出（守卫在 hooks.reveal 内部，与 onPeekHover 回调体同源）。
    const revealed = await opts.hooks.reveal();
    // 真实「指针离开」链路有 320ms 宽限：离开后胶囊**仍保持唤出态**，
    // 宽限到期才收起。必须等满这段，否则测的是「唤出后立刻收起」的时序。
    await sleep(opts.dockDelayMs);
    // 收起：等价 scheduleDockPill 的定时器到期效果。
    await opts.hooks.dock();
    // 等 dockPill 的收起时间线走完（+120ms 的 peekLead、+180ms 的 shrink）。
    // 不等的话下一轮的守卫 `!pillCollapsing` 必然不成立——dockPill 期间
    // pillCollapsing=true，180ms 后才被清掉——那一轮会白跑。
    await sleep(opts.slideMs);
    return revealed;
  };

  try {
    // ── warmup ────────────────────────────────────────────────────────
    // 冷启动（WebView2 首次建 peek 窗口 + 运行时初始化）落在这一轮，
    // **不计入统计**：否则第一轮会系统性偏慢，抬高 max 与 p95。
    if (!(await waitReady())) {
      return { rounds: 0, blocked: 0, error: READY_TIMEOUT_MSG };
    }
    await runCycle();
    // warmup 之后清空采样：冷启动的 span 与帧样本不进统计。
    opts.resetStats?.();

    // ── 测量轮 ────────────────────────────────────────────────────────
    for (let i = 0; i < opts.rounds; i++) {
      if (!(await waitReady())) {
        return { rounds, blocked, error: READY_TIMEOUT_MSG };
      }
      const revealed = await runCycle();
      rounds++;
      if (!revealed) blocked++;
      if (i < opts.rounds - 1) await sleep(opts.intervalMs);
    }
  } catch (err) {
    return {
      rounds,
      blocked,
      error: err instanceof Error ? err.message : String(err),
    };
  }
  return { rounds, blocked };
}
