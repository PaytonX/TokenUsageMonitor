/**
 * 平台标识（Rust `platform.rs` 的前端镜像）。
 *
 * 值由宿主在建窗时经 initialization_script 注入（`window.__TUM_PLATFORM__`），
 * 同步可用、零 IPC 往返——浅色令牌的作用域与二级窗口的红绿灯避让都依赖它
 * 在**首帧之前**就位，不能走异步 invoke。浏览器直开 / dev-mock 等无宿主
 * 环境按 "windows" 兜底，与项目既有的默认行为一致。
 */

export type PlatformId = "macos" | "windows";

declare global {
  interface Window {
    /** 由 Rust 建窗的 initialization_script 注入；缺省视为 windows。 */
    __TUM_PLATFORM__?: string;
  }
}

export function platformId(): PlatformId {
  return window.__TUM_PLATFORM__ === "macos" ? "macos" : "windows";
}

/**
 * 把平台标识挂到 `<html data-platform>`。styles/tokens.css 的 macOS 分支
 * （浅色档、二级窗口标题栏避让）以该属性为作用域；每个窗口入口在 mount
 * 前调用一次。
 */
export function applyPlatformAttributes(): void {
  document.documentElement.dataset.platform = platformId();
}
