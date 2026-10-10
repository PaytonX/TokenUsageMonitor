// 模型名的归一化与归并键。
//
// ## 为什么需要这一层
//
// 同一个模型会以不同大小写被各工具写进账本：zcode 记 `GLM-5.3-Flash`，
// TokenRouter 的台账记 `glm-5.3-flash`，MiniMax Code 记 `minimax/MiniMax-M3`，
// 另一些工具记 `minimax-m3`。模型 ID 本身是大小写无关的标识符，这些是
// **同一个模型**，但按字符串直接分组就会把它们拆成并列的多条——实测 90 天
// 窗口里仅大小写不同的就有 3 组、约 24 亿 token 被对半分到不同条目。
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
 * 归并键：短名去空白后转小写。
 *
 * 只做大小写与首尾空白归一，**不做**模糊匹配——`GLM-5.3` 与
 * `GLM-5.3-Flash`、`deepseek-v4-flash` 与 `deepseek-v4-flash-260425`
 * 确实是不同模型，任何"看起来像"的合并都会造出错误条目。
 */
export function canonicalModelKey(short: string): string {
  return short.trim().toLowerCase();
}

/**
 * 一组仅大小写不同的写法里，选出展示名。
 *
 * 取**用量最大**的那个写法（并列时取字典序最小），理由：它是用户平时在
 * 工具里看到的主写法，比强行统一成全小写更贴近心智；且规则与遍历顺序
 * 无关，重算结果稳定。并列取字典序是为了保证同输入同输出。
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
  return best;
}
