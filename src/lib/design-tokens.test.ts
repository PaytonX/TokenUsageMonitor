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

/** 禁用 hex → 豁免文件（相对 src）。豁免必须附原因。 */
const FORBIDDEN_HEX: Array<{ hex: string; exempt: string[]; why: string }> = [
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

/** 禁用 rgba 字面量（同色相换了形态逃避 hex 规则的）。 */
const FORBIDDEN_RGBA: Array<{ pattern: string; exempt: string[]; why: string }> = [
  { pattern: "rgba(232, 234, 240", exempt: [], why: "外来文字色 rgba 形态，按 alpha 映射到 text-muted/text-secondary" },
];

describe("design token lint", () => {
  it("svelte 源码不含禁用 hex（注释除外）", () => {
    const hits: string[] = [];
    for (const { path, rel } of FILES) {
      const code = stripComments(readFileSync(path, "utf8"));
      for (const rule of FORBIDDEN_HEX) {
        if (rule.exempt.includes(rel)) continue;
        if (hasHex(code, rule.hex)) hits.push(`${rel}: ${rule.hex} (${rule.why})`);
      }
    }
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("svelte 源码不含禁用 rgba 字面量（注释除外）", () => {
    const hits: string[] = [];
    for (const { path, rel } of FILES) {
      const code = stripComments(readFileSync(path, "utf8"));
      for (const rule of FORBIDDEN_RGBA) {
        if (rule.exempt.includes(rel)) continue;
        if (code.includes(rule.pattern)) hits.push(`${rel}: ${rule.pattern} (${rule.why})`);
      }
    }
    expect(hits, hits.join("\n")).toEqual([]);
  });

  it("扫描覆盖了足够多的 svelte 文件", () => {
    expect(FILES.length, `仅扫描到 ${FILES.length} 个 svelte 文件，疑似目录扫描失败`).toBeGreaterThan(10);
  });
});
