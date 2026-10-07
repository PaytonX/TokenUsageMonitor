<!--
  ZoomButton — 放大为独立窗口的 18×18 图标按钮
  ───────────────────────────────────────────────────────────────────
  替代 TrendPanel / ToolPanel / ModelPanel 各自定义的 `__zoom` 按钮。

  字形 ⤢（U+2922）走 tum-font-mono，等宽；hover 走 accent 描边 + 半透底。

  用法：
    <ZoomButton title="放大为独立窗口" onclick={openWindow} />
-->
<script lang="ts">
  import { t } from "../../i18n/store";

  interface Props {
    title?: string;
    /** 默认 "⤢"。若未来需要换成 SVG 也由这一处统一。 */
    glyph?: string;
    onclick?: (e: MouseEvent) => void;
    /** 可访问名；不传时回退到 title。 */
    label?: string;
  }

  let { title, glyph = "⤢", onclick, label }: Props =
    $props();
</script>

<button
  type="button"
  class="zoom-btn"
  title={title ?? $t("atom.zoomTitle")}
  aria-label={label ?? title ?? $t("atom.zoomTitle")}
  onclick={onclick}
>⤢</button>

<style>
  .zoom-btn {
    flex: none;
    width: 18px;
    height: 18px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    background: var(--tum-surface);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    line-height: 1;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .zoom-btn:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .zoom-btn:focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: 2px;
  }
</style>