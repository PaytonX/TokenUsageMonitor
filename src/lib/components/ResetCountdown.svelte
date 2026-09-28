<script lang="ts">
  interface Props {
    resetAt?: string; // ISO8601 UTC
    label?: string;
  }

  let { resetAt, label = "重置" }: Props = $props();

  let now = $state(Date.now());

  // Tick every second for the countdown text.
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });

  let remainingMs = $derived(
    resetAt ? new Date(resetAt).getTime() - now : 0,
  );

  let countdown = $derived.by(() => {
    if (remainingMs <= 0) return "已重置";
    const totalSec = Math.floor(remainingMs / 1000);
    const d = Math.floor(totalSec / 86400);
    const h = Math.floor((totalSec % 86400) / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    // Weekly and monthly windows run to hundreds of hours, which reads far
    // better as days: "29d 23h 59m" instead of "719h 59m". Under 24h the
    // hour/minute/second tiers are kept — a leading "0d" would only add noise.
    if (d > 0) return `${d}d ${h}h ${m}m`;
    if (h > 0) return `${h}h ${m}m`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  });
</script>

<div class="countdown" title={resetAt}>
  <span class="countdown__label">{label}</span>
  <span class="countdown__value">{countdown}</span>
</div>

<style>
  .countdown {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .countdown__label {
    text-transform: uppercase;
    letter-spacing: 0.8px;
    color: var(--tum-text-muted);
  }

  .countdown__value {
    color: var(--tum-text-secondary);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.4px;
  }
</style>
