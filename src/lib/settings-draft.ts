/**
 * 设置草稿的基线比对（方案 .scratch/glance-bench/issues/02b）。
 *
 * ## 先纠正一个此前的错误判断
 *
 * 我曾断言「我们的设置是改一下存一下，改完即生效，所以预览不需要草稿态」。
 * **那是错的。** `Settings.svelte` 里 `persistSettings()` 只有两个调用点：
 * 「保存设置」按钮与「保存并关闭」按钮，没有任何 `$effect`/watch 自动落盘。
 * 也就是说——**暂存（草稿）一直存在**，一直都要点按钮才写盘。
 *
 * 真正缺的不是草稿，是**丢弃**：关闭走的是 `handleClose`，它无条件调用
 * `persistSettings()`。所以「改了不想要」这件事目前无路可退——用户只能手动
 * 把自己刚才改的每一项改回去，或者认栽。
 *
 * ## 为什么还需要冲突检测
 *
 * `persistSettings()` 是**整对象写**（把散落在十几个 `$state` 里的字段拼成一个
 * `Settings` 再 `saveSettings(next)`），不是按 key 增量。而设置窗口是 hide 而非
 * destroy：托盘、`onSettingsChanged` 触发的其他路径都可能在这份暂存之外改过盘。
 * 整对象写会把那些改动一并抹掉，且没有任何提示。
 *
 * `settings_delta.rs` 的 `SettingsDelta::compute(old, new)` 是「算差分以决定要不要
 * 触发副作用（重跑轮询之类）」，**不是**部分写入保护，拿它当防线是错的。
 *
 * 这里的做法：提交前先读回后端当前值，与「打开时的基线」逐 key 比对，
 * 只对**我们没有改动、但别人改了**的 key 报冲突——那些才是会被静默覆盖的。
 */

/**
 * 键序无关的深比较。
 *
 * ⚠️ 不要退回 `JSON.stringify(a) !== JSON.stringify(b)`。
 * 那样比**对键序敏感**：baseline 里的嵌套对象来自 Rust 反序列化（字段序由
 * Rust 侧决定），而 buildNext() 重建的那些由 JS 构造序决定。语义相同的两份
 * 值只要键序不同就会被判为「改过」——后果是打开设置就恒脏（撤销按钮永远可点），
 * 更糟的是这些伪差异会进入 `weTouched`，把别人对这些键的**真实**改动过滤掉，
 * 冲突检测静默失效且不报错。
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    // NaN 与 NaN 在此判为不等；配置里不存在 NaN，可接受。
    return false;
  }
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = Object.keys(ao);
  if (keys.length !== Object.keys(bo).length) return false;
  return keys.every((k) => Object.prototype.hasOwnProperty.call(bo, k) && deepEqual(ao[k], bo[k]));
}

/** 一层 key 的粗粒度比对。嵌套对象/数组按键序无关地深比较。 */
export function changedKeys(baseline: unknown, current: unknown): string[] {
  if (baseline === null || current === null) return [];
  const a = baseline as Record<string, unknown>;
  const b = current as Record<string, unknown>;
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  const out: string[] = [];
  for (const k of keys) {
    if (!deepEqual(a[k], b[k])) out.push(k);
  }
  return out;
}

/** 暂存相对基线是否变了。 */
export function isDirty(baseline: unknown, staged: unknown): boolean {
  return changedKeys(baseline, staged).length > 0;
}

/**
 * 会被我们静默覆盖掉的他人改动。
 *
 * 返回那些满足三个条件的 key：
 * 1. 打开设置窗之后，别人把它改了（fresh ≠ baseline）
 * 2. 我们自己没动它（staged ≡ baseline）
 * 3. 我们仍会把整个对象写回去（提交动作本身就会覆盖它）
 *
 * 条件 2 是关键：我们自己改过的 key 即使别人也改了，那也是「我们正在决定的事」，
 * 按后写为准，不算**静默**覆盖——但它仍会被覆盖，所以上面那条叫法只对
 * 「别人单独改的」成立，见调用方的两段式提交。
 */
export function conflictingKeys(
  baseline: Record<string, unknown> | null,
  staged: Record<string, unknown> | null,
  fresh: Record<string, unknown> | null,
): string[] {
  if (!baseline || !staged || !fresh) return [];
  const othersMoved = changedKeys(baseline, fresh);
  const weTouched = new Set(changedKeys(baseline, staged));
  return othersMoved.filter((k) => !weTouched.has(k));
}

/**
 * 把后端快照整形成「与 buildNext() 可比」的形状。
 *
 * `buildNext()` 里 `hub_token_configured: true` 是硬编码的（含义是「用户已经对
 * 这个 secret 表过态」，清空也算表态）。若后端初值是 false 而我们不把基线同步
 * 整成 true，打开设置那一刻就会恒脏——撤销按钮永远可点，看起来像有未保存改动。
 */
export function normalizeBaseline(s: Record<string, unknown>): Record<string, unknown> {
  return { ...s, hub_token_configured: true };
}
