<!--
  ColorSwatch — 彩色小色块（图例 / 状态点）
  ───────────────────────────────────────────────────────────────────
  替代 TrendPanel / TrendLineChart / PillsOrSelect / TrendWindow /
  Settings / App 里散落的内联 `<i style="background:...">`。

  用法：
    <ColorSwatch color={activeColors[id] ?? "rgb(138,143,152)"} />
    <ColorSwatch color="#fbbf24" size={6} shape="round" />
-->
<script lang="ts">
  interface Props {
    /** 推荐 #RRGGBB 或 rgb() / rgba()；也接受命名 CSS 色。 */
    color: string;
    /** 像素边长。 */
    size?: number;
    /** square 默认；round 用于纯状态点。 */
    shape?: "square" | "round";
    /** 额外 class，便于定位覆盖。 */
    class?: string;
  }

  let {
    color,
    size = 8,
    shape = "square",
    class: klass = "",
  }: Props = $props();

  let radius = $derived(shape === "round" ? "50%" : "var(--tum-radius-xs)");
</script>

<i
  class={`swatch ${klass}`}
  style={`width:${size}px;height:${size}px;background:${color};border-radius:${radius}`}
  aria-hidden="true"
></i>

<style>
  .swatch {
    display: inline-block;
    flex: none;
  }
</style>