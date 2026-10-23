<!--
  Stat — 数字 + 弱化标签
  ───────────────────────────────────────────────────────────────────
  替代散布在 TrendPanel / ModelPanel / ToolPanel / DevicePanel /
  HeatmapGrid / TrendWindow / ToolWindow 里的
  `__stat / __stat b / __stat span` 三件套。

  设计语言：value 走主文字 + 等宽 + 表格数字；label 走 muted + 10px。

  用法：
    <Stat value="29" label="连续活跃" suffix="天" />
    <Stat animate={total} format={fmtTokens} label="累计 tokens" />
    <StatRow>
      <Stat value={fmtTokens(total)} label="累计 tokens" />
      <Stat value={fmtTokens(active.total)} label="本月消耗" />
    </StatRow>
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { untrack } from "svelte";
  import { createTween } from "../../motion/num";

  interface Props {
    /** 非 animate 模式的渲染值；animate 模式下被忽略，可省略。 */
    value?: string | number;
    label?: string;
    /** value 后挂单位（"天" / "%" 等），等宽渲染避免抖动。 */
    suffix?: string;
    /** 提供时对该数字做非对称补间（上升瞬时/下降限速），value 被忽略。 */
    animate?: number;
    /** animate 模式下的格式化函数；缺省四舍五入为整数。 */
    format?: (n: number) => string;
    /** 下降线性限速（单位/秒），按数值标度配置（如 tokens 面板传 25_000）。 */
    fallLimitPerSec?: number;
    /** 容器用，把多个 Stat 横排成一行。 */
    children?: Snippet;
  }

  let { value, label, suffix, animate, format, fallLimitPerSec, children }: Props = $props();

  // 系统减弱动态效果：$state + change 监听（App.svelte 先例模式），
  // 运行时切换系统设置立即生效；SSR 守卫（typeof matchMedia）保持。
  let reducedMotion = $state(false);
  $effect(() => {
    if (typeof matchMedia === "undefined") return;
    const motionMq = matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => (reducedMotion = motionMq.matches);
    sync();
    motionMq.addEventListener("change", sync);
    return () => motionMq.removeEventListener("change", sync);
  });

  // 初始值只取挂载时的 animate（此后 animate 变化一律由下方 $effect 驱动补间）。
  // untrack 读与直接读语义相同，仅显式声明不追踪，避免 state_referenced_locally 警告。
  const initial = untrack(() => animate ?? 0);
  let shown = $state(initial);
  const tween = createTween(initial, {
    fallLimitPerSec: untrack(() => fallLimitPerSec ?? 25),
  });

  $effect(() => {
    if (animate === undefined) return;
    if (reducedMotion) {
      shown = animate;
      return;
    }
    let raf = requestAnimationFrame(function frame(now) {
      const v = tween.step(animate, now);
      shown = v;
      if (v !== animate) raf = requestAnimationFrame(frame);
    });
    return () => cancelAnimationFrame(raf);
  });
</script>

<span class="stat">
  <b class="stat__value"
    >{animate !== undefined
      ? format
        ? format(shown)
        : String(Math.round(shown))
      : value}{#if suffix}<span class="stat__suffix">{suffix}</span>{/if}</b
  >
  {#if label}<span class="stat__label">{label}</span>{/if}
  {#if children}{@render children()}{/if}
</span>

<style>
  .stat {
    display: inline-flex;
    align-items: baseline;
    gap: var(--tum-space-2);
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
  }

  .stat__value {
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
    font-weight: 600;
  }

  .stat__suffix {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
    font-weight: 400;
    margin-left: 2px;
  }

  .stat__label {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
  }
</style>