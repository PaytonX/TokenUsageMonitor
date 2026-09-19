<script module lang="ts">
  let nextSeq = 0;
</script>

<script lang="ts">
  interface Props {
    /** Remaining percentage, 0..1 (ring fills by remaining quota) */
    value: number;
    size?: number;
    stroke?: number;
    label?: string;
    /** No data yet: neutral empty track, no progress arc, no crit glow. */
    idle?: boolean;
  }

  let { value, size = 56, stroke = 4, label, idle = false }: Props = $props();

  let gradientSeq = $state(nextSeq++);
  let gradientId = `tum-ring-grad-${gradientSeq}`;

  let clamped = $derived(Math.min(1, Math.max(0, value)));
  let radius = $derived((size - stroke) / 2);
  let circumference = $derived(2 * Math.PI * radius);
  let offset = $derived(circumference * (1 - clamped));

  // value = REMAINING; thresholds align with spec: used >= 95% crit, >= 80% warn
  let tone = $derived(idle ? "ok" : 1 - clamped >= 0.95 ? "crit" : 1 - clamped >= 0.8 ? "warn" : "ok");
  let isCrit = $derived(!idle && tone === "crit");

  let fontSize = $derived(Math.max(9, Math.round(size * 0.34)));
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
        <stop offset="0%" stop-color="#4cc2ff" />
        <stop offset="100%" stop-color="#6ccb5f" />
      {:else if tone === "warn"}
        <stop offset="0%" stop-color="#e8b53d" />
        <stop offset="100%" stop-color="#ffc83d" />
      {:else}
        <stop offset="0%" stop-color="#e05248" />
        <stop offset="100%" stop-color="#ff5f56" />
      {/if}
    </linearGradient>
  </defs>
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
