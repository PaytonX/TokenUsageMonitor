// 贴边把手（peek）小窗：左/右边 7×58 玻璃条、上/下边 58×7 + 2×22 握柄。
// 职责只有两件：hover 唤醒主胶囊、收到 peek-show 后复现自己。不持有业务数据。
import { emit, listen } from "@tauri-apps/api/event";

type Side = "left" | "right" | "top" | "bottom";

declare global {
  interface Window {
    /** 由 Rust 建窗时的 initialization_script 注入。 */
    __PEEK_SIDE__?: Side;
  }
}

const HOVER_DEBOUNCE_MS = 60;
const side: Side = window.__PEEK_SIDE__ ?? "right";

const host = document.getElementById("peek");
if (host) {
  host.className = "peek is-hidden";
  host.dataset.side = side;
  host.innerHTML = '<span class="grip"></span>';

  const style = document.createElement("style");
  style.textContent = `
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body { height: 100%; overflow: hidden; background: transparent; }
    .peek {
      /* ⚠️ 必须填满整个窗口，不能写死 7px / 58px。
         Windows 会把窗口撑到最小尺寸（横向 ~136px），写死尺寸会让画出来的条
         落在窗口的固定一端，贴左边时整条被推到屏幕外 —— 表现为「贴左边没有
         小把手」。填满窗口后「把手可见条 = 屏幕边缘那 thickness px」由 Rust 的
         几何唯一决定，CSS 不再需要知道窗口被撑成多大。 */
      width: 100%;
      height: 100%;
      display: flex;
      align-items: center;
      /* 与 --tum-glass 保持一致：透明窗里 backdrop-filter 采不到桌面，
         只用高不透明度深色保证对比度。 */
      background: rgba(22, 25, 31, 0.92);
      transition:
        opacity 160ms cubic-bezier(0.33, 1, 0.68, 1),
        transform 160ms cubic-bezier(0.33, 1, 0.68, 1);
      cursor: default;
    }
    /* 对齐到「屏内可见的那一端」（见 dock.rs::peek_rect 的表）。
       水平边：主轴是水平。竖直边：主轴变成垂直。 */
    .peek[data-side="right"]  { flex-direction: row; justify-content: flex-start; transform-origin: left center; }
    .peek[data-side="left"]   { flex-direction: row; justify-content: flex-end;   transform-origin: right center; }
    .peek[data-side="bottom"] { flex-direction: column; align-items: flex-start; transform-origin: center top; }
    .peek[data-side="top"]    { flex-direction: column; align-items: flex-end;   transform-origin: center bottom; }

    /* 圆角落在靠屏内的那一端，贴边那侧保持直角。 */
    .peek[data-side="right"]  { border-radius: 4px 0 0 4px; border-right: none; }
    .peek[data-side="left"]   { border-radius: 0 4px 4px 0; border-left: none; }
    .peek[data-side="bottom"] { border-radius: 4px 4px 0 0; border-bottom: none; }
    .peek[data-side="top"]    { border-radius: 0 0 4px 4px; border-top: none; }

    .grip { flex: none; background: rgba(255, 255, 255, 0.55); transition: transform 110ms ease; }
    .peek[data-side="right"] .grip, .peek[data-side="left"] .grip {
      width: 2px; height: 22px; border-radius: 2px;
    }
    .peek[data-side="bottom"] .grip, .peek[data-side="top"] .grip {
      width: 22px; height: 2px; border-radius: 2px;
    }
    .peek[data-side="right"]:hover .grip,
    .peek[data-side="left"]:hover .grip { transform: scaleY(1.3); }
    .peek[data-side="bottom"]:hover .grip,
    .peek[data-side="top"]:hover .grip { transform: scaleX(1.3); }

    /* 只做视觉隐藏：绝不在这里改 pointer-events —— 在指针仍位于把手上时翻转它，
       会让浏览器改判 hover 目标并派发一次假的 pointerleave，于是主窗收到
       peek-leave、刚滑入的胶囊 320ms 后就被收回（表现为「弹出即收回」）。
       把手能否接收鼠标由 Rust 的 set_ignore_cursor_events 在窗口级托管。 */
    .peek.is-hidden { opacity: 0; }
    .peek[data-side="right"].is-hidden,
    .peek[data-side="left"].is-hidden { transform: scaleX(0.4); }
    .peek[data-side="bottom"].is-hidden,
    .peek[data-side="top"].is-hidden { transform: scaleY(0.4); }
    @media (prefers-reduced-motion: reduce) {
      .peek, .grip { transition-duration: 0.001ms; }
    }
  `;
  document.head.appendChild(style);

  let debounceTimer: number | null = null;

  host.addEventListener("pointerenter", () => {
    if (debounceTimer !== null) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => {
      debounceTimer = null;
      // ⚠️ 只能在 hover 确认后才隐去把手。若在 pointerenter 里立刻加 is-hidden，
      // 该类的 pointer-events:none 会让浏览器在指针仍位于把手上时改判 hover
      // 目标并派发一次 pointerleave —— 于是这个 60ms 定时器被下面的 leave
      // 处理器清掉，peek-hover 永不发出，表现为「hover 老是不生效」。
      host.classList.add("is-hidden");
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
    if (next === "left" || next === "right" || next === "top" || next === "bottom") {
      host.dataset.side = next;
    }
    host.classList.remove("is-hidden");
  });
}
