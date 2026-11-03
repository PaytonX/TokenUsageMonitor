// 菜单栏 popover 面板（macOS 主入口，2026-11-02 拍板「popover 一步到位」）。
// 契约 = docs/mockups/2026-11-02-macos-shell.html 场景①：环 + 最紧张窗口 +
// 账户行 + 打开主面板/设置…/退出；失焦由宿主隐藏（ipc.rs Focused(false)），
// Esc 在这里主动调 hide_popover_panel。与 peek 同款：纯 TS 拼 DOM，不挂框架。
//
// 预览：浏览器直开（无宿主）时由 dev-mock 供数据 + 垫桌面底色；生产构建
// 两段都被 tree-shake / 绝不介入（守卫见各自文件顶部说明）。
import { installDevMock } from "../lib/dev-mock";

installDevMock();

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";

import { getUsage } from "../lib/api";
import { initLocaleWithBackend, t } from "../lib/i18n/store";
import { applyPlatformAttributes } from "../lib/platform";
import { percent, type UsageSnapshot, type WindowUsage } from "../lib/types";

initLocaleWithBackend();
applyPlatformAttributes();

// 浏览器直开（无 Tauri 宿主）时垫一层「桌面」底色。真实 popover 窗口是
// 透明窗（ipc.rs transparent(true)）：面板圆角外的像素露出的是桌面；浏览器
// 里没有原生窗口这一层，透明会落到白色页面底上，读起来像「四角露白」的
// 缺陷——那只是预览假象。真 `tauri dev` 下绝不介入（同 dev-mock 的判据）。
if (!("__TAURI_INTERNALS__" in window)) {
  document.body.style.background =
    "radial-gradient(900px 600px at 18% 12%, rgba(94, 116, 183, 0.55), transparent 60%)," +
    "linear-gradient(158deg, #2c3a66 0%, #202a4d 46%, #131a30 100%)";
}

interface TightWindow {
  kind: "5h" | "daily" | "weekly" | "monthly";
  window: WindowUsage;
  pct: number;
}

/** 该账户「最紧张」的额度窗口：已用百分比最大者；quota<=0 不参与。 */
function tightest(snap: UsageSnapshot): TightWindow | null {
  const entries: Array<[TightWindow["kind"], WindowUsage | undefined]> = [
    ["5h", snap.windows.five_hour],
    ["daily", snap.windows.daily],
    ["weekly", snap.windows.weekly],
    ["monthly", snap.windows.monthly],
  ];
  const candidates = entries.filter(
    (pair): pair is [TightWindow["kind"], WindowUsage] => !!pair[1] && pair[1].quota > 0,
  );
  if (candidates.length === 0) return null;
  const [kind, best] = candidates.reduce((a, b) =>
    percent(b[1]) > percent(a[1]) ? b : a,
  );
  return { kind, window: best, pct: Math.round(percent(best) * 100) };
}

/** 占比 → 语义色（令牌）。阈值对齐卡片详情的 warn/crit 观感。 */
function toneFor(pct: number): string {
  if (pct >= 85) return "var(--tum-crit)";
  if (pct >= 60) return "var(--tum-warn)";
  return "var(--tum-ok)";
}

const RING = 64;
const STROKE = 6;
const CIRC = 2 * Math.PI * ((RING - STROKE) / 2);

function ringMarkup(pct: number, tone: string): string {
  return `<svg width="${RING}" height="${RING}" viewBox="0 0 ${RING} ${RING}" style="transform: rotate(-90deg)">
    <circle cx="${RING / 2}" cy="${RING / 2}" r="${(RING - STROKE) / 2}"
            fill="none" stroke="var(--tum-border-strong)" stroke-width="${STROKE}"/>
    <circle cx="${RING / 2}" cy="${RING / 2}" r="${(RING - STROKE) / 2}"
            fill="none" stroke="${tone}" stroke-width="${STROKE}" stroke-linecap="round"
            stroke-dasharray="${CIRC.toFixed(2)}"
            stroke-dashoffset="${(CIRC * (1 - pct / 100)).toFixed(2)}"/>
  </svg>`;
}

let snapshots: UsageSnapshot[] = [];

