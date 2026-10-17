/**
 * perf 的类型定义。与实现分文件，避免运行时模块携带类型断言产生的循环引用。
 */

/** 单条通道的统计。 */
export interface PerfSpan {
  /** 升序排列的样本（ms）。 */
  samples: number[];
  /** 有效样本数。上限为 PERF_CAPACITY。 */
  n: number;
  p50: number;
  /**
   * 最近秩分位数。小样本下会退化为 max —— 所以下面还有 `p95Meaningful`，
   * 别在 `n < PERF_MIN_N_FOR_P95` 时拿它当结论。
   */
  p95: number;
  max: number;
  /** 达到或超过慢阈值的样本数。 */
  slow: number;
  /** 失败的调用数（仅 ipc 通道有意义）。 */
  fails: number;
  /** 因同帧合并而被丢弃的测量数。门槛判定要求为 0。 */
  dropped: number;
  /** 因窗口不可见或超上限而未计入的测量数。 */
  unreliable: number;
  /** 样本数是否足以让 p95 有意义。 */
  p95Meaningful: boolean;
}

export interface PerfSnapshot {
  ipc: PerfSpan;
  frame: PerfSpan;
  longFrames: PerfSpan;
  /** 当前环境是否真的在采集帧成本。false 表示没有这条数据，别拿别的通道冒充。 */
  longFramesSupported: boolean;
}

/** 快照的别名，便于按需导入。 */
export type PerfSources = PerfSnapshot;
