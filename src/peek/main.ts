// 贴边把手（peek）小窗：7×58 玻璃条 + 2×22 握柄。
// 职责只有两件：hover 唤醒主胶囊、收到 peek-show 后复现自己。不持有业务数据。
import { emit, listen } from "@tauri-apps/api/event";

type Side = "left" | "right";

declare global {
  interface Window {
    /** 由 Rust 建窗时的 initialization_script 注入（"left" | "right"）。 */
    __PEEK_SIDE__?: Side;
  }
}

const HOVER_DEBOUNCE_MS = 60;
const side: Side = window.__PEEK_SIDE__ === "left" ? "left" : "right";

const host = document.getElementById("peek");
if (host) {
  host.className = "peek";
  host.dataset.side = side;
  host.innerHTML = '<span class="grip"></span>';

  const style = document.createElement("style");
  style.textContent = `
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body { height: 100%; overflow: hidden; background: transparent; }
    .peek {
      width: 7px;
      height: 58px;
      display: flex;
      align-items: center;
      justify-content: center;
      background: rgba(48, 52, 56, 0.68);
      backdrop-filter: blur(32px) saturate(115%);
      -webkit-backdrop-filter: blur(32px) saturate(115%);
      border: 1px solid rgba(255, 255, 255, 0.16);
      transition: opacity 110ms ease;
      cursor: default;
    }
    .peek[data-side="right"] { border-radius: 4px 0 0 4px; border-right: none; }
    .peek[data-side="left"] { border-radius: 0 4px 4px 0; border-left: none; }
    .grip {
      width: 2px;
      height: 22px;
      border-radius: 2px;
      background: rgba(255, 255, 255, 0.55);
      transform: scaleY(1);
      transition: transform 110ms ease;
    }
    .peek:hover .grip { transform: scaleY(1.3); }
    .peek.is-hidden { opacity: 0; pointer-events: none; }
    @media (prefers-reduced-motion: reduce) {
      .peek, .grip { transition-duration: 0.001ms; }
    }
  `;
  document.head.appendChild(style);

  let debounceTimer: number | null = null;

  host.addEventListener("pointerenter", () => {
    // 立刻隐去把手（110ms 淡出），胶囊随后从窗外滑入盖住这里。
    host.classList.add("is-hidden");
    if (debounceTimer !== null) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => {
      debounceTimer = null;
      void emit("peek-hover");
    }, HOVER_DEBOUNCE_MS);
  });

  host.addEventListener("pointerleave", () => {
    if (debounceTimer !== null) {
      window.clearTimeout(debounceTimer);
      debounceTimer = null;
    }
    void emit("peek-leave");
  });

  // 胶囊滑出完成后由主窗通知：把手复现。payload 是该时刻胶囊贴靠的边——
  // `__PEEK_SIDE__` 只在建窗时注入过一次，用户把胶囊拖到屏幕另一侧后必须
  // 靠这里纠正 4px 圆角朝向（拖动期间把手处于 is-hidden，不会被看到）。
  void listen("peek-show", (event) => {
    const next = event.payload as Side | undefined;
    if (next === "left" || next === "right") host.dataset.side = next;
    host.classList.remove("is-hidden");
  });
}
