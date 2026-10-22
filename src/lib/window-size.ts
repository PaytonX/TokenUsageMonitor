// 窗口尺寸的**裁决规则**与尺寸算式。
//
// 抽成纯函数是为了能单测——「一笔尺寸写只在自己被请求时的模式仍然成立时
// 才落地」是竞态修复的核心不变量，不能只靠「读代码觉得对」。
//
// ## 背景：尺寸写为什么会乱序
//
// 胶囊态的尺寸由**前端**写（展开/收起明细），全窗态的尺寸由 **Rust** 写
// （`set_window_mode` 切模式）。两边的 `setSize` 各走各的 IPC，彼此不排队，
// 于是可能**乱序落地**：
//
// 1. 用户在胶囊态悬停把手 → 胶囊滑出 → 展开计时器触发 → 前端写「168×150」；
// 2. 用户点胶囊上的圆环切全窗态 → Rust 写「400×680」；
// 3. 第 1 步那笔「168×150」**晚到一步**落地 → 全窗态窗口被改回 168 宽。
//
// 结果就是 dashboard 内容被塞进胶囊宽度的窗口：标签栏的中文「一个字一行」
// 竖排成一列、标题被裁掉、下拉框出界（实测截图里的窗口正是 168×150
// ——胶囊展开态的尺寸）。
//
// 修法就是本文件这条规则：请求模式与当前模式不一致时，那笔尺寸属于**旧模式**，
// 落地必然是错的，一律跳过；模式切换收尾时再补写一笔当前模式应有的尺寸。

/** 窗口的两种形态。 */
export type WindowMode = "dashboard" | "compact";

/** 逻辑像素尺寸。 */
export interface WindowSize {
  width: number;
  height: number;
}

/**
 * 全窗态（dashboard）尺寸。
 *
 * 与 Rust `set_window_mode` 的 `LogicalSize::new(400u32, 680u32)` 保持一致。
 * 两边都要写尺寸，差一个数就会出现「内容按 A 尺寸排、窗口按 B 尺寸」这类错位。
 */
export const DASHBOARD_SIZE: WindowSize = { width: 400, height: 680 };

/**
 * 胶囊收起态尺寸。
 *
 * 与 Rust `set_window_mode` 的 `LogicalSize::new(168u32, 56u32)` 保持一致。
 */
export const PILL_COLLAPSED_SIZE: WindowSize = { width: 168, height: 56 };

/** 明细行高（px）。 */
export const PILL_MINI_ROW = 24;

/** 明细行间距（px）。 */
export const PILL_MINI_GAP = 6;

/** 明细区上下内边距（px）。 */
const PILL_MINI_PAD_TOP = 2;
const PILL_MINI_PAD_BOTTOM = 8;

/**
 * 胶囊展开态（明细可见）时的窗口高度（px）。
 *
 * 算式：56 + (2 + 30n + 6(n-1) + 8) = 60 + 30n，n = 明细行数。
 * 与 `tokens.css` 里 `.pill__mini` 的排版一致。
 */
export function pillExpandedHeight(rows: number): number {
  return (
    PILL_COLLAPSED_SIZE.height +
    PILL_MINI_PAD_TOP +
    PILL_MINI_PAD_BOTTOM -
    PILL_MINI_GAP +
    rows * (PILL_MINI_ROW + PILL_MINI_GAP)
  );
}

/**
 * 一笔尺寸写**要不要落地**。
 *
 * 规则：一笔写只在自己被请求时的模式**仍然成立**时才落地。
 *
 * @param requested 请求这笔写时的模式。
 * @param current 真正要落地时的当前模式。
 */
export function shouldApplySizeWrite(
  requested: WindowMode,
  current: WindowMode,
): boolean {
  return requested === current;
}

/** 尺寸写的执行器：真正把窗口改成这个尺寸。 */
export type SizeApplier = (size: WindowSize) => Promise<void>;

/** 尺寸写队列。 */
export interface SizeWriteQueue {
  /**
   * 排一笔尺寸写。
   *
   * 落地前会先问 `shouldApply`：返回 false 就**跳过**（这笔尺寸已过期）。
   * 返回的 Promise 在「落地或跳过」之后 resolve。
   */
  enqueue(forMode: WindowMode, size: WindowSize): Promise<void>;
  /** 等待已排队的写全部落地（模式切换时先排空，再让 Rust 改尺寸）。 */
  drain(): Promise<void>;
}

/**
 * 尺寸写队列：把「写窗口尺寸」的所有调用串成一条链，并在**落地前**裁决
 * 这笔写还有效没有。
 *
 * ## 为什么需要它
 *
 * 胶囊态的尺寸由**前端**写（展开/收起明细），全窗态的尺寸由 **Rust** 写
 * （`set_window_mode` 切模式）。两边的 `setSize` 各走各的 IPC，彼此不排队，
 * 于是可能**乱序落地**：一笔慢的胶囊 resize 晚于模式切换落地，把 400×680
 * 的全窗态窗口又改回 168 宽 —— dashboard 内容于是被塞进胶囊宽度的窗口
 * （标签栏中文竖排成一列、标题被裁、下拉框出界）。
 *
 * 队列保证两件事：
 *
 * 1. **串行**：不会有两个 `setSize` 同时在飞，落地顺序 = 排队顺序；
 * 2. **落地前裁决**：一笔写只在「请求时的模式仍然成立」时才落地。模式
 *    一变，在途的旧写自动作废。
 *
 * ## 裁决由调用方给
 *
 * `shouldApply` 是回调而不是写死在这里，因为「这笔写还有效没有」取决于
 * 调用方的状态（当前模式 + 是否正在切模式）。本模块只管**排队与时序**，
 * 判据留给调用方——但那条判据本身（`shouldApplySizeWrite`）也在本文件里，
 * 且有单测钉住。
 */
export function createSizeWriteQueue(opts: {
  /** 裁决：返回 false 则跳过这笔写。 */
  shouldApply: (forMode: WindowMode) => boolean;
  /** 真正落地。 */
  apply: SizeApplier;
}): SizeWriteQueue {
  let chain: Promise<void> = Promise.resolve();
  return {
    enqueue(forMode, size) {
      const run = chain.then(async () => {
        if (!opts.shouldApply(forMode)) return;
        await opts.apply(size);
      });
      // 队列不被单次失败「毒死」：失败在这里吞掉，下一项照常排队。
      chain = run.then(
        () => {},
        () => {},
      );
      return run;
    },
    drain() {
      return chain;
    },
  };
}
