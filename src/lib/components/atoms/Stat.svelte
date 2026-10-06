<!--
  Stat — 数字 + 弱化标签
  ───────────────────────────────────────────────────────────────────
  替代散布在 TrendPanel / ModelPanel / ToolPanel / DevicePanel /
  HeatmapGrid / TrendWindow / ToolWindow 里的
  `__stat / __stat b / __stat span` 三件套。

  设计语言：value 走主文字 + 等宽 + 表格数字；label 走 muted + 10px。

  用法：
    <Stat value="29" label="连续活跃" suffix="天" />
    <StatRow>
      <Stat value={fmtTokens(total)} label="累计 tokens" />
      <Stat value={fmtTokens(active.total)} label="本月消耗" />
    </StatRow>
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    value: string | number;
    label?: string;
    /** value 后挂单位（"天" / "%" 等），等宽渲染避免抖动。 */
    suffix?: string;
    /** 容器用，把多个 Stat 横排成一行。 */
    children?: Snippet;
  }

  let { value, label, suffix, children }: Props = $props();
</script>

<span class="stat">
  <b class="stat__value">{value}{#if suffix}<span class="stat__suffix">{suffix}</span>{/if}</b>
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