/// <reference types="node" />
// src/lib/design-tokens.test.ts
// 设计令牌合规 lint：扫描全部 .svelte（Settings.svelte 整体豁免，单独任务处理）。
// 规则随迁移逐批启用；每条禁用项的豁免必须写原因，目标清零。
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(process.cwd(), "src");

function listSvelte(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, name.name);
    if (name.isDirectory()) out.push(...listSvelte(p));
    else if (name.name.endsWith(".svelte")) out.push(p);
  }
  return out;
}

const FILES = listSvelte(SRC).filter((f) => !f.endsWith("Settings.svelte"));

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
  // 琥珀只允许：Settings 品牌点（遗留）、ColorSwatch 文档注释
  { hex: "#fbbf24", exempt: ["Settings.svelte", "lib/components/atoms/ColorSwatch.svelte"], why: "琥珀仅限品牌点/热力图今日描边" },
];

describe("design token lint", () => {
  it("svelte 源码不含禁用 hex（注释除外）", () => {
    const hits: string[] = [];
    for (const f of FILES) {
      const rel = f.slice(SRC.length + 1).replace(/\\/g, "/");
      const code = stripComments(readFileSync(f, "utf8"));
      for (const rule of FORBIDDEN_HEX) {
        if (rule.exempt.includes(rel)) continue;
        if (hasHex(code, rule.hex)) hits.push(`${rel}: ${rule.hex} (${rule.why})`);
      }
    }
    expect(hits, hits.join("\n")).toEqual([]);
  });
});
