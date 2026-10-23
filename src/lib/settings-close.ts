/** 关闭设置窗口时该做什么（由 Settings.svelte 的 requestClose 消费）。 */
export type CloseAction = "open-confirm" | "close";

/**
 * 有未保存草稿且确认条还没弹出 → 弹条；其余情况一律直接关。
 *
 * 这条规则是「无草稿点 ✕ 不该出现二次确认」的守卫，抽出来单独钉住：
 * 它一旦被写成 `hasDraft || confirming`、或把 confirming 当成必要条件，
 * 用户每次关窗都会被多问一句。
 */
export function resolveCloseAction(input: {
  hasDraft: boolean;
  confirming: boolean;
}): CloseAction {
  return input.hasDraft && !input.confirming ? "open-confirm" : "close";
}
