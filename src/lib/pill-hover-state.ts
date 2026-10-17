/**
 * 胶囊 hover 按钮条的显隐状态机（方案 .scratch/glance-bench/issues/03）。
 *
 * 为什么要抽成纯函数：03 是整个方案里最容易引入回归的一项。原先按钮显隐靠
 * CSS `.pill:hover { opacity: 1 }`——**触摸设备上它永不触发**，关闭按钮永久
 * 不可达（功能缺失，不是观感问题）。改成 JS 判定后这段逻辑必须可测。
 *
 * 这里借 lulu-loopp/glance `src/detector.rs` 的可移植部分——**不是**它的推力
 * 模型（它是推到边才呼出、我们是常驻贴边 + 悬停滑入），而是「把超时/回拉
 * 归零写成一个带单测的状态机」这个纪律。
 *
 * ## zone 为什么不在事件里
 * 早先事件带 `zone: "pill" | "strip" | "gap"`，但 reduceHover 的 enter 分支
 * 从不读它，且当时布局里关闭按钮是 `.pill` 的 absolute 子元素（指针移到它
 * 身上仍在 `.pill` 的 border-box 内），`gap`/`strip` 不存在真正的事件派发点。
 * 留着不被读取的字段会让人误以为「跨按钮的连续 hover 已被覆盖」。故删掉。
 * 将来真的做多按钮横条、按钮与胶囊之间出现缝隙时，再把 zone 加回来并让它
 * 参与判定——那时候它才有意义。
 */

/** 指针离开后的宽限窗（ms）。期间再进入即取消收起。 */
export const HOVER_GRACE_MS = 140;

export type HoverEvent =
  | { type: "enter"; at: number }
  | { type: "leave"; at: number }
  /**
   * 拖拽冻结状态变化。
   * `pointerInside` 是**解冻那一刻**指针是否在胶囊上——冻结期丢弃的 enter
   * 要靠它补回，否则「拖完就停在胶囊上准备关闭」这个最自然的序列会失灵。
   */
  | { type: "drag"; active: boolean; pointerInside: boolean; at: number }
  /** 胶囊是否处于可交互态（展开且未贴边收起） */
  | { type: "structure"; interactive: boolean; at: number }
  /** 推进时钟（供纯函数处理宽限期到期） */
  | { type: "tick"; at: number };

export interface HoverState {
  /** 按钮条当前是否可见。 */
  visible: boolean;
  /** 宽限截止时刻；null 表示没有在途的收起。 */
  hideAt: number | null;
  /** 拖拽冻结中。冻结期间不消费宽限期、也不响应指针进出。 */
  frozen: boolean;
  /** 胶囊当前是否可交互。不交互时按钮条一律不显示。 */
  interactive: boolean;
}

export function initialHoverState(): HoverState {
  return { visible: false, hideAt: null, frozen: false, interactive: false };
}

/**
 * 推进一串事件，返回最终状态。不修改 `start`。
 *
 * 规则（每条都对应一个真实场景）：
 * - enter：立刻显示、取消在途收起。
 * - leave：起一个 HOVER_GRACE_MS 的宽限计时，期间再进入即取消。
 * - drag active：只置冻结标志，**不清除 hideAt**。清除会让在途计时器被
 *   取消且不再重挂，按钮卡在可见态直到用户再次划过胶囊（已踩过的坑）。
 * - drag inactive：对账——指针此刻仍在胶囊上就补回可见性（enter 在冻结期
 *   被丢弃了），否则保持原有 hideAt 继续走完剩余宽限。
 * - structure false：立即隐藏并清掉计时。
 * - tick：冻结期间**不**消费宽限期（否则解冻就失去了余量）；否则到点收起。
 */
export function reduceHover(events: HoverEvent[], start: HoverState): HoverState {
  let s = start;
  for (const e of events) {
    switch (e.type) {
      case "enter": {
        if (s.frozen) break;
        s = { ...s, visible: s.interactive, hideAt: null };
        break;
      }
      case "leave": {
        if (s.frozen) break;
        if (!s.interactive) break;
        s = { ...s, hideAt: e.at + HOVER_GRACE_MS };
        break;
      }
      case "drag": {
        if (s.frozen === e.active) break;
        s = { ...s, frozen: e.active };
        if (!e.active && e.pointerInside && s.interactive) {
          // 冻结期进来的 enter 被丢弃了，这里补回。
          s = { ...s, visible: true, hideAt: null };
        }
        break;
      }
      case "structure": {
        if (s.interactive === e.interactive) break;
        s = e.interactive
          ? { ...s, interactive: true }
          : { ...s, interactive: false, visible: false, hideAt: null };
        break;
      }
      case "tick": {
        // 冻结期间不消费宽限：解冻后还剩多少余量就还剩多少。
        if (s.frozen) break;
        if (s.hideAt !== null && e.at >= s.hideAt) {
          s = { ...s, visible: false, hideAt: null };
        }
        break;
      }
    }
  }
  return s;
}

/** 便利封装：从初态跑完整串事件。 */
export function runHover(events: HoverEvent[]): HoverState {
  return reduceHover(events, initialHoverState());
}
