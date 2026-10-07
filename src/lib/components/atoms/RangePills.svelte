<!--
  RangePills — 分段区间选择
  ───────────────────────────────────────────────────────────────────
  替代 TrendPanel / ModelPanel / ToolPanel / ToolWindow / TrendWindow
  里散落的 `__range / __range:hover / __range.is-active` 五件套 CSS。

  默认走「无填充 + 主文字」风格；带 accent 时激活态改用 accent 半透底。
-->
<script lang="ts" generics="T extends string">
  import { t } from "../../i18n/store";

  interface RangeOption {
    key: T;
    label: string;
  }

  interface Props {
    options: RangeOption[];
    value: T;
    onChange: (key: T) => void;
    /** 强调色（#RRGGBB）。激活态底色由此推导；不传走 --tum-accent。 */
    accent?: string;
    /** aria-label，缺省取词典 atom.rangeLabel（"时间区间"）。 */
    label?: string;
  }

  let {
    options,
    value,
    onChange,
    accent,
    label,
  }: Props = $props();

  // 把 #RRGGBB 转成 rgba(...)，与安全转回 CSS 变量。空值走 token。
  function activeBg(hex?: string): string {
    if (!hex) return "rgba(76, 194, 255, 0.18)";
    const m = hex.match(/^#?([0-9a-f]{6})$/i);
    if (!m) return "rgba(76, 194, 255, 0.18)";
    const r = parseInt(m[1].slice(0, 2), 16);
    const g = parseInt(m[1].slice(2, 4), 16);
    const b = parseInt(m[1].slice(4, 6), 16);
    return `rgba(${r}, ${g}, ${b}, 0.18)`;
  }

  let bg = $derived(activeBg(accent));
</script>

<div class="pills" role="group" aria-label={label ?? $t("atom.rangeLabel")}>
  {#each options as opt (opt.key)}
    <button
      type="button"
      class="pills__btn"
      class:is-active={value === opt.key}
      style={value === opt.key && accent ? `--pills-active-bg:${bg}` : ""}
      onclick={(e) => {
        e.stopPropagation();
        onChange(opt.key);
      }}
    >{opt.label}</button>
  {/each}
</div>

<style>
  .pills {
    display: inline-flex;
    align-items: center;
    gap: var(--tum-space-1);
  }

  .pills__btn {
    border: none;
    background: transparent;
    color: var(--tum-text-muted);
    font-family: var(--tum-font);
    font-size: var(--tum-font-size-xs);
    padding: 2px var(--tum-space-2);
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition:
      background 0.2s ease,
      color 0.2s ease;
  }

  .pills__btn:hover {
    color: var(--tum-text-primary);
  }

  .pills__btn:focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: 2px;
  }

  .pills__btn.is-active {
    background: var(
      --pills-active-bg,
      rgba(76, 194, 255, 0.18)
    );
    color: var(--tum-text-primary);
  }
</style>