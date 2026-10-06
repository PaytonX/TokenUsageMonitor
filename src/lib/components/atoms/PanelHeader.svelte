<!--
  PanelHeader — 面板/窗口标题区
  ───────────────────────────────────────────────────────────────────
  替代散布在 TrendPanel / ModelPanel / ToolPanel / DevicePanel /
  ToolWindow / TrendWindow 里的 `__head + __title + __head-right` 三件套。

  设计语言：标题一律 uppercase + 等宽 + 字距 1.2px（弱化次要标题），
  右侧槽承载 RangePills / ZoomButton / 自定义 actions。

  用法：
    <PanelHeader title="趋势看板 · 本机工具">
      <RangePills options={RANGES} value={range} onChange={setRange} />
      <ZoomButton title="放大为独立窗口" onclick={openWindow} />
    </PanelHeader>
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    /** aria-label，覆盖默认由 title 推导。窗口级渲染时给具体描述。 */
    label?: string;
    /** 右侧槽：range pills / zoom / 自定义 actions。 */
    children?: Snippet;
  }

  let { title, label, children }: Props = $props();
</script>

<header class="panel-header" aria-label={label ?? title}>
  <span class="panel-header__title">{title}</span>
  <div class="panel-header__actions" role="group">
    {@render children?.()}
  </div>
</header>

<style>
  .panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--tum-space-2);
    flex-wrap: wrap;
    flex: none;
  }

  .panel-header__title {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
  }

  .panel-header__actions {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }
</style>