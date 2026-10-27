// 模型名的归一化与归并键。
//
// ## 为什么需要这一层
//
// 同一个模型会以不同大小写、不同前缀被各工具写进账本：zcode 记
// `GLM-5.3-Flash`，TokenRouter 的台账记 `glm-5.3-flash`，MiniMax Code 记
// `minimax/MiniMax-M3.1-Flash`，另一些工具记 `MiniMax-M3.1-Flash`。模型 ID
// 本身是大小写无关的标识符，`provider/` 前缀只是**记账方**的写法而不是模型
// 身份的一部分，这些都是**同一个模型**，但按字符串直接分组就会把它们拆成
// 并列的多条——实测账本里 `minimax/MiniMax-M3.1-Flash`（35.5 亿）与
// `MiniMax-M3.1-Flash`（0.9 亿）被拆成两条，排名因此不可信。
//
// ## 归并与 Provider 归因是两条正交的轴
//
// 模型名回答"用的是哪个模型"，Provider 回答"谁出的钱"。同一个
// `glm-5.3-flash` 经火山方舟账户调用时，Provider 应记火山（见
// `model-provider.ts` 的 `providerForRoutedSlice`），而模型**仍然**是
// glm-5.3-flash —— 不能因为 Provider 不同就当成两个模型，也不能因为模型
// 归并了就抹掉账户维度。

/** 剥掉 `provider/model` 前缀，只留模型短名（`minimax/MiniMax-M3` → `MiniMax-M3`）。 */
export function stripModelPrefix(raw: string): string {
  const idx = raw.lastIndexOf("/");
  return idx >= 0 ? raw.slice(idx + 1) : raw;
}

/**
 * 归并键：先剥 `provider/` 前缀，再去首尾空白转小写。
 *
 * 只做「前缀 + 大小写」两类归一，**不做**模糊匹配——`GLM-5.3` 与
 * `GLM-5.3-Flash`、`deepseek-v4-flash` 与 `deepseek-v4-flash-260425`、
 * `MiniMax-M3` 与 `MiniMax-M3.1-Flash`、`DeepSeek V4 Flash` 与
 * `DeepSeek V4.1 Flash` 确实是不同模型，任何"看起来像"的合并都会造出
 * 错误条目（少算一个模型、多算另一个模型的钱）。
 *
 * 前缀归一放在这里而不是只放在调用方：调用方既有传原始名（`device-agg.ts`
 * 直接用账本/设备上报的 `model` 字段）也有传短名的，把剥前缀做成归并键
 * 的一部分，两条路径才不会得出不同的分组结果。
 */
export function canonicalModelKey(raw: string): string {
  return stripModelPrefix(raw).trim().toLowerCase();
}

/**
 * 是否是「路由占位名」而非真实模型。
 *
 * 两类都来自路由自选模型，模型维度上没有信息量，不该占真模型的排名位：
 * - `__via_router__`：后端哨兵（`src-tauri/src/local/mod.rs` 的 `ROUTED_MODEL`），
 *   工具只知道请求了"路由"，真实承载模型由 TokenRouter 台账另行给出；
 * - `auto`：zcode / deepseek-harness（以及 Cherry Studio 的 `Auto`）在让路由
 *   自己挑模型时记下的占位值，实测近 30 天 1.23 亿 token 全落在这个桶里。
 *
 * `未标记模型`（UNCLASSIFIED_MODEL）**不在此列**：那是"上游没记模型"的未知态，
 * 与"走了路由"是两回事，仍按普通条目展示。
 */
export function isPlaceholderModelName(raw: string): boolean {
  const key = canonicalModelKey(raw);
  return key === "__via_router__" || key === "auto";
}

/**
 * 一组同一模型的不同写法里，选出展示名。
 *
 * 取**用量最大**的那个写法（并列时取字典序最小），理由：它是用户平时在
 * 工具里看到的主写法，比强行统一成全小写更贴近心智；且规则与遍历顺序
 * 无关，重算结果稳定。并列取字典序是为了保证同输入同输出。
 *
 * 返回值一律剥掉 `provider/` 前缀：前缀是记账方的叫法，展示层用它只会让
 * 同一个模型在不同范围里换名字（全端汇总的 variants 存的是原始名）。
 */
export function dominantDisplayName(variants: Map<string, number>): string {
  let best = "";
  let bestTotal = -1;
  for (const [name, total] of variants) {
    if (total > bestTotal || (total === bestTotal && name < best)) {
      best = name;
      bestTotal = total;
    }
  }
  return stripModelPrefix(best);
}