function render(): void {
  const tt = get(t);
  const host = document.getElementById("popover");
  if (!host) return;

  const rows = snapshots
    .map((s) => ({ snap: s, tight: tightest(s) }))
    .filter(({ tight }) => tight !== null)
    .sort((a, b) => (b.tight!.pct ?? 0) - (a.tight!.pct ?? 0));

  const top = rows[0]?.tight ?? null;
  const tone = top ? toneFor(top.pct) : "var(--tum-accent)";

  const accountRows = rows
    .map(
      ({ snap, tight }) => `
      <div class="acct-row">
        <span class="acct-dot" style="background:${toneFor(tight!.pct)}"></span>
        <div class="acct-meta">
          <div class="acct-name">${escapeHtml(snap.provider_display_name)}</div>
          <div class="acct-sub">${tt(winKindKey(tight!.kind))}</div>
        </div>
        <div class="acct-right">
          <span class="acct-pct" style="color:${toneFor(tight!.pct)}">${tight!.pct}%</span>
          <span class="acct-reset">${formatReset(tight!.window.reset_at, tt)}</span>
        </div>
      </div>`,
    )
    .join("");

  const heroMeta = top
    ? `<span class="hero-k">${tt("popover.tightest")}</span>
       <span class="hero-v"><b>${top.pct}%</b>${overTag(top.window)}</span>`
    : `<span class="hero-k">${tt("popover.noData")}</span>`;

  host.innerHTML = `
    <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html, body { height: 100%; background: transparent; overflow: hidden; }
      #popover {
        height: 100vh; display: flex; flex-direction: column;
        font-family: var(--tum-font); font-size: var(--tum-font-size-base);
        color: var(--tum-text-primary);
        background: var(--tum-glass);
        border: 1px solid var(--tum-border-strong);
        border-radius: var(--tum-radius-md);
        user-select: none; -webkit-user-select: none;
        -webkit-font-smoothing: antialiased;
      }
      .pop-head { display: flex; align-items: center; justify-content: space-between; padding: 12px 14px 4px; }
      .pop-title { font-size: 13px; font-weight: 600; }
      .icon-btn {
        width: 24px; height: 24px; border-radius: 7px; border: none; cursor: pointer;
        background: transparent; color: var(--tum-text-secondary);
        display: inline-flex; align-items: center; justify-content: center;
      }
      .icon-btn:hover { background: var(--tum-surface-hover); color: var(--tum-text-primary); }
      .pop-hero { display: flex; align-items: center; gap: 14px; padding: 6px 14px 10px; }
      .ring-wrap { position: relative; width: ${RING}px; height: ${RING}px; flex: none; }
      .ring-pct {
        position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;
        font-weight: 650; font-size: 15px; font-variant-numeric: tabular-nums;
      }
      .hero-meta { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
      .hero-k { font-size: 12px; color: var(--tum-text-secondary); }
      .hero-v { font-size: 12px; color: var(--tum-text-muted); }
      .hero-v b { color: var(--tum-text-primary); font-size: 14px; margin-right: 4px; }
      .over-tag { color: var(--tum-crit); font-size: 10.5px; }
      /* 滚动条与主窗 .shell__cards 同惯用法（--tum-border + transparent 轨道），
         但只在指针悬停列表时才显现——popover 是一瞥即走的面板，常驻滚动条
         比主窗更扎眼。标准属性管新 Chromium/WebView2，::-webkit 兜旧引擎。 */
      .pop-accounts {
        flex: 1; overflow-y: auto; padding: 0 8px; min-height: 0;
        scrollbar-width: thin; scrollbar-color: transparent transparent;
      }
      .pop-accounts:hover { scrollbar-color: var(--tum-border) transparent; }
      .pop-accounts::-webkit-scrollbar { width: 4px; }
      .pop-accounts::-webkit-scrollbar-track { background: transparent; }
      .pop-accounts::-webkit-scrollbar-thumb { background: transparent; border-radius: 2px; }
      .pop-accounts:hover::-webkit-scrollbar-thumb { background: var(--tum-border); }
      .acct-row { display: flex; align-items: center; gap: 9px; padding: 7px 8px; border-radius: 9px; }
      .acct-row:hover { background: var(--tum-surface-hover); }
      .acct-dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
      .acct-meta { min-width: 0; }
      .acct-name { font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
      .acct-sub { font-size: 10.5px; color: var(--tum-text-muted); margin-top: 1px; }
      .acct-right { margin-left: auto; display: flex; align-items: center; gap: 8px; flex: none; }
      .acct-pct { font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums; }
      .acct-reset { font-size: 10.5px; color: var(--tum-text-muted); min-width: 48px; text-align: right; }
      .pop-div { height: 1px; margin: 4px 12px; background: var(--tum-border); }
      .pop-foot { display: flex; gap: 6px; padding: 8px 10px 10px; }
      .pop-foot button {
        flex: 1; border: none; cursor: pointer; font-family: inherit;
        font-size: 12px; padding: 6px 0; border-radius: 8px;
        background: transparent; color: var(--tum-text-secondary);
      }
      .pop-foot button:hover { background: var(--tum-surface-hover); color: var(--tum-text-primary); }
      .pop-foot button.primary { color: var(--tum-accent); font-weight: 600; }
      .pop-empty { padding: 18px 14px; color: var(--tum-text-muted); font-size: 12px; text-align: center; }
      @media (prefers-reduced-motion: reduce) { * { transition: none !important; } }
    </style>
    <div class="pop-head">
      <span class="pop-title">TokenUsageMonitor</span>
      <button class="icon-btn" data-act="refresh" title="${tt("popover.refresh")}">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
             stroke-width="2.2" stroke-linecap="round"><path d="M21 12a9 9 0 1 1-2.6-6.3M21 3v6h-6"/></svg>
      </button>
    </div>
    ${top
      ? `<div class="pop-hero">
          <span class="ring-wrap">${ringMarkup(top.pct, tone)}<span class="ring-pct">${top.pct}%</span></span>
          <div class="hero-meta">${heroMeta}</div>
        </div>`
      : `<div class="pop-empty">${tt("popover.noData")}</div>`}
    ${rows.length > 0 ? `<div class="pop-accounts">${accountRows}</div>` : ""}
    <div class="pop-div"></div>
    <div class="pop-foot">
      <button class="primary" data-act="open-main">${tt("popover.openMain")}</button>
      <button data-act="settings">${tt("popover.settings")}</button>
      <button data-act="quit">${tt("popover.quit")}</button>
    </div>
  `;
}

