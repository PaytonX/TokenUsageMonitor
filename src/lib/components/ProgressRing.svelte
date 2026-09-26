<script module lang="ts">
  let nextSeq = 0;
</script>

<script lang="ts">
  import { lightenHex } from "../types";

  interface Props {
    /** Percentage 0..1. The arc always draws `value`; whether it means "used"
     *  or "remaining" is declared by `countdown`, which the tone mapping uses. */
    value: number;
    size?: number;
    stroke?: number;
    label?: string;
    /** No data yet: neutral empty track, no progress arc, no crit glow. */
    idle?: boolean;
    /** Optional outer ring (dual-ring mode): inner = `value`, outer = this.
     *  Used to juxtapose a short window (inner) against a long window
     *  (outer). When absent, a single ring is drawn. */
    outerValue?: number;
    /** Gap between the inner and outer rings, in px. */
    outerGap?: number;
    /** true = `value` (and `outerValue`) are REMAINING (countdown display);
     *  false = they are USED. Only affects the ok/warn/crit tone (thresholds are
     *  on used %, so used stays 80/95 under both mappings). Defaults to true. */
    countdown?: boolean;
    /** Optional per-account accent (#RRGGBB). Tints the "ok" arc gradient;
     *  warn/crit keep their status colors for clarity. Missing → system blue. */
    accent?: string;
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
  }: Props = $props();

  let gradientSeq = $state(nextSeq++);
  let gradientId = `tum-ring-grad-${gradientSeq}`;

  let clamped = $derived(Math.min(1, Math.max(0, value)));
  let hasOuter = $derived(outerValue !== undefined);
  // Single ring uses the full radius; dual ring lays the inner ring inside
  // the outer one (outer = `value`? no: outer = `outerValue`).
  let outerRadius = $derived((size - stroke) / 2);
  let radius = $derived(hasOuter ? outerRadius - stroke - outerGap : outerRadius);
  let circumference = $derived(2 * Math.PI * radius);
  let offset = $derived(circumference * (1 - clamped));

  let outerClamped = $derived(Math.min(1, Math.max(0, outerValue ?? 0)));
  let outerCircumference = $derived(2 * Math.PI * outerRadius);
  let outerOffset = $derived(outerCircumference * (1 - outerClamped));

  // value = the displayed fraction; thresholds align with spec: used >= 95%
  // crit, >= 80% warn. Recover "used" from the value according to `countdown`.
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

  // "ok" arc gradient: per-account accent when supplied, else system blue.
  let okStart = $derived(accent ?? "#4cc2ff");
  let okEnd = $derived(accent ? lightenHex(accent, 0.35) : "#6ccb5f");

  // Dual ring leaves a smaller center hole — shrink the label to fit.
  let fontSize = $derived(
    Math.max(9, Math.round(size * 0.34) * (hasOuter ? 0.72 : 1)),
  );
</script>

<svg
  width={size}
  height={size}
  viewBox={`0 0 ${size} ${size}`}
  class="ring"
  class:ring--crit={isCrit}
>
  <defs>
    <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="100%">
      {#if tone === "ok"}
        <stop offset="0%" stop-color={okStart} />
        <stop offset="100%" stop-color={okEnd} />
      {:else if tone === "warn"}
        <stop offset="0%" stop-color="#e8b53d" />
        <stop offset="100%" stop-color="#ffc83d" />
      {:else}
        <stop offset="0%" stop-color="#e05248" />
        <stop offset="100%" stop-color="#ff5f56" />
      {/if}
    </linearGradient>
  </defs>

  {#if hasOuter}
    <circle
      cx={size / 2}
      cy={size / 2}
      r={outerRadius}
      fill="none"
      stroke="rgba(255,255,255,0.06)"
      stroke-width={stroke}
    />
    {#if !idle}
      <circle
        cx={size / 2}
        cy={size / 2}
        r={outerRadius}
        fill="none"
        stroke={`url(#${gradientId})`}
        stroke-width={stroke}
        stroke-linecap="round"
        stroke-dasharray={outerCircumference}
        stroke-dashoffset={outerOffset}
        opacity="0.55"
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
        style="transition: stroke-dashoffset 0.6s ease"
      />
    {/if}
  {/if}

  <circle
    cx={size / 2}
    cy={size / 2}
    r={radius}
    fill="none"
    stroke="rgba(255,255,255,0.10)"
    stroke-width={stroke}
  />
  {#if !idle}
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke={`url(#${gradientId})`}
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={circumference}
      stroke-dashoffset={offset}
      transform={`rotate(-90 ${size / 2} ${size / 2})`}
      style="transition: stroke-dashoffset 0.6s ease"
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

<style>
  .ring {
    color: var(--tum-text-primary);
    display: block;
  }

  .ring--crit {
    filter: drop-shadow(0 0 5px rgba(255, 95, 86, 0.55));
  }

  .ring__label {
    font-family: var(--tum-font-mono);
    fill: var(--tum-text-primary);
    letter-spacing: 0.4px;
    font-variant-numeric: tabular-nums;
  }
</style>
