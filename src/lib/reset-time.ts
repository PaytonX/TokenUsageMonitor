/**
 * 胶囊窗口明细里的「距重置还有多久」。
 *
 * 只做纯计算、不读翻译：调用方套 i18n 文案。与 `ResetCountdown` 的显示口径
 * 保持一致——超过一天用 `d h`，一天内用 `h m`，最后一小时用 `m`。秒级只在
 * 不足一分钟时出现，因为胶囊展开区空间有限，`2h 10m 33s` 读起来太吵。
 */

/** `resetAt`（ISO8601）距 `now`（epoch ms）的剩余时间；缺失/非法返回 `—`。 */
export function formatResetIn(resetAt: string | undefined | null, now: number): string {
  if (!resetAt) return "—";
  const target = new Date(resetAt).getTime();
  if (!Number.isFinite(target)) return "—";
  const ms = target - now;
  if (!Number.isFinite(ms) || ms <= 0) return "0m";
  const totalSec = Math.floor(ms / 1000);
  const days = Math.floor(totalSec / 86_400);
  const hours = Math.floor((totalSec % 86_400) / 3_600);
  const mins = Math.floor((totalSec % 3_600) / 60);
  const secs = totalSec % 60;
  if (days > 0) return `${days}d ${String(hours).padStart(2, "0")}h`;
  if (hours > 0) return `${hours}h ${String(mins).padStart(2, "0")}m`;
  if (mins > 0) return `${mins}m`;
  return `${secs}s`;
}
