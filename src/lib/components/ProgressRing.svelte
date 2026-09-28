<script lang="ts">
  import { hexToRgb } from "../types";
  import { brandColorFor } from "../brand-glyphs";
  import ProviderLogo from "./ProviderLogo.svelte";

  interface Props {
    /** Percentage 0..1. The arc always draws `value`; whether it means "used"
     *  or "remaining" is declared by `countdown`, which the tone mapping uses. */
    value: number;
    size?: number;
    stroke?: number;
    label?: string;
    /** No data yet: neutral empty track, no progress arc, no crit glow. */
    idle?: boolean;
    /** Optional outer ring (dual-ring mode): inner = `value`, outer = this. */
    outerValue?: number;
    /** Gap between the inner and outer rings, in px. */
    outerGap?: number;
    /** true = the values are REMAINING; false = USED. Only affects the tone. */
    countdown?: boolean;
    /** Optional per-account accent (#RRGGBB) — wins over the brand color. */
    accent?: string;
    /** 品牌 kind：传入时环心渲染该品牌 mark，并用品牌色作环色。 */
    markKind?: string | null;
    /** 运行态：环心光斑以 2600ms 呼吸（仅 markKind 存在时有意义）。 */
    running?: boolean;
  }

  let {
    value,
    size = 56,
    stroke = 4,
    label,
    idle = false,
    outerValue,
    outerGap = 2,
    countdown = true,
    accent,
    markKind = null,
    running = false,
  }: Props = $props();

  let clamped = $derived(Math.min(1, Math.max(0, value)));
  let hasOuter = $derived(outerValue !== undefined);
  let outerRadius = $derived((size - stroke) / 2);
  let radius = $derived(hasOuter ? outerRadius - stroke - outerGap : outerRadius);
  let circumference = $derived(2 * Math.PI * radius);
  let offset = $derived(circumference * (1 - clamped));

  let outerClamped = $derived(Math.min(1, Math.max(0, outerValue ?? 0)));
  let outerCircumference = $derived(2 * Math.PI * outerRadius);
  let outerOffset = $derived(outerCircumference * (1 - outerClamped));

  // Tone thresholds stay on used %: used >= 95% crit, >= 80% warn.
  let usedFraction = $derived(countdown ? 1 - clamped : clamped);
  let tone = $derived(
    idle
      ? "ok"
      : usedFraction >= 0.95
        ? "crit"
        : usedFraction >= 0.8
          ? "warn"
          : "ok",
  );
  let isCrit = $derived(!idle && tone === "crit");

  // 环色优先级：账户强调色（若传入）→ 品牌注册表品牌色 → 系统蓝。
  // 胶囊传入的 accent 本身已是「账户色 ?? 品牌色」，因此圆环与徽章永远同色。
  // warn/crit 仍切到状态色，保证危急状态一眼可辨。
  let brand = $derived(
    accent ?? (markKind ? brandColorFor(markKind) : "#4cc2ff"),
  );
  let ringColor = $derived(
    tone === "crit"
      ? "var(--tum-crit)"
      : tone === "warn"
        ? "var(--tum-warn)"
        : brand,
  );
  let trackStroke = $derived(
    `rgba(${hexToRgb(brand) ?? "76,194,255"}, 0.2)`,
  );

  // 挂载后下一帧再施加偏移：让「从空到当前值」也走 420ms 过渡。
  let ready = $state(false);
  $effect(() => {
    const id = requestAnimationFrame(() => (ready = true));
    return () => cancelAnimationFrame(id);
  });
  let shownOffset = $derived(ready ? offset : circumference);
  let shownOuterOffset = $derived(ready ? outerOffset : outerCircumference);

  let markSize = $derived(Math.round(size * 0.58));
  let fontSize = $derived(
    Math.max(9, Math.round(size * 0.34) * (hasOuter ? 0.72 : 1)),
  );
</script>

<span class="ring-wrap" style={`width:${size}px;height:${size}px`}>
  {#if markKind}
    <span
      class="ring__halo"
      class:ring__halo--on={running}
      style={`background: radial-gradient(closest-side, ${brand}, transparent 72%)`}
    ></span>
  {/if}
  <svg
    width={size}
    height={size}
    viewBox={`0 0 ${size} ${size}`}
    class="ring"
    class:ring--crit={isCrit}
  >
    {#if hasOuter}
      <circle
        cx={size / 2}
        cy={size / 2}
        r={outerRadius}
        fill="none"
        stroke={trackStroke}
        stroke-width={stroke}
      />
      {#if !idle}
        <circle
          cx={size / 2}
          cy={size / 2}
          r={outerRadius}
          fill="none"
          stroke={ringColor}
          stroke-width={stroke}
          stroke-linecap="round"
          stroke-dasharray={outerCircumference}
          stroke-dashoffset={shownOuterOffset}
          opacity="0.55"
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
          class="ring__arc"
        />
      {/if}
    {/if}

    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke={trackStroke}
      stroke-width={stroke}
    />
    {#if !idle}
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke={ringColor}
        stroke-width={stroke}
        stroke-linecap="round"
        stroke-dasharray={circumference}
        stroke-dashoffset={shownOffset}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
        class="ring__arc"
      />
    {/if}
    {#if label}
      <text
        x="50%"
        y="50%"
        dominant-baseline="central"
        text-anchor="middle"
        fill="currentColor"
        class="ring__label"
        style={`font-size: ${fontSize}px`}
      >{label}</text>
    {/if}
  </svg>
  {#if markKind}
    <span class="ring__mark" style={`width:${markSize}px;height:${markSize}px`}>
      <ProviderLogo kind={markKind} size={markSize} {accent} />
    </span>
  {/if}
</span>

<style>
  .ring-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }

  .ring {
    color: var(--tum-text-primary);
    display: block;
    position: relative;
    z-index: 1;
  }

  /* 420ms 圆环推进曲线（spec 决策 7） */
  .ring__arc {
    transition: stroke-dashoffset 420ms var(--tum-ease-ring);
  }

  .ring--crit {
    filter: drop-shadow(0 0 5px rgba(255, 95, 86, 0.55));
  }

  .ring__mark {
    position: absolute;
    inset: 0;
    margin: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    z-index: 2;
  }

  /* 环心光斑尺寸/呼吸（品牌色径向渐变由内联样式注入） */
  .ring__halo {
    position: absolute;
    inset: 0;
    margin: auto;
    width: 62%;
    height: 62%;
    border-radius: 50%;
    opacity: 0;
    z-index: 0;
  }

  .ring__halo--on {
    animation: ring-breathe 2600ms ease-in-out infinite;
  }

  @keyframes ring-breathe {
    0%,
    100% {
      opacity: 0.24;
      transform: scale(0.93);
    }
    50% {
      opacity: 0.66;
      transform: scale(1.06);
    }
  }

  .ring__label {
    font-family: var(--tum-font-mono);
    fill: var(--tum-text-primary);
    letter-spacing: 0.4px;
    font-variant-numeric: tabular-nums;
  }
</style>
