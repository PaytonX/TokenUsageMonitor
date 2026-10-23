import { describe, expect, it } from "vitest";
import { createTween } from "./num";

describe("createTween", () => {
  it("首步直接贴住目标（挂载不从 0 爬）", () => {
    const tw = createTween(0);
    expect(tw.step(100, 1000)).toBe(100);
  });

  it("上升按 τ=60ms 指数趋近", () => {
    const tw = createTween(0);
    tw.step(0, 0); // 首步贴 0
    const v1 = tw.step(100, 60); // dt=60 → alpha = 1 - e^(-1) ≈ 0.6321
    expect(v1).toBeCloseTo(100 * (1 - Math.exp(-1)), 2);
    const v2 = tw.step(100, 120); // 从 63.21 继续
    expect(v2).toBeCloseTo(100 - 100 * Math.exp(-2), 1);
  });

  it("上升末段吸附到目标（无小数长尾）", () => {
    const tw = createTween(0);
    tw.step(0, 0);
    let v = 0;
    for (let i = 1; i <= 600; i++) v = tw.step(100, i * 1);
    expect(v).toBe(100);
  });

  it("下降被线性限速（限速=下限，差值/2s 兜底提速）", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    // 翻转帧冻结速率：max(40, (100-0)/2) = 50/s（限速是下限，不是上限）
    const v = tw.step(0, 60); // dt=60ms → 100 - 50*0.06 = 97
    expect(v).toBeCloseTo(97, 6);
  });

  it("下降最终收敛到目标且不越过", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    let v = 100;
    let now = 0;
    while (v !== 0 && now < 60_000) {
      now += 16;
      v = tw.step(0, now);
    }
    expect(v).toBe(0);
  });

  it("数值来回摆动：升降各自遵循自己的曲线", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.step(100, 60); // 上升：指数 → 100*(1-e^-1) ≈ 63.21
    const down = tw.step(0, 120); // 下降：翻转帧冻结 max(40, 63.21/2) = 40 → 40*0.06=2.4
    expect(down).toBeCloseTo(100 * (1 - Math.exp(-1)) - 2.4, 6);
  });

  it("大标度下降在收敛上限内精确到达（12.5e6 → 0 ≤ 2s）", () => {
    const tw = createTween(0, { fallLimitPerSec: 25 });
    tw.step(0, 0);
    tw.reset(12.5e6);
    // 翻转帧冻结速率 = max(25, 12.5e6/2) = 6.25e6/s → 恰 2s 收敛，rAF 链可停帧
    let v = 12.5e6;
    let now = 0;
    while (v !== 0 && now < 2400) {
      now += 16;
      v = tw.step(0, now);
    }
    expect(v).toBe(0);
    expect(now).toBeLessThanOrEqual(2400);
  });

  it("同毫秒重复 step 原地保持（不直贴不砸底）", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    const first = tw.step(50, 10); // 正常下降一步：翻转帧冻结 max(40, 25) = 40/s → 降 0.4
    expect(first).toBeCloseTo(99.6, 6);
    const again = tw.step(30, 10); // 同 now：dt=0 → 按冻结速率前进 0ms = 原地保持
    expect(again).toBe(first); // 不前进
    expect(again).not.toBe(30); // 不 snap 到 target
    expect(tw.value).toBe(first);
  });

  it("NaN/Infinity 输入被守卫（返回值不变、保持有限）", () => {
    const tw = createTween(0, { fallLimitPerSec: 40 });
    tw.step(0, 0);
    tw.reset(100);
    expect(tw.step(NaN, 1000)).toBe(100);
    expect(Number.isFinite(tw.value)).toBe(true);
    expect(tw.step(100, NaN)).toBe(100); // now 非法同样无害：时钟不动
    expect(Number.isFinite(tw.value)).toBe(true);
    expect(tw.step(Infinity, 1000)).toBe(100);
    expect(Number.isFinite(tw.value)).toBe(true);
  });

  it("下降中 target 变深则重冻结速率，收敛仍 ≤ 2s 级", () => {
    const tw = createTween(0, { fallLimitPerSec: 25 });
    tw.step(0, 0);
    tw.reset(100);
    // 先朝 0 下降一小段（翻转帧冻结 50/s → 每帧 0.8）
    let v = 100;
    let now = 0;
    for (let i = 0; i < 10; i++) {
      now += 16;
      v = tw.step(0, now);
    }
    expect(v).toBeLessThan(100); // 确认已在下降途中
    // 中途改道到 -100：重冻结 max(25, (v+100)/2) → 从改道时刻起 2s 上限内到达
    const changedAt = now;
    while (v !== -100 && now - changedAt < 2400) {
      now += 16;
      v = tw.step(-100, now);
    }
    expect(v).toBe(-100);
    expect(now - changedAt).toBeLessThanOrEqual(2400);
  });
});
