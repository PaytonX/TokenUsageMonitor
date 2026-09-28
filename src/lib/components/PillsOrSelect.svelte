<script lang="ts">
  // 药丸 / 下拉自适应选择器：条目 ≤3 时用胶囊按钮铺开；>3 时改原生下拉，
  // 避免多个选中项挤压面板空间。用于模型、工具、Provider 等选择。
  import { hexToRgb } from "../types";
  import ProviderLogo from "./ProviderLogo.svelte";

  export interface ChoiceItem {
    id: string;
    label: string;
  }

  /** 可选品牌视觉：色 + 品牌 kind（渲染 14px glyph）+ 是否实验性来源。 */
  export interface ChoiceBrand {
    color: string;
    kind?: string;
    experimental?: boolean;
  }

  let {
    items,
    value,
    onPick,
    dotFor,
    brandFor,
  }: {
    items: ChoiceItem[];
    value: string | null;
    onPick: (id: string) => void;
    /** 可选：为每个条目渲染一个小圆点（如 Provider 强调色）。 */
    dotFor?: (id: string) => string | undefined;
    /** 可选：选中项的徽章视觉（品牌 16% 底 / 45% 描边 / 纯色文字 + 品牌 glyph）。 */
    brandFor?: (id: string) => ChoiceBrand | null;
  } = $props();

  let useSelect = $derived(items.length > 3);

  function onSel(e: Event) {
    const sel = e.currentTarget as HTMLSelectElement;
    onPick(sel.value);
  }

  function badgeStyle(brand: ChoiceBrand | null): string {
    if (!brand) return "";
    const rgb = hexToRgb(brand.color);
    if (!rgb) return "";
    return [
      `background: rgba(${rgb}, 0.16)`,
      `border-color: rgba(${rgb}, 0.45)`,
      `color: ${brand.color}`,
    ].join("; ");
  }
</script>

{#if useSelect}
  <select class="ps" value={value ?? ""} onchange={onSel} aria-label="选择">
    {#each items as it (it.id)}
      <option value={it.id}>{it.label}</option>
    {/each}
  </select>
{:else}
  <div class="ps__pills" role="group" aria-label="选择">
    {#each items as it (it.id)}
      {@const brand = brandFor?.(it.id) ?? null}
      {@const active = it.id === value}
      <button
        type="button"
        class="ps__pill"
        class:is-active={active}
        style={active ? badgeStyle(brand) : ""}
        onclick={() => onPick(it.id)}
      >
        {#if active && brand?.kind}
          <ProviderLogo kind={brand.kind} size={14} />
        {:else if dotFor}
          <i class="ps__dot" style={`background:${dotFor(it.id) ?? "#8a8f98"}`}></i>
        {/if}
        {it.label}
        {#if active && brand?.experimental}
          <span
            class="ps__exp"
            style={brand.color
              ? `background: rgba(${hexToRgb(brand.color) ?? "255,255,255"}, 0.22)`
              : ""}
          >实验</span>
        {/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .ps {
    flex: none;
    max-width: 100%;
    padding: 3px 8px;
    font-family: var(--tum-font);
    font-size: 11px;
    color: #e8eaf0;
    background: rgba(45, 50, 60, 0.96);
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-sm);
    outline: none;
    cursor: pointer;
  }
  .ps:focus-visible {
    border-color: var(--tum-accent);
  }
  /* 原生下拉选项：显式给深色底 + 亮色文字，避免"深底深字"看不清，仅靠悬停
     反差才能辨认。 */
  .ps option {
    background-color: #23262d;
    color: #e8eaf0;
  }

  .ps__pills {
    display: flex;
    gap: 2px;
    flex-wrap: wrap;
  }

  .ps__pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    border: 1px solid var(--tum-border);
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    padding: 0 10px;
    border-radius: 11px;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .ps__pill:hover {
    color: var(--tum-text-primary);
    border-color: var(--tum-border-strong);
  }
  /* 选中项：品牌色 16% 底 / 45% 描边 / 纯色文字，由内联 style 注入；
     未提供品牌信息时回退到系统强调色。 */
  .ps__pill.is-active {
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    border-color: var(--tum-accent-stroke);
    font-weight: 600;
  }

  .ps__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex: none;
  }
  .ps__exp {
    font-size: 10px;
    font-weight: 600;
    border-radius: 6px;
    padding: 1px 5px;
    color: inherit;
  }
</style>