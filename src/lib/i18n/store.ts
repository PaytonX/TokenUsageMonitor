// i18n 响应式封装（svelte/store，普通 .ts 模块——与 Svelte 5 runes 组件完全
// 兼容，也规避了 svelte-check 对 `.svelte.ts` 模块从 .ts 侧导入的解析限制）。
//
// 用法（模板里 $ 前缀自动订阅，语言切换后表达式自动重算）：
//   import { t } from "<相对路径>/i18n/store";
//   <span>{$t("tab.overview")}</span>
//   <span>{$t("freshness.minutesAgo", { n: 3 })}</span>
// 脚本（非响应式上下文）里取当前文案：`get(t)("key")`。
//
// 每个窗口入口（src/*-main.ts 等）在 mount 前调用一次 initLocaleWithBackend()；
// 设置窗口切换语言调 applyLocaleSetting()（写 localStorage 镜像 + 立即生效）；
// 收到后端 settings-changed 的窗口经 syncLocaleFromSetting() 镜像并按需切换。

import { derived, writable } from "svelte/store";
import { getSettings, onSettingsChanged } from "../api";
import { resolveLocale, tFor, type LangSetting } from "./dicts";

export type { LangSetting, Locale } from "./dicts";

/** localStorage 键：语言设置镜像（其它窗口启动时直接读取，不必等 IPC）。 */
export const LOCALE_KEY = "tum.locale";

/** 设置原值（"auto" | 显式语言），后端 `Settings.language` 的镜像。 */
export const langSetting = writable<LangSetting>("auto");

/** 当前生效语言（解析后）。 */
export const locale = derived(langSetting, (s) => resolveLocale(s));

/** 取词函数。激活语言缺键回退 zh-CN，再缺则返回键名本身（渐进迁移的兜底）。 */
export const t = derived(locale, (l) => {
  return (key: string, params?: Record<string, string | number>): string =>
    tFor(l, key, params);
});

locale.subscribe((l) => {
  try {
    document.documentElement.lang = l;
  } catch {
    /* ignore（node 测试环境） */
  }
});

function isKnownSetting(v: string | null | undefined): v is LangSetting {
  return v === "zh-CN" || v === "en" || v === "auto";
}

/** 窗口入口调用一次：读 localStorage 镜像并生效（无镜像 = 跟随系统）。 */
export function initLocale(): void {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(LOCALE_KEY);
  } catch {
    /* ignore（node / 隐私模式） */
  }
  if (isKnownSetting(raw)) langSetting.set(raw);
}

/** 设置窗口切换语言：立即生效并写镜像；其它窗口随后经 settings-changed 同步。 */
export function applyLocaleSetting(setting: LangSetting): void {
  try {
    localStorage.setItem(LOCALE_KEY, setting);
  } catch {
    /* ignore */
  }
  langSetting.set(setting);
}

/** 收到后端设置（初始读取或 settings-changed）时调用：镜像到本地并按需切换。
 *  未知值（旧配置缺省 / dev-mock 桩）不动本地状态。 */
export function syncLocaleFromSetting(
  setting: string | null | undefined,
): void {
  if (!isKnownSetting(setting)) return;
  try {
    if (localStorage.getItem(LOCALE_KEY) !== setting) {
      localStorage.setItem(LOCALE_KEY, setting);
    }
  } catch {
    /* ignore */
  }
  langSetting.set(setting);
}

/** 窗口入口一站式接线：镜像先即时生效，再异步对齐后端配置并订阅后续变更。
 *  在每个 main 入口 mount 前调用一次（dev-mock 浏览器预览下 IPC 桩静默失败，
 *  仅镜像生效，行为退化为「读 localStorage / 跟随系统」，可接受）。 */
export function initLocaleWithBackend(): void {
  initLocale();
  void getSettings()
    .then((s) => syncLocaleFromSetting(s.language))
    .catch(() => {
      /* dev-mock / 后端不可用：保持镜像值 */
    });
  void onSettingsChanged((s) => syncLocaleFromSetting(s.language));
}
