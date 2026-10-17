<!--
  设置窗口的实时预览（方案 .scratch/glance-bench/issues/02）。

  渲染**真实**的 ProviderCard，数据来自 dev-mock 的 fixture，不碰 IPC。

  ## 为什么 1:1 渲染而不是 transform: scale()

  最省事的做法是量一个缩小的容器套 `transform: scale(0.46)`。**不行**：
  `transform` 会创建新的 containing block 与 backdrop root，而本项目的
  视觉核心就是玻璃——DetailCard 用 `backdrop-filter: blur()`，其采样范围与
  坐标空间在 transform 祖先内会变；`position: fixed/absolute` 的浮层也会
  相对被缩放的祖先定位并同样缩放。结果是「渲染了真组件」但**观感不是真的**，
  用它去调样式比没有预览更糟。

  所以这里按 dashboard 的原始 400×680 渲染，设置窗口相应加宽。

  ## 排序与配色必须与主面板同源

  预览曾经自己写了一份 `localeCompare(provider_id)` 排序。那是**第二实现**：
  真机用的是 `cardOrder` 偏好（用户拖拽写回 localStorage）。两份实现必然分叉，
  于是「改排序 → 预览变化」这条核心验收根本触发不了——而且接上也可能接错。
  现在直接消费同一份 `readCardOrder()`，不重复实现排序。

  同理 `accent` 取自与主面板相同的 `accounts[].accent_color`；少了它，用户在
  设置里改的账户自定义色在预览里会静默回落到品牌色，看起来「正常」实则没生效。

  ## 有意省略的东西（预览范围内不存在）

  - 玻璃模糊层：静态底色代替。透明窗里 backdrop-filter 只采样窗内内容，
    放进设置窗这种实底背景上本来就取不到桌面。**保真的主张只覆盖
    「不引入 scale」这个决定，不覆盖玻璃的最终观感。**
  - hover / 点击：pointer-events: none，且不传 onHover/onSelect，
    预览内的卡片不会触发底部的明细浮层。
  - error / burn / active 三态：fixture 是健康态，卡片永远显示正常。
    这是已知取舍，见 issue「仍然未做」。
-->
<script lang="ts">
  import ProviderCard from "./ProviderCard.svelte";
  import { previewSnapshots } from "../dev-mock";
  import { readCardOrder } from "../prefs";
  import type { UsageSnapshot } from "../types";
  import type { Settings } from "../types";

  interface Props {
    /** 预览要展示的快照序列。默认用 fixture。 */
    snapshots?: UsageSnapshot[];
    /** 当前设置——取账户强调色的来源，与 App.svelte 的 accentById 同源。 */
    settings?: Settings | null;
    /** 轮询周期（秒）。决定新鲜度徽章的分档。 */
    pollIntervalSec?: number;
    /**
     * 预览里的时钟。不跟着真实时间跑，否则倒计时一直在跳，会掩盖
     * 「设置改动导致的排版变化」——那才是预览要让人看清的东西。
     */
    now?: number;
  }

  const {
    snapshots = previewSnapshots(),
    settings = null,
    pollIntervalSec = 300,
    now = Date.now(),
  }: Props = $props();

  /**
   * 与 App.svelte 同源的排序：按 `cardOrder` 偏好排，未列入的保持原序，
   * 不自行发明第二套排序规则。
   */
  const ordered = $derived.by(() => {
    const order = readCardOrder();
    if (order.length === 0) return snapshots;
    const pos = new Map(order.map((id, i) => [id, i]));
    return [...snapshots].sort(
      (a, b) => (pos.get(a.provider_id) ?? Infinity) - (pos.get(b.provider_id) ?? Infinity),
    );
  });

  /** 与 App.svelte 的 accentById 同源：instance_id -> 自定义色。 */
  const accentById = $derived(
    (settings?.accounts ?? []).reduce<Record<string, string>>((m, a) => {
      m[a.instance_id] = a.accent_color;
      return m;
    }, {}),
  );
</script>

<div class="preview">
  <div class="preview__frame">
    {#each ordered as snap (snap.provider_id)}
      <ProviderCard
        snapshot={snap}
        accent={accentById[snap.provider_id]}
        {pollIntervalSec}
        {now}
        lastRefreshAt={now}
        countdown={true}
      />
    {/each}
  </div>
</div>

<style>
  .preview {
    width: 100%;
    padding: var(--tum-space-3);
    box-sizing: border-box;
    /* 预览不吃交互：它是设置的「示意图」，不是第二个可操作面板。 */
    pointer-events: none;
    user-select: none;
    overflow: hidden;
  }

  /* 与 dashboard 同宽（400），使内部组件走完全相同的排版分支。
     刻意不加 transform / zoom —— 理由见文件头。 */
  .preview__frame {
    width: 400px;
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
    /* 玻璃层的替身：静态底色。真实 dashboard 里 backdrop-filter 只采样
       窗内内容（见 DetailCard 的注释），在实底背景上取不到桌面，
       所以这里不假装有模糊，直接给一个可读的实色面板。 */
    background: rgba(30, 33, 38, 0.72);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    padding: var(--tum-space-3);
    box-sizing: border-box;
  }
</style>
