<script lang="ts">
  interface Props {
    active?: boolean;
    tone?: "ok" | "warn" | "crit";
    size?: number;
    /** 可选品牌色：传入时覆盖 tone 的状态色（明细行用品牌色小圆点）。 */
    accent?: string | null;
  }

  let { active = false, tone = "ok", size = 8, accent = null }: Props = $props();

  // 状态色取自设计令牌；accent（品牌色）优先，用于明细行的品牌小圆点。
  const TONE_COLORS: Record<string, string> = {
    ok: "var(--tum-ok)",
    warn: "var(--tum-warn)",
    crit: "var(--tum-crit)",
  };

  let color = $derived(accent ?? TONE_COLORS[tone] ?? TONE_COLORS.ok);
</script>

<span
  class="dot"
  style={`width:${size}px;height:${size}px;background:${
    active ? color : "rgba(255,255,255,0.35)"
  }`}
  aria-hidden="true"
>
  {#if active}
    <span class="dot__wave" style={`width:${size}px;height:${size}px;background:${color}`}
    ></span>
  {/if}
</span>

<style>
  .dot {
    position: relative;
    display: inline-block;
    border-radius: 50%;
    flex: none;
    transition: background 0.3s ease;
  }

  /* 扩散只动画 opacity + transform，避免每帧重绘 */
  .dot__wave {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    pointer-events: none;
    animation: dot-pulse 1.6s ease-out infinite;
  }

  @keyframes dot-pulse {
    0% {
      opacity: 0.7;
      transform: scale(1);
    }
    70% {
      opacity: 0;
      transform: scale(2.4);
    }
    100% {
      opacity: 0;
      transform: scale(2.4);
    }
  }
</style>