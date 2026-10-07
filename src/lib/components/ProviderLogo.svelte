<script lang="ts">
  import {
    FALLBACK_BRAND_COLOR,
    glyphFor,
    maskUrl,
    normalizeKind,
  } from "../brand-glyphs";
  import { t } from "../i18n/store";

  interface Props {
    kind: string;
    size?: number;
    accent?: string | null;
  }

  let { kind, size = 18, accent = null }: Props = $props();

  // 没有真实矢量的已知品牌仍用首字母 tile 回退（拉丁字母在各语言下一致）。
  const INITIAL_OVERRIDES: Record<string, string> = {
    minimax: "M",
    deepseek: "D",
    volcengine: "V",
    openai: "O",
    gemini: "G",
    anthropic: "A",
    qwen: "Q",
    kimi: "K",
    spark: "S",
    xiaomi: "Mi",
    xai: "x",
    codex: "C",
  };

  // 字形随 UI 语言本地化的品牌：中文显示汉字 logo，英文显示拉丁字母（经 i18n 词典取词）。
  const LOCALIZED_INITIAL_KEYS: Record<string, string> = {
    doubao: "logo.glyphDoubao",
    local: "logo.glyphLocal",
  };

  const glyph = $derived(glyphFor(kind));
  // 着色优先级：账户自定义强调色（用户在设置里选的颜色）优先，其次注册表品牌色，
  // 最后兜底色。Kimi 那种 tile 分支是多色成品（黑底 + 蓝点 + 白 K），无法被单色
  // 强调色套色，仍按自身颜色渲染。
  const brandColor = $derived(accent ?? glyph?.color ?? FALLBACK_BRAND_COLOR);
  const localizedInitialKey = $derived(LOCALIZED_INITIAL_KEYS[normalizeKind(kind)]);
  const initial = $derived(
    (localizedInitialKey ? $t(localizedInitialKey) : undefined) ??
      INITIAL_OVERRIDES[normalizeKind(kind)] ??
      (kind.trim() ? kind.trim().charAt(0).toUpperCase() : "?"),
  );
</script>

<span class="logo" style={`width:${size}px;height:${size}px`} aria-hidden="true">
  {#if glyph?.tile}
    <span
      class="logo__tile"
      style={`width:${size}px;height:${size}px;background:${glyph.tile.bg}`}
    >
      <svg viewBox="0 0 24 24" width={size} height={size} fill="none"
        >{@html glyph.tile.inner}</svg
      >
    </span>
  {:else if glyph?.path}
    <span
      class="logo__mask"
      style={`width:${size}px;height:${size}px;background-color:${brandColor};-webkit-mask-image:${maskUrl(
        glyph.path,
      )};mask-image:${maskUrl(glyph.path)}`}
    ></span>
  {:else if glyph}
    <span
      class="logo__tile"
      style={`width:${size}px;height:${size}px;background:${brandColor};font-size:${Math.max(
        size * 0.52,
        9,
      )}px`}
    >
      {initial}
    </span>
  {:else}
    <span
      class="logo__dot"
      style={`width:${size}px;height:${size}px;background:${brandColor}`}
    ></span>
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

  .logo__mask {
    display: inline-block;
    flex: none;
    -webkit-mask-position: center;
    mask-position: center;
    -webkit-mask-size: contain;
    mask-size: contain;
    -webkit-mask-repeat: no-repeat;
    mask-repeat: no-repeat;
  }

  .logo__tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 30%;
    color: #fff;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0;
    user-select: none;
    overflow: hidden;
  }

  .logo__dot {
    display: inline-block;
    flex: none;
    border-radius: 50%;
  }
</style>
