<script lang="ts">
  // 药丸 / 下拉自适应选择器：条目 ≤3 时用胶囊按钮铺开；>3 时改原生下拉，
  // 避免多个选中项挤压面板空间。用于模型、工具、Provider 等选择。
  export interface ChoiceItem {
    id: string;
    label: string;
  }

  let {
    items,
    value,
    onPick,
    dotFor,
  }: {
    items: ChoiceItem[];
    value: string | null;
    onPick: (id: string) => void;
    /** 可选：为每个条目渲染一个小圆点（如 Provider 强调色）。 */
    dotFor?: (id: string) => string | undefined;
  } = $props();

  let useSelect = $derived(items.length > 3);

  function onSel(e: Event) {
    const sel = e.currentTarget as HTMLSelectElement;
    onPick(sel.value);
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
      <button
        type="button"
        class="ps__pill"
        class:is-active={it.id === value}
        onclick={() => onPick(it.id)}
      >
        {#if dotFor}
          <i class="ps__dot" style={`background:${dotFor(it.id) ?? "#8a8f98"}`}></i>
        {/if}
        {it.label}
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
    border: 1px solid var(--tum-border);
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    padding: 2px 10px;
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .ps__pill:hover {
    color: var(--tum-text-primary);
    border-color: var(--tum-border-strong);
  }
  .ps__pill.is-active {
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    border-color: var(--tum-accent-stroke);
  }

  .ps__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex: none;
  }
</style>