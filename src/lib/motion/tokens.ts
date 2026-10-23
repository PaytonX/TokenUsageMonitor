/**
 * JS 定时器 ← CSS 时长 token 的单点桥接。
 *
 * JS 侧的 setTimeout/rAF 节拍无法直接消费 CSS 变量；把「读 token + fallback」
 * 收在这一个函数里，token 改名/改值时 JS 自动跟上，不再出现双份魔法数。
 * SSR / 测试环境（无 window）以及 CSS 未加载时回退 fallback（fallback 值
 * 必须与 tokens.css 当前值一致，改动 token 时同步改这里）。
 */
export function durMs(name: string, fallback: number): number {
  if (typeof window === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  // 单位防呆：parseFloat 不看单位，"0.2s" 会被静默解析成 0.2——数值上恰好
  // 能骗过下面的 `ms > 0` 守卫，tokens.css 将来一旦出现秒单位，动画就会
  // 快 1000 倍且无任何报错。因此按后缀分派单位：以 "ms" 结尾按毫秒解析，
  // 以 "s" 结尾 ×1000，其余（无单位/垃圾/空串）一律按 NaN 落 fallback。
  let ms = Number.NaN;
  if (raw.endsWith("ms")) {
    ms = Number.parseFloat(raw);
  } else if (raw.endsWith("s")) {
    ms = Number.parseFloat(raw) * 1000;
  }
  return Number.isFinite(ms) && ms > 0 ? ms : fallback;
}
