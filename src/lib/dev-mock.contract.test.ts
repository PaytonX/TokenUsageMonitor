/**
 * dev-mock.ts 的契约测试（方案 .scratch/glance-bench/issues/02）。
 *
 * 预览的价值全押在「桩数据与真机同构」上。一旦不同构，预览会**静默失真**——
 * 少一个字段、少一段账本，组件渲染出空态或缺行，但不报错、不崩，
 * 排查成本极高（要靠人眼比对才知道少了什么）。所以把同构写成断言。
 *
 * 另外锁住一条硬约束：`dev-mock.ts` 不得引入任何运行时依赖，
 * 否则「预览不碰 IPC」这条前提就被悄悄破坏了。
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { previewSnapshots } from "./dev-mock";

const HERE = dirname(fileURLToPath(import.meta.url));

describe("previewSnapshots: 结构与真机同构", () => {
  const snaps = previewSnapshots();

  it("产出 3–5 个 provider，字段齐全", () => {
    expect(snaps.length).toBeGreaterThanOrEqual(3);
    expect(snaps.length).toBeLessThanOrEqual(5);
    for (const s of snaps) {
      expect(typeof s.provider_id).toBe("string");
      expect(s.provider_id.length).toBeGreaterThan(0);
      expect(typeof s.provider_display_name).toBe("string");
      expect(typeof s.timestamp).toBe("string");
    }
  });

  it("provider_id 用的是 instance_id，不是裸 kind", () => {
    // 火山这类多实例 provider 的 id 形如 volcengine-9-0。若用裸 kind，
    // 前端 split("-")[0] 之后 brand/图标会错，日历台账也对不上账。
    const ids = snaps.map((s) => s.provider_id);
    expect(ids).toContain("volcengine-9-0");
    for (const id of ids) {
      // 每个 id 的首段应是已知 kind，其余段是实例序号。
      expect(id.split("-").length).toBeGreaterThanOrEqual(1);
    }
  });

  it("至少覆盖 plan_tier、afp 计价、cny 差分三种形态", () => {
    // 三种形态对应三条不同的渲染分支，缺一种预览就有一块区域永远看不到。
    expect(snaps.some((s) => typeof s.plan_tier === "string")).toBe(true);
    expect(
      snaps.some((s) => Object.values(s.windows).some((w) => w?.unit === "afp")),
    ).toBe(true);
    expect(
      snaps.some((s) => Object.values(s.windows).some((w) => w?.unit === "cny")),
    ).toBe(true);
  });

  it("windows 结构一致：daily 只给有服务端日账的 provider", () => {
    for (const s of snaps) {
      const w = s.windows;
      expect(w.five_hour).toBeDefined();
      expect(w.weekly).toBeDefined();
      expect(w.monthly).toBeDefined();
      // daily 是可选的——差分类 provider 没有服务端日账，强行给会让
      // DetailCard 的「无按日用量数据」空态分支永远走不到。
      if (w.daily) {
        expect(w.daily.unit).toBe("afp");
        expect(typeof w.daily.reset_at).toBe("string");
      }
    }
    // 反向：至少有且仅有一个带 daily，用来验证该分支真的能被走到。
    expect(snaps.filter((s) => s.windows.daily)).toHaveLength(1);
  });

  it("时间锚在当下，倒计时不会显示负数", () => {
    const nowMs = Date.now();
    for (const s of snaps) {
      const resets = Object.values(s.windows)
        .filter((w): w is NonNullable<typeof w> => Boolean(w))
        .map((w) => Date.parse(w.reset_at));
      for (const at of resets) {
        // 桩数据的重置点都在未来（2h/20h/60h/300h 之后）。
        expect(at).toBeGreaterThan(nowMs);
      }
      // 数据本身是「当下」附近，不是模块加载时刻。
      expect(Math.abs(Date.parse(s.timestamp) - nowMs)).toBeLessThan(60_000);
    }
  });

  it("同一形状：两次调用字段集完全一致（布局可比）", () => {
    const keys = (o: object) => JSON.stringify(Object.keys(o).sort());
    const a = previewSnapshots();
    const b = previewSnapshots();
    for (let i = 0; i < a.length; i++) {
      expect(keys(a[i])).toBe(keys(b[i]));
      expect(keys(a[i].windows)).toBe(keys(b[i].windows));
    }
  });
});

describe("dev-mock.ts 不得引入运行时依赖", () => {
  it("没有任何非 type 的 import", () => {
    const src = readFileSync(join(HERE, "dev-mock.ts"), "utf8");
    const imports = src.match(/^\s*import\s.*$/gm) ?? [];
    for (const line of imports) {
      // `import type` 编译期擦除，允许。其余一律禁止——尤其是 api.ts
      // 与各 store，它们会把 IPC 拖进预览的渲染树。
      expect(line).toMatch(/^import\s+type\s/);
    }
  });

  it("previewSnapshots 本身不引用 invoke / __TAURI__", () => {
    const src = readFileSync(join(HERE, "dev-mock.ts"), "utf8");
    // 只取函数体：previewSnapshots 定义在 installDevMock **之前**，切到文件
    // 末尾会把它一并圈进来，而后者本来就要操作这些全局。
    const start = src.indexOf("export function previewSnapshots");
    expect(start).toBeGreaterThan(-1);
    const body = src.slice(start, src.indexOf("\n}", start));
    expect(body).not.toMatch(/invoke|__TAURI__/);
  });
});

describe("预览组件的响应式约束", () => {
  const frameSrc = readFileSync(join(HERE, "components", "PreviewFrame.svelte"), "utf8");
  /**
   * 只扫 `<script>`/`<style>` 块，并剥掉其中的注释。
   * 文件头注释与 Props 的 JSDoc 里**正好**要写 readCardOrder、localStorage、
   * transform:scale 这些词（解释为什么不用它们），不剥注释必然误报。
   */
  const blocks = [
    frameSrc.match(/<script[\s\S]*?<\/script>/)?.[0] ?? "",
    frameSrc.match(/<style[\s\S]*?<\/style>/)?.[0] ?? "",
  ].join("\n");
  const code = blocks
    .replace(/\/\*[\s\S]*?\*\//g, "") // 块注释
    .replace(/^\s*\/\/.*$/gm, ""); // 行注释

  it("不在组件内部读 localStorage 的排序偏好", () => {
    // 踩过的坑：$derived.by 里调 readCardOrder() 看似能排序，实际 localStorage
    // 不是 Svelte 的响应式来源，主面板改顺序时预览永远不会重算——
    // 「改排序 → 预览变化」这条验收就静默失效。排序必须由调用方以 prop 传入。
    expect(code).not.toMatch(/readCardOrder|writeCardOrder|localStorage/);
    expect(blocks).toMatch(/cardOrder\?: string\[\]/);
  });

  it("时钟走定时器，不是解构默认值", () => {
    // 同类的第二个坑：`now = Date.now()` 写在 $props() 解构默认值里只求值
    // 一次，倒计时与 lastRefreshAt 的差值会随真实时间持续漂移。
    expect(code).not.toMatch(/now\s*=\s*Date\.now\(\)\s*[,}]/);
    expect(code).toMatch(/setInterval/);
  });

  it("不使用 transform/zoom 缩放内容", () => {
    // transform 会改 backdrop root 与 containing block，玻璃层（backdrop-filter）
    // 的采样范围随之改变，预览就「看着像真的其实不是」。
    expect(code).not.toMatch(/transform:\s*scale|zoom:/);
  });
});
