/// <reference types="node" />
// src/lib/design-tokens.test.ts
// 设计令牌合规 lint：扫描全部 .svelte（Settings.svelte 整体豁免，单独任务处理）。
// 规则随迁移逐批启用；每条禁用项的豁免必须写原因，目标清零。
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// 锚定到本模块上两级（src/），vitest 从任意 cwd 启动都能扫对。
// fileURLToPath 会保留 URL 尾部的分隔符，这里去掉，保证 rel = f.slice(SRC.length + 1) 成立。
const SRC = fileURLToPath(new URL("../", import.meta.url)).replace(/[\\/]+$/, "");

function listSvelte(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, name.name);
    if (name.isDirectory()) out.push(...listSvelte(p));
    else if (name.name.endsWith(".svelte")) out.push(p);
  }
  return out;
}

/** 统一以 src 相对路径（正斜杠）标识文件：排除与豁免都按精确路径匹配。 */
const FILES = listSvelte(SRC)
  .map((f) => ({ path: f, rel: f.slice(SRC.length + 1).replace(/\\/g, "/") }))
  .filter((file) => file.rel !== "Settings.svelte");

/** 去掉注释再匹配，避免文档示例误报（如 ColorSwatch 顶部的用法注释）。 */
function stripComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/(^|\s)\/\/[^\n]*/g, "$1");
}

/** 词边界匹配，#fff 不会命中 #fff000。 */
function hasHex(content: string, hex: string): boolean {
  return new RegExp(hex + "(?![0-9a-fA-F])", "i").test(content);
}

/** 颜色规则：hex 词边界，或 rgb()/rgba() 形态正则。豁免文件（相对 src）必须附原因。 */
type ColorRule = { exempt: string[]; why: string } & ({ hex: string } | { re: RegExp });

/** 禁用 hex → 豁免文件（相对 src）。豁免必须附原因。 */
const FORBIDDEN_HEX: ColorRule[] = [
  // 旧版 success/crit 别名色（组件库文档 §5 明令禁止）
  { hex: "#34d399", exempt: ["Settings.svelte"], why: "旧 success 绿，改用 --tum-ok" },
  { hex: "#f87171", exempt: ["Settings.svelte"], why: "旧 crit 红，改用 --tum-crit" },
  // 琥珀只允许：Settings 品牌点（遗留）。ColorSwatch 顶部的用法示例在 HTML 注释里，
  // stripComments 已将其剔除，无需文件级豁免（文件级豁免会静默放行未来的真实使用）。
  { hex: "#fbbf24", exempt: ["Settings.svelte"], why: "琥珀仅限品牌点/热力图今日描边" },
  { hex: "#ff7a6e", exempt: [], why: "偏色红，改用 --tum-crit" },
  { hex: "#ffc77a", exempt: [], why: "偏色琥珀，改用 --tum-warn" },
  { hex: "#ff5f56", exempt: ["Settings.svelte"], why: "裸 crit 值，改用 var(--tum-crit)" },
  { hex: "#6ccb5f", exempt: ["Settings.svelte"], why: "裸 ok 值，改用 var(--tum-ok)" },
  { hex: "#8a8f98", exempt: [], why: "图例兜底灰一律写 rgb(138,143,152)" },
  { hex: "#e8eaf0", exempt: [], why: "外来文字色，改用 var(--tum-text-primary)" },
  { hex: "#23262d", exempt: [], why: "外来弹层底色，改用 var(--tum-bg-solid)" },
];

/** 禁用 rgb()/rgba() 字面量（同色相换了形态逃避 hex 规则的）。
 *  正则匹配：无空格 / 多空格 / 大小写 / 无 alpha 的 rgb() 形态都拦得住。 */
const FORBIDDEN_RGBA: ColorRule[] = [
  { re: /rgba?\(\s*232\s*,\s*234\s*,\s*240/i, exempt: [], why: "外来文字色 rgba 形态，按 alpha 映射到 text-muted/text-secondary" },
];

/** 值保持型 px 规则：只禁恰好等于令牌值的裸 px；8/9/12/14/15/17/10.5px 等离散值
 *  是有意保留的遗留尺寸，\b 词边界 + 精确数值保证不误伤（110px / 10.5px 均不匹配）。 */
const FORBIDDEN_PX: ColorRule[] = [
  { re: /font-size:\s*(10|11|13)px\b/, exempt: [], why: "token 值字号禁止裸 px：10→var(--tum-font-size-xs) / 11→sm / 13→base" },
  { re: /border-radius:\s*(4|8|12|16|999)px\b/, exempt: [], why: "token 值圆角禁止裸 px：4→var(--tum-radius-xs) / 8→sm / 12→md / 16→lg / 999→pill" },
];

/** 文件迭代 + 注释剥离 + 豁免判定 + 命中格式只实现这一份，hex/rgba 两个测试共用。 */
function collectHits(rules: ColorRule[]): string[] {
  const hits: string[] = [];
  for (const { path, rel } of FILES) {
    const code = stripComments(readFileSync(path, "utf8"));
    for (const rule of rules) {
      if (rule.exempt.includes(rel)) continue;
      const what = "hex" in rule ? rule.hex : rule.re.source;
      if ("hex" in rule ? hasHex(code, rule.hex) : rule.re.test(code)) {
        hits.push(`${rel}: ${what} (${rule.why})`);
      }
    }
  }
  return hits;
}

describe("design token lint", () => {
  it("svelte 源码不含禁用 hex（注释除外）", () => {
    const hits = collectHits(FORBIDDEN_HEX);
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("svelte 源码不含禁用 rgba 字面量（注释除外）", () => {
    const hits = collectHits(FORBIDDEN_RGBA);
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("svelte 源码不含 token 值的裸 px 字号/圆角（注释除外）", () => {
    const hits = collectHits(FORBIDDEN_PX);
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("扫描覆盖了足够多的 svelte 文件", () => {
    expect(FILES.length, `仅扫描到 ${FILES.length} 个 svelte 文件，疑似目录扫描失败`).toBeGreaterThan(10);
  });
});
