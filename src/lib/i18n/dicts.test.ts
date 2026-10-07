import { describe, expect, it } from "vitest";
import {
  DICTS,
  format,
  resolveLocale,
  tFor,
  zhCN,
} from "./dicts";

// 纯逻辑层测试（node 环境；store.ts 的 svelte/store 包装不在此覆盖）。

describe("i18n 词典", () => {
  it("两种语言的键集合完全一致", () => {
    const zh = Object.keys(zhCN).sort();
    const en = Object.keys(DICTS.en).sort();
    expect(en).toEqual(zh);
  });

  it("所有取值都是非空字符串", () => {
    for (const dict of [zhCN, DICTS.en]) {
      for (const [key, value] of Object.entries(dict)) {
        expect(typeof value, key).toBe("string");
        expect(value.trim().length, key).toBeGreaterThan(0);
      }
    }
  });

  it("键使用 <域>.<名称> 的点分命名", () => {
    for (const key of Object.keys(zhCN)) {
      expect(key).toMatch(/^[a-z][a-zA-Z0-9]*(\.[a-zA-Z0-9]+)+$/);
    }
  });
});

describe("resolveLocale", () => {
  it("显式语言直接生效", () => {
    expect(resolveLocale("en")).toBe("en");
    expect(resolveLocale("zh-CN")).toBe("zh-CN");
  });

  it("auto / 非法值 / 未设置 → 跟随系统（node 下无 navigator，回退 zh-CN）", () => {
    expect(resolveLocale("auto")).toBe("zh-CN");
    expect(resolveLocale("fr-FR")).toBe("zh-CN");
    expect(resolveLocale(null)).toBe("zh-CN");
    expect(resolveLocale(undefined)).toBe("zh-CN");
  });
});

describe("format 插值", () => {
  it("替换具名 {占位符}", () => {
    expect(format("{n} 分钟前", { n: 5 })).toBe("5 分钟前");
  });

  it("未知占位符保留原样，便于发现漏传参数", () => {
    expect(format("{a} → {b}", { a: 1 })).toBe("1 → {b}");
  });

  it("数字参数转字符串", () => {
    expect(format("约 {h} 小时 {m} 分钟", { h: 2, m: 5 })).toBe("约 2 小时 5 分钟");
  });
});

describe("tFor", () => {
  it("按语言取词并插值", () => {
    expect(tFor("zh-CN", "freshness.minutesAgo", { n: 3 })).toBe("3 分钟前");
    expect(tFor("en", "freshness.minutesAgo", { n: 3 })).toBe("3m ago");
  });

  it("激活语言缺键时回退 zh-CN", () => {
    // 临时往 zh-CN 塞一个 en 没有的键，验证回退链路；结束后清理。
    const probe = "common.__fallback_probe__";
    zhCN[probe] = "回退词条";
    try {
      expect(tFor("en", probe)).toBe("回退词条");
    } finally {
      delete zhCN[probe];
    }
  });

  it("两种语言都缺键时返回键名本身（渐进迁移的兜底）", () => {
    expect(tFor("zh-CN", "nope.missingKey")).toBe("nope.missingKey");
  });
});
