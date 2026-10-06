// 原子组件索引 ─── 仅在新页面/新面板里通过这里导入。
//
// 使用约定：
//   - 新页面优先从 `$lib/components/atoms` 取小零件；
//   - 已有的 PulseDot / UsageBar / MiniPanel / ResetCountdown / ProviderLogo
//     已是「天然原子」，保留原路径，不重复导入此 barrel；
//   - atoms 只承载「视觉外观 + 一两个 prop」，业务状态/数据获取放父级。
export { default as PanelHeader } from "./PanelHeader.svelte";
export { default as RangePills } from "./RangePills.svelte";
export { default as ZoomButton } from "./ZoomButton.svelte";
export { default as Stat } from "./Stat.svelte";
export { default as ColorSwatch } from "./ColorSwatch.svelte";