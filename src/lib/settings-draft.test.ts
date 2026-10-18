/**
 * settings-draft 单测（方案 .scratch/glance-bench/issues/02b）。
 *
 * 这些用例守的是「用户丢配置」这条路径——`persistSettings()` 是整对象写，
 * 一个没被识别的冲突就是静默覆盖。
 */
import { describe, expect, it } from "vitest";
import { changedKeys, conflictingKeys, isDirty, normalizeBaseline } from "./settings-draft";

describe("settings-draft: changedKeys", () => {
  it("相同对象没有差异", () => {
    expect(changedKeys({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } })).toEqual([]);
  });

  it("新增/删除/改值都能识别", () => {
    expect(changedKeys({ a: 1 }, { a: 2 })).toEqual(["a"]);
    expect(changedKeys({ a: 1 }, { a: 1, b: 2 })).toEqual(["b"]);
    expect(changedKeys({ a: 1, b: 2 }, { a: 1 })).toEqual(["b"]);
  });

  it("嵌套对象按整体比较（内层变化也算改了）", () => {
    expect(changedKeys({ router: { on: false } }, { router: { on: true } })).toEqual([
      "router",
    ]);
  });

  it("嵌套对象键序不同但内容相同 → 判为未改", () => {
    // 这条是从对抗审查来的真问题：baseline 的嵌套对象来自 Rust 反序列化
    // （字段序由 Rust 决定），buildNext() 重建的那些由 JS 构造序决定。退回
    // JSON.stringify 逐键比较的话，顺序一不一致语义相同的值也会被判为「改过」，
    // 后果是打开设置就恒脏，且伪差异会进入 weTouched 把别人的真实改动过滤掉。
    expect(
      changedKeys({ router: { on: true, port: 1, name: "x" } }, { router: { name: "x", port: 1, on: true } }),
    ).toEqual([]);
    expect(
      changedKeys(
        { rate_overrides: { USD: 7, CNY: 7, EUR: 7 } },
        { rate_overrides: { EUR: 7, CNY: 7, USD: 7 } },
      ),
    ).toEqual([]);
  });

  it("数组元素顺序有意义（accounts 增删要能识别）", () => {
    expect(
      changedKeys({ accounts: [{ id: "a" }, { id: "b" }] }, { accounts: [{ id: "b" }, { id: "a" }] }),
    ).toEqual(["accounts"]);
  });

  it("null 基线不产出差异（尚未加载时不误报）", () => {
    expect(changedKeys(null, { a: 1 })).toEqual([]);
    expect(changedKeys({ a: 1 }, null)).toEqual([]);
  });
});

describe("settings-draft: normalizeBaseline", () => {
  it("把 hub_token_configured 整形成 true，与 buildNext() 对齐", () => {
    // buildNext() 里这个字段硬编码 true。若不整形，后端初值为 false 时
    // 打开设置就恒脏——撤销按钮永远可点，看着像有未保存改动。
    const out = normalizeBaseline({ autostart: true, hub_token_configured: false });
    expect(out.hub_token_configured).toBe(true);
    expect(out.autostart).toBe(true);
  });

  it("整形成后与 buildNext() 的对应产物判为无差异", () => {
    const baseline = normalizeBaseline({ poll_interval_seconds: 300, hub_token_configured: false });
    const staged = { poll_interval_seconds: 300, hub_token_configured: true };
    expect(changedKeys(baseline, staged)).toEqual([]);
  });
});

describe("settings-draft: isDirty", () => {
  it("与基线一致 → 干净", () => {
    expect(isDirty({ poll_interval_seconds: 300 }, { poll_interval_seconds: 300 })).toBe(
      false,
    );
  });

  it("任一字段变了 → 脏", () => {
    expect(isDirty({ poll_interval_seconds: 300 }, { poll_interval_seconds: 600 })).toBe(true);
  });
});

describe("settings-draft: conflictingKeys（丢数据防线）", () => {
  it("别人改了我们没动的 key → 报冲突", () => {
    // 典型：设置窗开着暂存，托盘把 always_on_top 关了，我们点保存会抹掉它。
    const conflicts = conflictingKeys(
      { poll_interval_seconds: 300, autostart: true },
      { poll_interval_seconds: 600, autostart: true },
      { poll_interval_seconds: 300, autostart: false },
    );
    expect(conflicts).toContain("autostart");
    expect(conflicts).not.toContain("poll_interval_seconds");
  });

  it("我们也改了同一个 key → 不算静默覆盖（后写为准，不报）", () => {
    // 这是用户在决定的事，不是丢数据。报出来只会变成噪音。
    const conflicts = conflictingKeys(
      { poll_interval_seconds: 300 },
      { poll_interval_seconds: 600 },
      { poll_interval_seconds: 900 },
    );
    expect(conflicts).toEqual([]);
  });

  it("两边都没动 → 无冲突", () => {
    expect(
      conflictingKeys({ a: 1, b: 2 }, { a: 1, b: 2 }, { a: 1, b: 2 }),
    ).toEqual([]);
  });

  it("任一端缺基线时不报（不能凭空声称冲突）", () => {
    expect(conflictingKeys(null, { a: 1 }, { a: 2 })).toEqual([]);
    expect(conflictingKeys({ a: 1 }, null, { a: 2 })).toEqual([]);
    expect(conflictingKeys({ a: 1 }, { a: 1 }, null)).toEqual([]);
  });

  it("真实场景：账户数组被别处改动", () => {
    const baseline = { accounts: [{ instance_id: "a" }] };
    const staged = { accounts: [{ instance_id: "a" }] };
    const fresh = { accounts: [{ instance_id: "a" }, { instance_id: "b" }] };
    // 我们没动 accounts，别人加了 b；整对象写会把 b 抹掉。
    expect(conflictingKeys(baseline, staged, fresh)).toContain("accounts");
  });
});
