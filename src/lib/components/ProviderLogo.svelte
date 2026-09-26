<script lang="ts">
  interface Props {
    kind: string;
    size?: number;
    accent?: string | null;
  }

  let { kind, size = 18, accent = null }: Props = $props();

  const BRAND_COLORS: Record<string, string> = {
    minimax: "#e0488f",
    deepseek: "#4d6bfe",
    volcengine: "#1664ff",
    openai: "#10a37f",
    gemini: "#8e75f5",
    anthropic: "#d4a24f",
    qwen: "#615ced",
    kimi: "#16c2a3",
    doubao: "#3a6bff",
    spark: "#e63a3c",
    xiaomi: "#ff6900",
    xai: "#1d1d1d",
    codex: "#10a37f",
  };

  const INITIAL_OVERRIDES: Record<string, string> = {
    minimax: "M",
    deepseek: "D",
    volcengine: "V",
    openai: "O",
    gemini: "G",
    anthropic: "A",
    qwen: "Q",
    kimi: "K",
    doubao: "豆",
    spark: "S",
    xiaomi: "Mi",
    xai: "x",
    codex: "C",
    local: "本",
  };

  function normalizeKind(raw: string): string {
    if (raw === "kimi_global") return "kimi";
    return raw.split("_")[0];
  }

  const normalized = $derived(normalizeKind(kind));
  const brandColor = $derived(
    BRAND_COLORS[normalized] ?? accent ?? "#3b82f6",
  );
  const initial = $derived(
    INITIAL_OVERRIDES[normalized] ??
      (kind.trim() ? kind.trim().charAt(0).toUpperCase() : "?"),
  );
</script>

<span class="logo" style={`width:${size}px;height:${size}px`} aria-hidden="true">
  {#if normalized === "openai"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.92 6.042 6.042 0 0 0-6.51-2.783A5.975 5.975 0 0 0 4.4 4.938a5.985 5.985 0 0 0-3.999 2.93 5.985 5.985 0 0 0 .52 4.916 5.98 5.98 0 0 0 2.11 11.095 5.98 5.98 0 0 0 5.6 2.026A5.98 5.98 0 0 0 19.6 19.06a5.99 5.99 0 0 0 3.998-2.926 5.98 5.98 0 0 0-.516-4.92 5.98 5.98 0 0 0-.8-.793Z"
        stroke={brandColor}
        stroke-width="1.6"
        stroke-linejoin="round"
      />
      <path
        d="M12 15.063a3.064 3.064 0 1 0 0-6.127 3.064 3.064 0 0 0 0 6.127ZM13.806 8.586l3.76-2.17M15.063 12l3.76 2.17M10.194 15.414l-3.76 2.17M8.937 12l-3.76-2.17M10.194 8.586l-.78-4.3M13.806 15.414l.78 4.3"
        stroke={brandColor}
        stroke-width="1.6"
        stroke-linecap="round"
      />
    </svg>
  {:else if normalized === "anthropic"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill={brandColor}>
      <path d="M12 4.2c-.6 0-1.15.34-1.42.88L5.9 14.6l-.9 1.84h4.3l.86-1.8 1.3-2.72c.1-.2.3-.33.54-.33s.44.13.54.33l1.3 2.72.86 1.8h4.3l-.9-1.84-4.68-9.52A1.58 1.58 0 0 0 12 4.2Zm0 4.5 1.55 3.2h-3.1L12 8.7Z" />
    </svg>
  {:else if normalized === "gemini"}
    <svg viewBox="0 0 24 24" width={size} height={size}>
      <path
        d="M12 2.6c.45 3.55 1.85 6.2 4.5 8.05-2.65 1.85-4.05 4.5-4.5 8.05-.45-3.55-1.85-6.2-4.5-8.05C10.15 8.8 11.55 6.15 12 2.6Z"
        fill="#8e75f5"
      />
      <path
        d="M17.6 12.1c.25 2 .95 3.45 2.4 4.45-1.45 1-2.15 2.45-2.4 4.45-.25-2-.95-3.45-2.4-4.45 1.45-1 2.15-2.45 2.4-4.45Z"
        fill="#5bb0f5"
      />
    </svg>
  {:else if normalized === "xai"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="1.8" y="1.8" width="20.4" height="20.4" rx="5" fill="#1d1d1d" />
      <path
        d="m8.2 8.2 7.6 7.6M15.8 8.2l-7.6 7.6"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
      />
      <path
        d="M12 10.4c1.5-.5 2.6-1.2 3.4-2.1M12 13.6c-1.5.5-2.6 1.2-3.4 2.1"
        stroke="#fff"
        stroke-width="1.1"
        stroke-linecap="round"
        opacity=".65"
      />
    </svg>
  {:else if normalized === "kimi"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="2" y="2" width="20" height="20" rx="5" fill={brandColor} />
      <path
        d="M8.6 7.4v9.2M8.6 12l5.1-4.6M13.7 7.4v9.2M15.4 9.6v4.8"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else if normalized === "codex"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <rect x="2" y="2" width="20" height="20" rx="5" fill={brandColor} />
      <path
        d="m9.8 9.2-2.8 2.8 2.8 2.8M14.2 9.2l2.8 2.8-2.8 2.8"
        stroke="#fff"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else if normalized === "deepseek"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M4 13.2c2.6 0 3.9-1.3 3.9-3.5V7h2.1v2.9c0 3.4-2.1 5.5-5.6 5.5H4v-2.2Z"
        fill={brandColor}
      />
      <path
        d="M14.6 7h2.7l3.7 5.6V7H22v10h-.9l-3.8-5.8V17h-2.7V7Z"
        fill={brandColor}
      />
    </svg>
  {:else if normalized === "minimax"}
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none">
      <path
        d="M3.5 17V7l6 6 5.5-6v10M15 17l5.5-10"
        stroke={brandColor}
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else}
    <span
      class="logo__tile"
      style={`width:${size}px;height:${size}px;background:${brandColor};font-size:${Math.max(
        size * 0.52,
        9,
      )}px`}
    >
      {initial}
    </span>
  {/if}
</span>

<style>
  .logo {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    line-height: 0;
  }

  .logo :global(svg) {
    display: block;
  }

  .logo__tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 28%;
    color: #fff;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0;
    user-select: none;
  }
</style>
