/**
 * 非对称数字补间 —— glance 频谱条思路的前端移植。
 *
 * 上升（数值变大）：指数趋近 τ=60ms（glance 滚动 τ），视觉上「立即到位」
 * 但保留一丝惯性感；下降（数值变小）：翻转帧冻结速率后线性限速（下限
 * fallLimitPerSec，大差值按 gap/2s 兜底），任何下降 ≤2s 精确收敛——
 * 防止用量清零时数字「砸」下来，也防止百万级计数按 tokens/s 爬行数天。
 *
 * 纯函数 + 显式时钟：rAF 只负责驱动，vitest 注入假时钟直测。
 * 首步（updatedAt 为 null）直接贴住目标——挂载时显示真实值，不从 0 爬。
 */

export interface TweenConfig {
  /** 上升指数趋近时间常数（ms）。glance：60。 */
  riseTauMs?: number;
  /** 下降线性限速（单位/秒），是速率下限：翻转帧按 gap/FALL_MAX_SECONDS 兜底提速。 */
  fallLimitPerSec?: number;
}

const DEFAULTS: Required<TweenConfig> = {
  riseTauMs: 60,
  fallLimitPerSec: 40,
};

/** 下降收敛上限（秒）——限速是「看起来在动」，不是「爬回起点」。 */
const FALL_MAX_SECONDS = 2;

export function createTween(initial = 0, config: TweenConfig = {}) {
  const cfg = { ...DEFAULTS, ...config };
  let value = initial;
  let updatedAt: number | null = null;
  // 下降方向状态：速率只在两个离散时机冻结（翻转帧 / 下降中 target 又变了），
  // 之后整段下降按冻结值线性钳制——绝不逐帧重算（逐帧重算=渐近线，永不收敛）。
  let falling = false;
  let fallRate = cfg.fallLimitPerSec;
  let fallTarget = NaN; // 本次冻结所针对的 target；NaN 哨兵，非法输入已被守卫挡下

  /** 推进一步。now：毫秒时间戳；target：目标值。返回新的显示值。 */
  function step(target: number, now: number): number {
    // NaN/Infinity 守卫：非法输入无害化——保持原值，时钟与方向状态都不动
    if (!Number.isFinite(target) || !Number.isFinite(now)) return value;
    if (updatedAt === null) {
      // 首步：直接贴住目标——挂载时显示真实值，不从 0 爬
      updatedAt = now;
      value = target;
      return value;
    }
    const dt = Math.max(0, now - updatedAt);
    updatedAt = now;
    if (dt === 0) {
      // 同毫秒（或时钟回拨）：无时间流逝，原地保持——不直贴目标、不限速绕过
      return value;
    }
    if (target >= value) {
      falling = false;
      const alpha = 1 - Math.exp(-dt / cfg.riseTauMs);
      const next = value + (target - value) * alpha;
      // 末段吸附：差值 < 0.01 视为到位，避免无限小数长尾
      value = target - next < 0.01 ? target : next;
    } else {
      if (!falling || target !== fallTarget) {
        fallRate = Math.max(cfg.fallLimitPerSec, (value - target) / FALL_MAX_SECONDS);
        fallTarget = target;
        falling = true;
      }
      value = Math.max(target, value - (fallRate * dt) / 1000);
    }
    return value;
  }

  return {
    step,
    /** 当前显示值。 */
    get value() {
      return value;
    },
    /** 硬置显示值（时钟不动，下一步正常补间）。 */
    reset(v: number) {
      value = v;
    },
  };
}
