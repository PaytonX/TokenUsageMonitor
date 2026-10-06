// 工具用量三系列（输入/缓存/输出）的颜色唯一来源；ToolPanel 与 ToolWindow 共用，
// 防止两处色板漂移。数据系列色属设计文档允许的可编程强调色，不走 --tum-* 令牌。
export const TOOL_SERIES = [
  { key: "input", label: "输入", color: "#76a9ff" },
  { key: "cache_read", label: "缓存", color: "#ffcc66" },
  { key: "output", label: "输出", color: "#4cc2ff" },
] as const;
