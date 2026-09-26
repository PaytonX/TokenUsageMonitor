<script lang="ts">
  interface Props {
    active?: boolean;
    tone?: "ok" | "warn" | "crit";
    size?: number;
  }

  let { active = false, tone = "ok", size = 8 }: Props = $props();
</script>

<span
  class="dot"
  class:dot--on={active}
  data-tone={tone}
  style={`width:${size}px;height:${size}px`}
  aria-hidden="true"
>
  {#if active}
    <span class="dot__wave" style={`width:${size}px;height:${size}px`}></span>
  {/if}
</span>

<style>
  .dot {
    position: relative;
    display: inline-block;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.22);
    flex: none;
  }

  .dot--on {
    background: var(--tum-ok);
  }

  .dot--on[data-tone="warn"] {
    background: var(--tum-warn);
  }

  .dot--on[data-tone="crit"] {
    background: var(--tum-crit);
  }

  .dot__wave {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    pointer-events: none;
    animation: dot-pulse 1.6s ease-out infinite;
  }

  .dot--on .dot__wave {
    box-shadow: 0 0 0 0 rgba(108, 203, 95, 0.5);
  }

  .dot--on[data-tone="warn"] .dot__wave {
    box-shadow: 0 0 0 0 rgba(255, 200, 61, 0.5);
  }

  .dot--on[data-tone="crit"] .dot__wave {
    box-shadow: 0 0 0 0 rgba(255, 95, 86, 0.55);
  }

  /* 扩散只动画 opacity + transform（box-shadow 起始宽度由各 tone 类提供），
     避免动画 box-shadow 造成每帧重绘 */
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