function overTag(w: WindowUsage): string {
  const tt = get(t);
  return w.over_quota ? `<span class="over-tag">${tt("popover.overQuota")}</span>` : "";
}

function winKindKey(kind: TightWindow["kind"]): string {
  return {
    "5h": "popover.win5h",
    daily: "popover.winDaily",
    weekly: "popover.winWeekly",
    monthly: "popover.winMonthly",
  }[kind];
}

function formatReset(resetAt: string | undefined, tt: (k: string) => string): string {
  if (!resetAt) return "—";
  const secs = Math.floor((Date.parse(resetAt) - Date.now()) / 1000);
  if (!Number.isFinite(secs) || secs <= 0) return "—";
  const d = Math.floor(secs / 86400);
  if (d >= 1) return tt("popover.resetDays").replace("{n}", String(d));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h${String(m).padStart(2, "0")}m`;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c] ?? c);
}

async function refresh(): Promise<void> {
  try {
    snapshots = await getUsage();
  } catch {
    snapshots = [];
  }
  render();
}

document.getElementById("popover")?.addEventListener("click", (e) => {
  const act = (e.target as HTMLElement).closest<HTMLElement>("[data-act]")?.dataset.act;
  if (!act) return;
  if (act === "refresh") void refresh();
  if (act === "open-main") void invoke("show_dashboard_window");
  if (act === "settings") void invoke("open_settings");
  if (act === "quit") void invoke("quit_app");
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void invoke("hide_popover_panel");
});

// 语言切换镜像（设置窗改语言 → settings-changed → 各窗口刷新文案）。
t.subscribe(() => render());

void refresh();
void listen("usage-updated", () => void refresh());
