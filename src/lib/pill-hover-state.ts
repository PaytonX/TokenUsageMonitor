/**
 * 胶囊 hover 按钮条的显隐状态机（方案 .scratch/glance-bench/issues/03）。
 *
 * 为什么要抽成纯函数：03 是整个方案里最容易引入回归的一项。把「常驻关闭
 * 按钮」改成 hover 浮出，意味着要新增一段指针判定；而这段判定和现有的
 * 把手 hover/leave 链路是交错的（App.svelte:602-604 记了这个坑）。
 * 人工验收「从左划到右不消失」靠人眼，回归只能靠重放三轮。
 *
 * 这里照搬 lulu-loopp/glance `src/detector.rs` 的可移植部分——**不是**它的
 * 推力模型（我们是常驻贴边 + 悬停滑入，它是推到边才呼出，模型不同），
 * 而是把「超时 / 回拉归零」写成一个带单测的状态机这个纪律。
 */

/** 指针相对按钮条的位置。 */
export type HoverZone =
  /** 在胶囊主体（含把手）上 */
  | "pill"
  /** 在按钮条上 */
  | "strip"
  /** 在按钮条与胶囊之间的缝隙上——最容易漏掉的一格 */
  | "gap"
  /** 都不在 */
  | "outside";

export type HoverEvent =
  | { type: "enter"; zone: Exclude<HoverZone, "outside">; at: number }
  | { type: "leave"; at: number }
  /** 拖拽中：冻结一切收起判定 */
  | { type: "drag"; active: boolean; at: number }
  /** 胶囊收起/展开等结构性变化 */
  | { type: "structure"; visible: boolean; at: number }
  /** 推进时钟（供纯函数处理宽限期到期） */
  | { type: "tick"; at: number };

/** 指针离开后的宽限窗（ms）。期间再进入即取消收起。 */
export const HOVER_GRACE_MS = 140;

/** 状态机的完整状态。 */
export interface HoverState {
  /** 按钮条当前是否可见。 */
  visible: boolean;
  /** 宽限截止时刻；null 表示没有在途的收起。 */
  hideAt: number | null;
  /** 拖拽冻结中。冻结期间 leave 不启动宽限计时。 */
  frozen: boolean;
  /** 胶囊当前是否可交互（展开态）。收起态按钮条不显示。 */
  interactive: boolean;
}

export function initialHoverState(): HoverState {
  return { visible: false, hideAt: null, frozen: false, interactive: false };
}

/** 结构性可见性：胶囊不处于可交互态时，按钮条一律不显示。 */
function applyStructure(state: HoverState, interactive: boolean): HoverState {
  if (state.interactive === interactive) return state;
  if (!interactive) return { ...state, interactive, visible: false, hideAt: null };
  return { ...state, interactive };
}

/**
 * 推进一串事件，返回最终状态。
 *
 * 规则（每条都对应一个真实场景）：
 * - 进入 pill / strip / **gap**：立刻显示、取消在途收起。gap 必须算在内，
 *   否则指针在按钮与胶囊之间横穿时按钮条会闪。
 * - 离开：起一个 HOVER_GRACE_MS 的宽限计时，期间再进入即取消。
 * - tick：过了截止时刻才收起。
 * - drag active：冻结。不启动计时、不收起——拖到一半被收起判定拽回屏幕
 *   边缘是已知故障（App.svelte:616-617）。
 * - structure false：立即隐藏并清掉计时。
 */
export function reduceHover(events: HoverEvent[], start: HoverState): HoverState {
  let s = start;
  for (const e of events) {
    switch (e.type) {
      case "enter": {
        // 拖拽冻结期间不响应进入：否则拖动途中按钮条自己冒出来。
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
        // 冻结期间若有在途的收起计时，一并作废——恢复时宁可多显示一会儿，
        // 也不能在用户松手瞬间把按钮条抽走。
        if (e.active) s = { ...s, hideAt: null };
        break;
      }
      case "structure": {
        s = applyStructure(s, e.visible);
        break;
      }
      case "tick": {
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
