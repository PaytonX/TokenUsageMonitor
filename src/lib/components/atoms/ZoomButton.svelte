<!--
  ZoomButton — 放大为独立窗口的 18×18 图标按钮
  ───────────────────────────────────────────────────────────────────
  替代 TrendPanel / ToolPanel / ModelPanel 各自定义的 `__zoom` 按钮。

  图标用内嵌 Lucide SVG（atoms/Icon.svelte，maximize）；hover 走 accent 描边 + 半透底。

  用法：
    <ZoomButton title="放大为独立窗口" onclick={openWindow} />
-->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { t } from "../../i18n/store";

  interface Props {
    title?: string;
    onclick?: (e: MouseEvent) => void;
    /** 可访问名；不传时回退到 title。 */
    label?: string;
  }

  let { title, onclick, label }: Props =
    $props();
</script>

<button
  type="button"
  class="zoom-btn"
  title={title ?? $t("atom.zoomTitle")}
  aria-label={label ?? title ?? $t("atom.zoomTitle")}
  onclick={onclick}
><Icon name="maximize" size={12} /></button>

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
    line-height: 1;
    cursor: pointer;
    transition: all var(--tum-dur-fast, 150ms) var(--tum-ease-spring);
  }

  .zoom-btn:active {
    transform: scale(0.94);
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