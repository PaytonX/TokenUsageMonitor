<script lang="ts">
  interface Props {
    /** Remaining percentage, 0..1 */
    value: number;
    size?: number;
    stroke?: number;
    label?: string;
  }

  let { value, size = 56, stroke = 4, label }: Props = $props();

  let clamped = $derived(Math.min(1, Math.max(0, value)));
  let radius = $derived((size - stroke) / 2);
  let circumference = $derived(2 * Math.PI * radius);
  let offset = $derived(circumference * (1 - clamped));
  let color = $derived(
    clamped > 0.3 ? "var(--tum-accent)" : clamped > 0.1 ? "var(--tum-warning)" : "var(--tum-danger)",
  );
  // Font-size scales with ring size so the label always fits inside the inner
  // diameter. Roughly 40% of the ring's diameter reads well for "100%"-style
  // labels; clamp the lower bound so tiny rings (e.g. compact=28) stay legible.
  let fontSize = $derived(Math.max(9, Math.round(size * 0.34)));
</script>

<svg
  width={size}
  height={size}
  viewBox={`0 0 ${size} ${size}`}
  class="ring"
>
  <circle
    cx={size / 2}
    cy={size / 2}
    r={radius}
    fill="none"
    stroke="rgba(148,163,184,0.14)"
    stroke-width={stroke}
  />
  <circle
    cx={size / 2}
    cy={size / 2}
    r={radius}
    fill="none"
    stroke={color}
    stroke-width={stroke}
    stroke-linecap="round"
    stroke-dasharray={circumference}
    stroke-dashoffset={offset}
    transform={`rotate(-90 ${size / 2} ${size / 2})`}
    style="transition: stroke-dashoffset 0.6s ease, stroke 0.4s ease"
  />
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

  .ring__label {
    font-family: var(--tum-font-mono);
    fill: var(--tum-text-primary);
    letter-spacing: 0.4px;
    font-variant-numeric: tabular-nums;
  }
</style>
