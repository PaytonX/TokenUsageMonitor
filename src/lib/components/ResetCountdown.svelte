<script lang="ts">
  import { t } from "../i18n/store";

  interface Props {
    resetAt?: string; // ISO8601 UTC
    label?: string;
  }

  // label 默认值是 i18n 键（"countdown.reset"）；外部传入的非键字符串经 $t
  // 原样返回，故既有的字面量调用方不受影响。
  let { resetAt, label = "countdown.reset" }: Props = $props();

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
    if (remainingMs <= 0) return $t("countdown.resetDone");
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
    <span class="countdown__label">{$t(label)}</span>
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
