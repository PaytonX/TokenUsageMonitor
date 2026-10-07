// i18n 纯逻辑层：词典装配、语言解析、插值与取词。
// 不依赖 Svelte runes，可在 node 测试（vitest）中直接导入；
// 响应式包装见 ./store.ts。
//
// 语言设置取值（与 Rust `Settings.language` 约定一致）：
//   "auto" —— 跟随系统语言；
//   "zh-CN" | "en" —— 显式指定。
// 解析失败一律回退系统语言，系统语言不是英文时回退中文（项目默认语言）。

import { en } from "./en";
import { zhCN } from "./zh-CN";

export type Locale = "zh-CN" | "en";
/** 设置项里的语言值：显式语言或 "auto"。 */
export type LangSetting = "auto" | Locale;
export type Dict = Record<string, string>;

export { zhCN };

export const LOCALES: readonly Locale[] = ["zh-CN", "en"];
export const LANG_SETTINGS: readonly LangSetting[] = ["auto", "zh-CN", "en"];

export const DICTS: Record<Locale, Dict> = {
  "zh-CN": zhCN,
  en,
};

/** 系统语言 → 受支持的语言；不认识的区域一律回退中文。 */
export function systemLocale(): Locale {
  if (typeof navigator === "undefined" || !navigator.language) return "zh-CN";
  return navigator.language.toLowerCase().startsWith("en") ? "en" : "zh-CN";
}

/** 把设置值解析为实际生效的语言：非法值按 "auto" 处理。 */
export function resolveLocale(setting: string | null | undefined): Locale {
  if (setting === "zh-CN" || setting === "en") return setting;
  return systemLocale();
}

/** 插值：把模板里的 {name} 占位符替换为参数。缺参保留原样，便于发现漏传。 */
export function format(
  template: string,
  params?: Record<string, string | number>,
): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (whole, key: string) =>
    Object.prototype.hasOwnProperty.call(params, key)
      ? String(params[key])
      : whole,
  );
}

/** 纯函数取词：激活语言缺键回退 zh-CN，再缺则返回键名本身。 */
export function tFor(
  locale: Locale,
  key: string,
  params?: Record<string, string | number>,
): string {
  const text = DICTS[locale]?.[key] ?? zhCN[key] ?? key;
  return format(text, params);
}
