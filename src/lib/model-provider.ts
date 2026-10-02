// 模型名 → Provider 归属层：趋势/日历"跨工具按 Provider 统计"的归因地基。
//
// 三层归因顺序：
// 1. 模型名前缀匹配（MiniMax-M3 → minimax、deepseek-v4 → deepseek、GLM → 智谱…）；
// 2. 单 Provider 工具兜底：MiniMax Code 只会调 MiniMax 模型、DeepSeek Harness
//    只会调 DeepSeek——它们的"未标记"行按工具默认归属（源数据 model 列大量
//    为空，这条规则挽回近半用量的归因）；
// 3. 都不命中 → "other"（诚实单列，不强行归因）。
//
// 颜色取品牌色（brand-glyphs 的品牌注册表），趋势线/日历联动/徽标共用。

import { brandColorFor } from "./brand-glyphs";

export interface ProviderMeta {
  key: string;
  label: string;
}

/** 归一化后的 provider key → 展示名。 */
const PROVIDER_LABELS: Record<string, string> = {
  minimax: "MiniMax",
  deepseek: "DeepSeek",
  glm: "GLM 智谱",
  volcengine: "火山方舟",
  openai: "OpenAI",
  anthropic: "Claude",
  gemini: "Gemini",
  qwen: "Qwen",
  kimi: "Kimi",
  doubao: "豆包",
  xai: "xAI",
  other: "其他",
};

/** 模型名前缀 → provider key。按最长前缀优先匹配。 */
const PREFIX_RULES: [string, string][] = [
  ["minimax", "minimax"],
  ["deepseek", "deepseek"],
  ["glm", "glm"],
  ["chatglm", "glm"],
  ["ark-", "volcengine"],
  ["doubao", "doubao"],
  ["gpt", "openai"],
  ["o1", "openai"],
  ["o3", "openai"],
  ["o4", "openai"],
  ["text-embedding", "openai"],
  ["claude", "anthropic"],
  ["gemini", "gemini"],
  ["qwen", "qwen"],
  ["qwq", "qwen"],
  ["kimi", "kimi"],
  ["moonshot", "kimi"],
  ["grok", "xai"],
  ["spark", "spark"],
  ["ernie", "other"],
  ["hunyuan", "other"],
];

/** 单 Provider 工具的工具 id → 默认 provider key。 */
const TOOL_DEFAULT_PROVIDER: Record<string, string> = {
  "minimax-code": "minimax",
  "deepseek-harness": "deepseek",
};

export function providerLabel(key: string): string {
  return PROVIDER_LABELS[key] ?? key;
}

export function providerColor(key: string): string {
  return brandColorFor(key);
}

/** 模型名（+ 可选的来源工具 id）→ provider key。 */
export function providerForModel(model: string, source?: string): string {
  const n = model.toLowerCase();
  // 最长前缀优先，避免 "deepseek-flash" 被 "dee…" 之类短规则抢先。
  let best: { len: number; key: string } | null = null;
  for (const [prefix, key] of PREFIX_RULES) {
    if (n.includes(prefix) && (!best || prefix.length > best.len)) {
      best = { len: prefix.length, key };
    }
  }
  if (best) return best.key;
  if (source && TOOL_DEFAULT_PROVIDER[source]) return TOOL_DEFAULT_PROVIDER[source];
  return "other";
}
