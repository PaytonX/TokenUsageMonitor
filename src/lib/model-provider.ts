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

import { BRAND_GLYPHS, FALLBACK_BRAND_COLOR, normalizeKind } from "./brand-glyphs";

export interface ProviderMeta {
  key: string;
  label: string;
}

/** 归一化后的 provider key → 展示名。
 *  值有两种形态：i18n 键（"mp.*"，随界面语言翻译）或品牌名本身（两种语言
 *  写法一致，不需要翻译）。渲染端一律经 $t(label)——tFor 对非键字符串原样
 *  返回，品牌名因此直通显示。 */
const PROVIDER_LABELS: Record<string, string> = {
  minimax: "MiniMax",
  deepseek: "DeepSeek",
  glm: "mp.glm",
  volcengine: "mp.volcengine",
  openai: "OpenAI",
  anthropic: "Claude",
  gemini: "Gemini",
  qwen: "Qwen",
  kimi: "Kimi",
  doubao: "mp.doubao",
  xai: "xAI",
  other: "mp.other",
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

/// 未登记品牌的取色池：避开品牌色簇（蓝紫为主），用暖色/粉/青等拉开差距。
/// 仍以 key 哈希取色——同一 Provider 每次渲染颜色稳定。
/// 10 色、跨色相均匀分布的调色板。容量按"同屏堆叠带通常 ≤8"取，
/// 保证贪心分配总能挑到与已用色足够远的项（曾用 6 色时第 7 条必然碰撞）。
const UNREGISTERED_PALETTE = [
  "#f2b35b", // 琥珀
  "#5fd4a2", // 薄荷绿
  "#f27b9b", // 玫红
  "#6fd1d1", // 青
  "#b58cf5", // 紫
  "#e8934a", // 橙
  "#a3d977", // 黄绿
  "#7f9cf5", // 靛蓝（明显亮于品牌蓝簇）
  "#e86ec1", // 品红
  "#8ed9a0", // 浅绿
];

/** 品牌注册表里是否有该 provider 的**真实矢量图标**（而非仅颜色占位）。 */
function hasBrandGlyph(key: string): boolean {
  const g = BRAND_GLYPHS[normalizeKind(key)];
  return !!g && (!!g.path || !!g.tile);
}

function hashOf(key: string): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/**
 * 堆叠图配色：**主次分层**。
 * - 已登记真实品牌图标的 Provider（minimax / deepseek / openai / gemini /
 *   kimi / volcengine / anthropic / xai…）→ 品牌色，保证识别锚点；
 * - 未登记的（glm 智谱、other 兜底桶）→ 从避开品牌色簇的调色板按 key 哈希取色，
 *   同一 key 稳定、不同 key 高概率不同。
 *
 * 不能一律走品牌色：现有品牌色本身集中在蓝紫区（minimax #5b8cff 与
 * deepseek #4d6bfe 仅差 6/16/68），而未登记项回落的系统蓝 #3b82f6 也在同一
 * 区间，多条堆叠带叠在一起几乎无法分辨。
 */
function brandColorOf(key: string): string | null {
  if (!hasBrandGlyph(key)) return null;
  return BRAND_GLYPHS[normalizeKind(key)].color || null;
}

export function providerColor(key: string): string {
  return brandColorOf(key) ?? UNREGISTERED_PALETTE[hashOf(key) % UNREGISTERED_PALETTE.length];
}

/** 颜色 → `[r,g,b]`。hex 形态直接切片；rgb()/rgba() 形态（brand-glyphs 兜底灰
 *  rgb(138,143,152)）用正则取三元组，避免 colorDistance 被 NaN 毒化。 */
function rgbOf(hex: string): [number, number, number] {
  const m = /^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(?:,[^)]*)?\)$/i.exec(hex.trim());
  if (m) return [+m[1], +m[2], +m[3]];
  return [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ];
}

function colorDistance(a: string, b: string): number {
  const [ar, ag, ab] = rgbOf(a);
  const [br, bg, bb] = rgbOf(b);
  return Math.round(Math.sqrt((ar - br) ** 2 + (ag - bg) ** 2 + (ab - bb) ** 2));
}

/** 堆叠带可接受的最小色距（RGB 欧氏）。低于此值肉眼难以分辨层次。 */
const MIN_STACK_DISTANCE = 90;

/**
 * 堆叠图调色板：**主次分层**。`orderedKeys` 必须按用量降序。
 *
 * - 主（首个）：用品牌色——它是最强的那条带，识别锚点最值钱；
 * - 次：先试品牌色，但若与**已分配色**的距离 < {@link MIN_STACK_DISTANCE}
 *   就从调色板（含哈希偏好项）里挑离已用色最远的一个——既保品牌优先，
 *   又保证任意两条相邻堆叠带肉眼可分。
 *
 * 为什么需要这层：现有品牌色本身集中在蓝紫区（minimax #5b8cff 与
 * deepseek #4d6bfe 色距仅 36），全部采用品牌色会让堆叠带糊成一片。
 */
export function providerPalette(orderedKeys: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  const used: string[] = [];
  orderedKeys.forEach((key, i) => {
    const brand = brandColorOf(key);
    const candidates: string[] = [];
    if (i === 0 && brand) {
      candidates.push(brand); // 主：无条件品牌色
    } else if (brand && used.every((u) => colorDistance(brand, u) >= MIN_STACK_DISTANCE)) {
      candidates.push(brand); // 次：品牌色不撞就保留
    }
    const hashed = UNREGISTERED_PALETTE[hashOf(key) % UNREGISTERED_PALETTE.length];
    candidates.push(hashed);
    for (const c of UNREGISTERED_PALETTE) candidates.push(c);

    // 在候选里挑"离已用色最远"的一个（同距时保持候选序 = 品牌优先）
    let best = candidates[0];
    let bestD = -1;
    for (const c of candidates) {
      const d = used.length === 0 ? Infinity : Math.min(...used.map((u) => colorDistance(c, u)));
      if (d > bestD) {
        bestD = d;
        best = c;
      }
    }
    out[key] = best;
    used.push(best);
  });
  return out;
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
