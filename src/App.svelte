<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import { fly } from "svelte/transition";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    getUsage,
    getProviderStates,
    getSettings,
    onUsageUpdated,
    onProviderError,
    onSettingsChanged,
    setWindowMode,
    syncPeekWindow,
    emitPeekShow,
    onPeekHover,
    onPeekLeave,
    onPeekReveal,
    forceRefresh,
    openSettings,
    onTabsChanged,
    readHiddenTabs,
    readCardOrder,
    writeCardOrder,
    type PageTab,
    type UsageSnapshot,
    type ProviderState,
    type ProviderError,
    type Settings,
    type BurnInfo,
    type PeekSide,
    ringWindowRemaining,
    providerShortName,
    payAsYouGoLabel,
    isPayAsYouGo,
    hexToRgb,
  } from "./lib";
  import ProviderCard from "./lib/components/ProviderCard.svelte";
  import ProgressRing from "./lib/components/ProgressRing.svelte";
  import HeatmapGrid from "./lib/components/HeatmapGrid.svelte";
  import MiniPanel from "./lib/components/MiniPanel.svelte";
  import DetailCard from "./lib/components/DetailCard.svelte";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";
  import TrendPanel from "./lib/components/TrendPanel.svelte";
  import ToolPanel from "./lib/components/ToolPanel.svelte";
  import ModelPanel from "./lib/components/ModelPanel.svelte";
  import DevicePanel from "./lib/components/DevicePanel.svelte";
  import PillsOrSelect from "./lib/components/PillsOrSelect.svelte";
  import { brandColorFor, EXPERIMENTAL_KINDS } from "./lib/brand-glyphs";

  type Mode = "dashboard" | "compact";

  // C7/C8 多视图：dashboard 在"总量/趋势/工具"间切换；模型/设备依赖本地采集
  // 扩展与多端同步(B8)，暂以占位项呈现。
  type ViewMode = "overview" | "trend" | "tools" | "models" | "devices";
  const VIEW_KEY = "tum.view";
  let view = $state<ViewMode>(
    localStorage.getItem(VIEW_KEY) === "trend"
      ? "trend"
      : localStorage.getItem(VIEW_KEY) === "tools"
        ? "tools"
        : localStorage.getItem(VIEW_KEY) === "models"
          ? "models"
          : localStorage.getItem(VIEW_KEY) === "devices"
            ? "devices"
            : "overview",
  );
  $effect(() => {
    localStorage.setItem(VIEW_KEY, view);
  });

  // 页签显隐：总量恒常驻，趋势/工具/模型由设置窗口开关控制（tabs-changed 事件
  // 触发重读）。关闭的页签不出现在标签行，也不渲染其组件（功能不启用）。
  let tabsRev = $state(0);
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void onTabsChanged(() => {
      if (!disposed) tabsRev++;
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
  const extraTabs = $derived.by(() => {
    void tabsRev;
    const hidden = new Set<PageTab>(readHiddenTabs());
    const defs: { key: PageTab; label: string }[] = [
      { key: "trend", label: "趋势" },
      { key: "tools", label: "工具" },
      { key: "models", label: "模型" },
      { key: "devices", label: "设备" },
    ];
    return defs
      .filter((t) => !hidden.has(t.key))
      .map((t) => ({ key: t.key as ViewMode, label: t.label }));
  });
  const tabs = $derived([
    { key: "overview", label: "总量" },
    ...extraTabs,
  ] as { key: ViewMode; label: string }[]);
  // 当前页签被禁用时回落总量，避免停留在未渲染的页签上。
  $effect(() => {
    if (view !== "overview" && !extraTabs.some((t) => t.key === view)) {
      view = "overview";
    }
  });

  let snapshots = $state<UsageSnapshot[]>([]);
  let errors = $state<Record<string, string>>({});
  let burns = $state<Record<string, BurnInfo | null>>({});
  let actives = $state<Record<string, boolean>>({});
  let mode = $state<Mode>("dashboard");
  let heatmapTabId = $state<string | null>(null);
  const FOCUS_FALLBACK_COLOR = "#8a8f98";
  const FOCUS_KEY = "tum.focus";
  const HEATMAP_VIEW_KEY = "tum.heatmapView";

  // 热力图时间范围："compact" = 近 31 天 5×7 网格；"calendar" = GitHub 式
  // 近 6 个月日历（自然月标注 + 累计/峰值/活跃统计）。默认日历——滚动
  // 31 天窗口不直观，按自然月看当月与过去数月更符合直觉。
  let heatmapView = $state<"compact" | "calendar">(
    localStorage.getItem(HEATMAP_VIEW_KEY) === "compact" ? "compact" : "calendar",
  );
  $effect(() => {
    localStorage.setItem(HEATMAP_VIEW_KEY, heatmapView);
  });

  // Global focus: "all" (aggregate min) or one provider_id. Drives the
  // header ring, the chips row and the heatmap panel.
  let focus = $state(localStorage.getItem(FOCUS_KEY) ?? "all");
  // Provider whose card the pointer is currently over; drives the floating
  // detail overlay. `anchorRect` is the trigger card's viewport box so the
  // overlay can park right beside it instead of docking at window bottom.
  let hoveredId = $state<string | null>(null);
  let anchorRect = $state<DOMRect | null>(null);
  // Overlay box is measured before reveal so placement uses real dimensions
  // (content height varies per provider). Stays hidden until first measure.
  let overlayEl = $state<HTMLElement | null>(null);
  let measuredW = $state(0);
  let measuredH = $state(0);
  let overlayX = $state(0);
  let overlayY = $state(0);
  let overlayReady = $state(false);
  const OVERLAY_GAP = 8;
  const VIEWPORT_MARGIN = 8;

  // Hiding the detail overlay is deferred by a grace window so the pointer
  // can travel from the card into the overlay (or back) without it vanishing
  // mid-move. Entering the overlay cancels the pending hide (pinning it);
  // leaving it re-arms a hide so it closes when you go elsewhere.
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleOverlayHide() {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      hoveredId = null;
      anchorRect = null;
      hideTimer = null;
    }, 350);
  }
  function cancelOverlayHide() {
    if (hideTimer) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
  }
  function onCardHover(id: string, hovering: boolean, rect?: DOMRect) {
    if (hovering) {
      cancelOverlayHide();
      hoveredId = id;
      anchorRect = rect ?? null;
      // Hide until the ResizeObserver re-measures at the new anchor — avoids
      // a one-frame ghost at the previous card's coordinates.
      overlayReady = false;
    } else if (hoveredId === id) {
      scheduleOverlayHide();
    }
  }

  // Compute fixed coordinates for the overlay from the trigger rect and the
  // measured content box: clamp horizontally; prefer below, flip above when
  // the bottom edge would overflow; clamp to viewport as the last resort.
  function placeOverlay() {
    const rect = anchorRect;
    if (!rect) return;
    const w = measuredW || 320;
    const h = measuredH;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const maxX = Math.max(VIEWPORT_MARGIN, vw - w - VIEWPORT_MARGIN);
    overlayX = Math.min(Math.max(rect.left, VIEWPORT_MARGIN), maxX);
    const below = rect.bottom + OVERLAY_GAP;
    const above = rect.top - OVERLAY_GAP - h;
    if (h <= 0 || below + h <= vh - VIEWPORT_MARGIN) {
      overlayY = below;
    } else if (above >= VIEWPORT_MARGIN) {
      overlayY = above;
    } else {
      overlayY = Math.max(VIEWPORT_MARGIN, vh - h - VIEWPORT_MARGIN);
    }
  }

  // Measure the overlay content box whenever it mounts or its size changes.
  // ResizeObserver callbacks run before paint, so reveal-after-measure has
  // no visible flash. The initial synchronous apply covers same-frame mount.
  // Re-run when the displayed snapshot changes, not only when the element
  // mounts: crossing between two equal-height cards does not resize the box,
  // so ResizeObserver would not fire and the overlay could stay hidden.
  // $effect runs after the DOM patch, so the synchronous apply() below
  // measures the new content and reveals it without waiting for a resize.
  $effect(() => {
    void detailSnapshot;
    const el = overlayEl;
    if (!el) return;
    const apply = () => {
      const box = el.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) {
        measuredW = box.width;
        measuredH = box.height;
        overlayReady = true;
      }
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Reposition whenever the anchor moves or the content box changes size.
  $effect(() => {
    void anchorRect;
    void measuredW;
    void measuredH;
    placeOverlay();
  });

  // 总量页卡片拖拽排序：snapshots 保持后端到达顺序（usage-updated 刷新不打断
  // 用户排序），展示顺序由 cardOrder 偏好重排；拖拽经过时实时让位（dragOrder
  // 临时序列 + flip 动画），dragend 提交并持久化，未拖成则原样还原。
  let cardOrder = $state<string[]>(readCardOrder());
  let draggingId = $state<string | null>(null);
  let dragOrder = $state<string[]>([]);
  const orderedSnapshots = $derived.by(() => {
    const order = draggingId !== null ? dragOrder : cardOrder;
    if (order.length === 0) return snapshots;
    const pos = new Map(order.map((id, i) => [id, i]));
    const known = snapshots
      .filter((s) => pos.has(s.provider_id))
      .sort(
        (a, b) => (pos.get(a.provider_id) ?? 0) - (pos.get(b.provider_id) ?? 0),
      );
    const fresh = snapshots.filter((s) => !pos.has(s.provider_id));
    return [...known, ...fresh];
  });
  function onCardDragStart(e: DragEvent, id: string) {
    draggingId = id;
    dragOrder = orderedSnapshots.map((s) => s.provider_id);
    e.dataTransfer?.setData("text/plain", id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function onCardDragOver(e: DragEvent, targetId: string) {
    e.preventDefault();
    if (!draggingId || targetId === draggingId) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const before = e.clientY < rect.top + rect.height / 2;
    if (dragOrder.indexOf(draggingId) < 0 || dragOrder.indexOf(targetId) < 0)
      return;
    const next = dragOrder.filter((id) => id !== draggingId);
    const at = next.indexOf(targetId);
    next.splice(before ? at : at + 1, 0, draggingId);
    dragOrder = next;
  }
  function onCardDragEnd() {
    if (draggingId === null) return;
    if (dragOrder.length > 0) {
      cardOrder = dragOrder;
      writeCardOrder(cardOrder);
    }
    draggingId = null;
    dragOrder = [];
  }
  const PILL_DRAG_THRESHOLD_PX = 4;

  // ---- 贴边 peek 双窗（spec §2 决策 1-5 / §3.2 时序）----
  const PILL_COLLAPSED_W = 168; // 与 Rust set_window_mode 的 LogicalSize 保持一致
  const PILL_COLLAPSED_H = 56;
  const PILL_MINI_PAD_TOP = 2;
  const PILL_MINI_PAD_BOTTOM = 8;
  const PILL_MINI_ROW = 24;
  const PILL_MINI_GAP = 6;
  // 展开高度 = 56 + (2 + 30n + 6(n-1) + 8) = 60 + 30n；同 MiniPanel 的 CSS。
  const PILL_EXPANDED_ROW = PILL_MINI_ROW + PILL_MINI_GAP; // 30
  const PILL_EXPANDED_BASE =
    PILL_COLLAPSED_H + PILL_MINI_PAD_TOP + PILL_MINI_PAD_BOTTOM - PILL_MINI_GAP; // 60
  const PILL_SLIDE_MS = 150; // 与 .pill-layer 的 transition 时长一致
  const PILL_PEEK_BACK_MS = 90; // 滑出完成后把手复现的延迟
  const PILL_REVEAL_MINI_MS = 120; // 滑入开始后展开明细的延迟
  const PILL_DOCK_DELAY_MS = 320; // 指针离开后停留多久才收起
  // 模式恢复判定阈值（逻辑像素）：介于 compact(168) 与 dashboard(400) 之间。
  const PILL_RESTORE_MAX_W = 200;
  let settings = $state<Settings | null>(null);

  // Per-account accent map (instance_id -> #RRGGBB), derived from settings.
  // Snapshot `provider_id` doubles as the account `instance_id`, so lookup by it.
  let accentById = $derived(
    (settings?.accounts ?? []).reduce(
      (m, a) => {
        m[a.instance_id] = a.accent_color;
        return m;
      },
      {} as Record<string, string>,
    ),
  );
  // 解析某个 provider 的强调色：账户自定义色优先，否则用品牌注册表的品牌色。
  const colorOf = (id: string) =>
    accentById[id] ?? brandColorFor(id.split("-")[0]);

  // Countdown mode: false = show USED, true = show REMAINING (1 - used).
  let displayRemaining = $derived(settings?.countdown_mode ?? false);

  let now = $state(new Date());
  // Wall-clock at the moment we last received an `usage-updated` (or did a
  // manual refresh). Drives the "next refresh in Ns" countdown in the footer.
  let lastRefreshAt = $state<number>(Date.now());
  let unlistenFns: Array<() => void> = [];

  onMount(async () => {
    // 并行拉取用量 + 状态 + 设置，减少启动等待。
    const [usage, states] = await Promise.all([
      getUsage(),
      getProviderStates(),
    ]);
    snapshots = usage;
    errors = extractErrors(states);
    settings = await getSettings();
    lastRefreshAt = Date.now();

    if (snapshots.length > 0 && !heatmapTabId) {
      heatmapTabId = snapshots[0].provider_id;
    }

    // Live updates.
    unlistenFns.push(
      await onUsageUpdated((update) => {
        const snap = update.snapshot;
        snapshots = snapshots
          .filter((s) => s.provider_id !== snap.provider_id)
          .concat(snap)
          .sort((a, b) => a.provider_id.localeCompare(b.provider_id));
        burns = { ...burns, [snap.provider_id]: update.burn };
        actives = { ...actives, [snap.provider_id]: update.active };
        if (!heatmapTabId) heatmapTabId = snap.provider_id;
        errors = { ...errors, [snap.provider_id]: "" };
        lastRefreshAt = Date.now();
      }),
    );

    unlistenFns.push(
      await onProviderError(({ id, error }) => {
        errors = { ...errors, [id]: formatError(error) };
        // An error still represents a poll attempt — reset the countdown so
        // users see "next refresh in 5m" rather than a stale negative number.
        lastRefreshAt = Date.now();
      }),
    );

    // Settings saved (from the Settings window): the backend has already
    // dropped disabled providers from its cache and stopped their pollers,
    // so re-pulling usage here removes their cards immediately.
    unlistenFns.push(
      await onSettingsChanged(async (next) => {
        settings = next;
        snapshots = await getUsage();
        const states = await getProviderStates();
        errors = extractErrors(states);
        lastRefreshAt = Date.now();
      }),
    );

    unlistenFns.push(await onPeekHover(() => void revealPill()));
    unlistenFns.push(await onPeekLeave(() => scheduleDockPill()));
    unlistenFns.push(await onPeekReveal(() => void revealPill()));

    // 前端可能刚被重载（HMR / WebView 崩溃恢复）：Rust 侧的窗口尺寸与 compact 标志
    // 都还在，但这里的 mode 会回到初始值 —— 若不同步，就会在 168x56 的窗里渲染
    // dashboard 视图（内容被裁掉、头部按钮落在窗外，用户自己点不回来）。
    // 因此按实际窗口宽度对齐模式，并重新同步把手与鼠标捕获。
    try {
      const win = getCurrentWindow();
      const [size, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
      if (size.width / scale <= PILL_RESTORE_MAX_W) {
        mode = "compact";
        await syncPeek();
      }
    } catch {
      // 取不到窗口尺寸：维持默认的 dashboard 模式。
    }

    // 1-second clock tick for the timestamp header.
    const tick = setInterval(() => (now = new Date()), 1000);
    unlistenFns.push(() => clearInterval(tick));
  });

  // Aggregate progress ring shows the worst remaining % across all providers,
  // resolved through the user's `ring_window` setting (auto = most critical).
  let ringWindow = $derived((settings?.ring_window ?? "auto") || "auto");
  let aggregateRemaining = $derived(
    snapshots.length === 0
      ? 0
      : Math.min(...snapshots.map((s) => ringWindowRemaining(s, ringWindow))),
  );
  let aggregateLabel = $derived(
    snapshots.length === 0
      ? "--"
      : `${Math.round(aggregateRemaining * 100)}%`,
  );

  // Snapshot behind the global focus ("all" → aggregate; one provider → it).
  let focusedSnapshot = $derived(
    focus === "all"
      ? null
      : (snapshots.find((s) => s.provider_id === focus) ?? null),
  );
  let ringPercent = $derived(
    focusedSnapshot
      ? ringWindowRemaining(focusedSnapshot, ringWindow)
      : aggregateRemaining,
  );
  let ringLabel = $derived(
    focusedSnapshot
      ? `${Math.round(ringPercent * 100)}%`
      : aggregateLabel,
  );

  // Ring arc/label mirror `displayRemaining`: remaining figures above flip to
  // "used" (1 - remaining) when the user prefers the used display.
  let ringArcValue = $derived(displayRemaining ? ringPercent : 1 - ringPercent);
  let ringArcLabel = $derived(
    displayRemaining
      ? ringLabel
      : `${Math.round((1 - ringPercent) * 100)}%`,
  );

  // 顶棚环 / 迷你胶囊：focus 到单个按量付费 provider 时，百分比无意义，
  // 改为金额（countdown=余额 / used=当月消费）。聚合(focus=all)时保持百分比。
  let headerMoney = $derived(
    focusedSnapshot
      ? payAsYouGoLabel(focusedSnapshot, displayRemaining)
      : null,
  );
  // focus 到单个按量付费 provider：顶部圆环以表情替代（通用按量计费交互）。
  let headerIsPayAsYouGo = $derived(
    focusedSnapshot ? isPayAsYouGo(focusedSnapshot) : false,
  );

  // The active snapshot whose heatmap is shown in the bottom panel.
  let activeSnapshot = $derived(
    focusedSnapshot ??
    snapshots.find((s) => s.provider_id === heatmapTabId) ??
    snapshots[0] ??
    null,
  );

  // Provider behind the floating detail overlay (hovered card). Rendered once
  // at window level so small cards (e.g. DeepSeek) can never clip its content.
  let detailSnapshot = $derived(
    hoveredId ? (snapshots.find((s) => s.provider_id === hoveredId) ?? null) : null,
  );

  // HeatmapGrid self-fetches per provider via get_heatmap, so every provider
  // can own a heatmap tab regardless of snapshot-embedded cells.
  $effect(() => {
    if (snapshots.length === 0) return;
    const stillExists = snapshots.some((s) => s.provider_id === heatmapTabId);
    if (!stillExists) {
      heatmapTabId = snapshots[0].provider_id;
    }
  });

  // Persist focus across restarts.
  $effect(() => {
    localStorage.setItem(FOCUS_KEY, focus);
  });

  // If the focused provider disappears (disabled in Settings), fall back to
  // "all". Skipped while snapshots is still empty (startup race).
  $effect(() => {
    if (snapshots.length === 0) return;
    if (focus !== "all" && !snapshots.some((s) => s.provider_id === focus)) {
      focus = "all";
    }
  });

  // Mini pill (compact mode) — focused provider color/name.
  const focusedColor = $derived(
    focusedSnapshot ? colorOf(focusedSnapshot.provider_id) : FOCUS_FALLBACK_COLOR,
  );
  const focusedName = $derived(
    focusedSnapshot
      ? providerShortName(
          focusedSnapshot.provider_id,
          focusedSnapshot.provider_display_name,
        )
      : "全部来源",
  );
  // 胶囊右侧品牌色芯片：账户强调色的低饱和底 + 同色文字展示完整短名，
  // flex:1 填满分隔线到右缘（固定布局、非自适应），消除右侧空白；
  // 超长自定义标签以省略号兜底，悬停 title 显示完整账户名。
  const badgeStyle = $derived.by(() => {
    const rgb = hexToRgb(focusedColor);
    if (!rgb) {
      return "background: rgba(255,255,255,0.08); border:1px solid rgba(255,255,255,.2); color:var(--tum-text-primary);";
    }
    return [
      `background: rgba(${rgb}, 0.16)`,
      `border: 1px solid rgba(${rgb}, 0.45)`,
      `color: ${focusedColor}`,
    ].join("; ");
  });
  const providerFullName = $derived(
    focusedSnapshot ? focusedSnapshot.provider_display_name : "全部来源",
  );

  let pillDragStart: { x: number; y: number; fromControl: boolean } | null = null;
  let pillDidDrag = false;

  // peek 双窗状态：pillRevealed = 胶囊已滑入（占满窗口）；pillSide = 贴靠边。
  let pillSide = $state<PeekSide>("right");
  let pillRevealed = $state(false);
  let pillExpanded = $state(false);
  let pillResizeGen = 0;
  let peekGen = 0;
  let dockTimer: ReturnType<typeof setTimeout> | null = null;
  let miniTimer: ReturnType<typeof setTimeout> | null = null;

  // 聚焦账户是否正在上报（驱动圆环的 2600ms 呼吸 halo）。
  let pillActive = $derived(
    !!(focusedSnapshot && actives[focusedSnapshot.provider_id]),
  );

  function clearDockTimer() {
    if (dockTimer !== null) {
      clearTimeout(dockTimer);
      dockTimer = null;
    }
  }

  function clearMiniTimer() {
    if (miniTimer !== null) {
      clearTimeout(miniTimer);
      miniTimer = null;
    }
  }

  /** 与后端同步「谁捕获鼠标 + 胶囊贴哪一边」。收起态 = docked。 */
  async function syncPeek(): Promise<void> {
    if (mode !== "compact") return;
    try {
      pillSide = await syncPeekWindow(!pillRevealed);
      // 收起态顺带让把手显形自愈：把手页只在收到 peek-show 时清除自己的
      // is-hidden，若之前因异常时序（例如重载、模式竞态）停在隐藏态，它就
      // 既看不见也点不到 —— 那样胶囊再也唤不出来。这里与停靠状态一并纠正。
      if (!pillRevealed) void emitPeekShow(pillSide).catch(() => {});
    } catch {
      // 窗口正在切换模式 / 已被关闭：保持当前状态即可。
    }
  }

  async function expandPill() {
    if (mode !== "compact" || snapshots.length === 0) return;
    const gen = ++pillResizeGen;
    const height = PILL_EXPANDED_BASE + snapshots.length * PILL_EXPANDED_ROW;
    try {
      await getCurrentWindow().setSize(
        new LogicalSize(PILL_COLLAPSED_W, height),
      );
      if (gen === pillResizeGen) pillExpanded = true;
    } catch {
      // IPC 失败：保持折叠。
    }
  }

  async function collapsePill(): Promise<void> {
    const gen = ++pillResizeGen;
    pillExpanded = false;
    try {
      await getCurrentWindow().setSize(
        new LogicalSize(PILL_COLLAPSED_W, PILL_COLLAPSED_H),
      );
    } catch {
      // 忽略：窗口可能正处于模式切换中。
    }
  }

  /** 把手（或已滑入的胶囊）被唤醒：解除穿透 → 滑入 → 120ms 后展开明细。 */
  async function revealPill() {
    if (mode !== "compact") return;
    const gen = ++peekGen;
    clearDockTimer();
    clearMiniTimer();
    pillRevealed = true;
    await syncPeek(); // docked=false：主窗接管鼠标，把手退出交互
    if (gen !== peekGen) return;
    miniTimer = setTimeout(() => {
      miniTimer = null;
      if (gen !== peekGen) return;
      void expandPill();
    }, PILL_REVEAL_MINI_MS);
  }

  /** 指针离开：停留 320ms 未被唤醒则收起。 */
  function scheduleDockPill() {
    if (mode !== "compact") return;
    clearDockTimer();
    const gen = peekGen;
    dockTimer = setTimeout(() => {
      dockTimer = null;
      if (gen !== peekGen) return;
      void dockPill();
    }, PILL_DOCK_DELAY_MS);
  }

  /** 折叠 → 滑出 150ms → 再 90ms 后把手复现、主窗恢复穿透。
   *
   *  ⚠️ 不要在这里加 `if (!pillRevealed) return` 之类的短路：即使胶囊从未滑入
   *  （指针 60ms 内刷过把手就离开，把手已把自己隐去，而 peek-hover 被防抖抑制
   *  或被随后的 peek-leave 取代），也必须照常走到 emitPeekShow —— 这是把手唯一
   *  的「复活」路径。少了它，把手会永久停在 opacity:0 + pointer-events:none，
   *  胶囊再也无法被唤醒（等于应用不可达）。 */
  async function dockPill() {
    const gen = ++peekGen;
    clearMiniTimer();
    await collapsePill();
    pillRevealed = false;
    setTimeout(async () => {
      if (gen !== peekGen) return;
      // 先取回最新贴靠边（用户可能刚把胶囊拖到屏幕另一侧），再把边随
      // peek-show 发给把手页，让它纠正圆角朝向并复现自己。
      await syncPeek(); // docked=true：把手接回鼠标
      if (gen !== peekGen) return;
      void emitPeekShow(pillSide).catch(() => {});
    }, PILL_SLIDE_MS + PILL_PEEK_BACK_MS);
  }

  // Compact pill three-state tone (spec §5.2): ringPercent is aggregate
  // REMAINING, so used = 1 - ringPercent. <80 silent / 80-95 amber / >=95 red.
  // Empty data normalizes to "ok" so the no-data idle ring never breathes red.
  let pillTone = $derived(
    snapshots.length === 0
      ? "ok"
      : 1 - ringPercent >= 0.95
        ? "crit"
        : 1 - ringPercent >= 0.8
          ? "warn"
          : "ok",
  );

  function onPillPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    pillDragStart = {
      x: event.clientX,
      y: event.clientY,
      fromControl: (event.target as HTMLElement).closest("button") !== null,
    };
    pillDidDrag = false;
    if (!pillDragStart.fromControl) {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    }
  }

  function onPillPointerMove(event: PointerEvent) {
    if (!pillDragStart) return;
    const dx = event.clientX - pillDragStart.x;
    const dy = event.clientY - pillDragStart.y;
    if (!pillDidDrag && Math.hypot(dx, dy) > PILL_DRAG_THRESHOLD_PX) {
      pillDidDrag = true;
      void getCurrentWindow().startDragging();
    }
  }

  function onPillPointerUp(event: PointerEvent) {
    if (!pillDragStart || event.button !== 0) return;
    const didDrag = pillDidDrag;
    const fromControl = pillDragStart.fromControl;
    pillDragStart = null;
    pillDidDrag = false;
    if (!didDrag && !fromControl) {
      void toggleMode();
    } else if (didDrag) {
      // 拖拽结束即收边：compact 胶囊在拖动过程中就被 Rust 贴死到最近的左/右边缘，
      // 所以松手就代表「拖到边缘了」—— 立刻滑出，而不是等指针离开 320ms。
      // 先取回最新贴靠边（可能刚被拖到另一侧），再走与自动收起相同的链路。
      void syncPeek().then(() => dockPill());
    }
  }

  let timeLabel = $derived(
    now.toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    }),
  );

  // Per-provider default interval is 5 minutes (300s). User can override via
  // Settings. 0 means "use default".
  let pollIntervalSec = $derived(
    settings && settings.poll_interval_seconds > 0
      ? settings.poll_interval_seconds
      : 300,
  );
  let nextRefreshMs = $derived(lastRefreshAt + pollIntervalSec * 1000);
  let nextRefreshLabel = $derived.by(() => {
    const remaining = Math.max(0, nextRefreshMs - now.getTime());
    const totalSec = Math.floor(remaining / 1000);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    if (m > 0) return `${m}m ${s.toString().padStart(2, "0")}s`;
    return `${s}s`;
  });

  async function toggleMode() {
    const next: Mode = mode === "dashboard" ? "compact" : "dashboard";
    await setWindowMode(next);
    mode = next;
    pillRevealed = false;
    pillExpanded = false;
    pillResizeGen++;
    peekGen++;
    clearDockTimer();
    clearMiniTimer();
    // compact：重建把手（Rust 侧建窗 + 定位 + 交出/接管鼠标）并取回贴靠边；
    // dashboard：set_window_mode 已在 Rust 里销毁把手并恢复鼠标交互。
    await syncPeek();
  }

  async function refresh() {
    await forceRefresh();
    snapshots = await getUsage();
    lastRefreshAt = Date.now();
  }

  async function closeApp() {
    await getCurrentWindow().close();
  }

  function formatError(err: ProviderError): string {
    if (err.Network) return `网络错误 · ${err.Network.message}`;
    if (err.Auth) return `鉴权失败 · ${err.Auth.message}`;
    if (err.Parse) return `解析错误 · ${err.Parse.message}`;
    if (err.RateLimited) return "被限流，稍后重试";
    if (err.NotConfigured) return "未配置";
    if (err.Internal) return `内部错误 · ${err.Internal.message}`;
    return "未知错误";
  }

  function extractErrors(states: Record<string, ProviderState>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [id, st] of Object.entries(states)) {
      if (st.last_error) out[id] = formatError(st.last_error);
    }
    return out;
  }
</script>

<main class="shell" class:shell--compact={mode === "compact"} data-tauri-drag-region oncontextmenu={(e) => e.preventDefault()}>
  {#if mode === "compact"}
    <div
      class="pill-layer"
      class:is-docked={!pillRevealed}
      data-side={pillSide}
      role="presentation"
      onpointerenter={() => clearDockTimer()}
      onpointerleave={() => scheduleDockPill()}
    >
      <div
        class="pill"
        data-tone={pillTone}
        onpointerdown={onPillPointerDown}
        onpointermove={onPillPointerMove}
        onpointerup={onPillPointerUp}
        oncontextmenu={(e) => e.preventDefault()}
        role="group"
        aria-label="迷你用量面板"
      >
        <div class="pill__main">
          <button
            type="button"
            class="pill__restore"
            aria-label="恢复主面板"
            onclick={(e) => {
              e.stopPropagation();
              void toggleMode();
            }}
          >
            <span class="pill__ring">
              {#if headerIsPayAsYouGo && focusedSnapshot}
                <span class="pill__avatar" title={providerFullName}>
                  <ProviderLogo
                    kind={focusedSnapshot.provider_id.split("-")[0]}
                    size={22}
                    accent={focusedColor}
                  />
                </span>
              {:else}
                <ProgressRing
                  value={ringArcValue}
                  label=""
                  size={38}
                  stroke={3.5}
                  idle={snapshots.length === 0}
                  countdown={displayRemaining}
                  markKind={focusedSnapshot
                    ? focusedSnapshot.provider_id.split("-")[0]
                    : null}
                  running={pillActive}
                  accent={focusedColor}
                />
              {/if}
              <span
                class="pill__percent"
                class:pill__percent--crit={pillTone === "crit"}
              >{headerMoney ?? ringArcLabel}</span>
            </span>
            <span class="pill__divider"></span>
            <span class="pill__badge" style={badgeStyle} title={providerFullName}
              >{focusedName}</span
            >
          </button>
          <button
            type="button"
            class="pill__close"
            title="关闭应用"
            aria-label="关闭应用"
            onpointerdown={(e) => e.stopPropagation()}
            onclick={(e) => {
              e.stopPropagation();
              void closeApp();
            }}
          >✕</button>
        </div>
        {#if pillExpanded}
          <MiniPanel {snapshots} {actives} countdown={displayRemaining} accentFor={(id) => accentById[id]} />
        {/if}
      </div>
    </div>
  {:else}
    <header class="shell__header" data-tauri-drag-region>
      <div class="shell__brand">
        <span class="shell__dot"></span>
        <span class="shell__title">TokenUsageMonitor</span>
      </div>
      <div class="shell__actions" data-tauri-drag-region={false}>
        {#if headerIsPayAsYouGo && focusedSnapshot}
          <span class="shell__avatar" title={providerFullName}>
            <ProviderLogo
              kind={focusedSnapshot.provider_id.split("-")[0]}
              size={22}
              accent={focusedColor}
            />
          </span>
        {:else}
          <ProgressRing value={headerMoney ? 0 : ringArcValue} label={headerMoney ?? ringArcLabel} size={28} stroke={3} idle={snapshots.length === 0 || !!headerMoney} countdown={displayRemaining} />
        {/if}
        <button class="shell__btn" onclick={refresh} title="立即刷新">↻</button>
        <button class="shell__btn" onclick={() => openSettings()} title="设置">⚙</button>
        <button class="shell__btn" onclick={toggleMode} title="折叠到迷你态">⤢</button>
        <button class="shell__btn shell__btn--close" onclick={closeApp} title="关闭">×</button>
      </div>
    </header>

    <div class="view-row" data-tauri-drag-region={false}>
      {#each tabs as t (t.key)}
        <button
          type="button"
          class="view-tab"
          class:is-active={view === t.key}
          onclick={() => (view = t.key)}
        >{t.label}</button>
      {/each}
    </div>

    {#if view === "overview"}
      {#if snapshots.length > 0}
      <div class="focus-row" data-tauri-drag-region={false}>
        <PillsOrSelect
          items={[{ id: "all", label: "全部" }, ...snapshots.map((s) => ({ id: s.provider_id, label: s.provider_display_name }))]}
          value={focus}
          onPick={(id) => {
            focus = id;
          }}
          dotFor={(id) => (id === "all" ? undefined : colorOf(id))}
          brandFor={(id) =>
            id === "all"
              ? null
              : {
                  color: colorOf(id),
                  kind: id.split("-")[0],
                  experimental: EXPERIMENTAL_KINDS.has(id.split("-")[0]),
                }}
        />
      </div>
    {/if}

    <!-- svelte-ignore a11y_no_static_element_interactions:
         card drag-and-drop is a pointer-only interaction; keyboard users reach
         the same selection via the card title button (see ProviderCard). -->
    <section
      class="shell__cards"
      ondragover={(e) => {
        if (draggingId) e.preventDefault();
      }}
    >
      {#if snapshots.length === 0}
        <div class="shell__empty">
          <p>正在拉取最新用量…</p>
          <p class="shell__hint">首次启动可能需要 1-2 秒</p>
        </div>
      {:else}
        {#each orderedSnapshots as snap (snap.provider_id)}
          <!-- svelte-ignore a11y_no_static_element_interactions:
               drag handle wrapper; selection stays keyboard-accessible via the
               card title button (see ProviderCard). -->
          <div
            class="shell__card-slot"
            class:shell__card-slot--dragging={draggingId === snap.provider_id}
            draggable="true"
            ondragstart={(e) => onCardDragStart(e, snap.provider_id)}
            ondragover={(e) => onCardDragOver(e, snap.provider_id)}
            ondragend={onCardDragEnd}
            ondrop={(e) => e.preventDefault()}
            animate:flip={{ duration: 200 }}
          >
            <ProviderCard
              snapshot={snap}
              error={errors[snap.provider_id] ?? null}
              burn={burns[snap.provider_id] ?? null}
              active={actives[snap.provider_id] ?? false}
              {lastRefreshAt}
              focused={focus === snap.provider_id}
              accent={accentById[snap.provider_id]}
              countdown={displayRemaining}
              onHover={onCardHover}
              onSelect={() => {
                focus = snap.provider_id;
                heatmapTabId = snap.provider_id;
              }}
            />
          </div>
        {/each}
      {/if}
    </section>

    {#if snapshots.length > 0}
      <section class="shell__heatmap" data-tauri-drag-region={false}>
        <div class="heatmap__head">
          <span class="heatmap__title">日历热力图</span>
          {#if activeSnapshot?.provider_id.startsWith("minimax")}
            <span class="heatmap__src">· 来源：本机 MiniMax Code</span>
          {/if}
          {#if focus === "all"}
            <PillsOrSelect
              items={snapshots.map((s) => ({ id: s.provider_id, label: s.provider_display_name }))}
              value={heatmapTabId}
              onPick={(id) => (heatmapTabId = id)}
              dotFor={(id) => colorOf(id)}
            />
          {/if}
          <div class="heatmap__view" role="group" aria-label="热力图时间范围">
            <button
              class="heatmap__view-btn"
              class:is-active={heatmapView === "compact"}
              onclick={() => (heatmapView = "compact")}
              title="最近 31 天滚动窗口"
            >31天</button>
            <button
              class="heatmap__view-btn"
              class:is-active={heatmapView === "calendar"}
              onclick={() => (heatmapView = "calendar")}
              title="按自然月查看近 6 个月用量"
            >月历</button>
          </div>
        </div>
        {#if activeSnapshot}
          <HeatmapGrid
            providerId={activeSnapshot.provider_id}
            view={heatmapView}
            emptyHint={activeSnapshot.provider_id === "deepseek"
              ? "DeepSeek 依据余额下降累计消耗，启用后需积累数日才有数据"
              : "该来源暂无热力图数据"}
          />
        {/if}
      </section>
    {/if}
    {:else if view === "trend"}
      {#if snapshots.length > 0}
        <TrendPanel
          providerIds={snapshots.map((s) => s.provider_id)}
          colors={snapshots.reduce(
            (m, s) => {
              m[s.provider_id] = colorOf(s.provider_id);
              return m;
            },
            {} as Record<string, string>,
          )}
        />
      {:else}
        <div class="shell__empty"><p>暂无用量数据</p></div>
      {/if}
    {:else if view === "tools"}
      <ToolPanel />
    {:else if view === "devices"}
      <DevicePanel providerCount={snapshots.length} />
    {:else}
      <ModelPanel />
    {/if}

    <footer class="shell__footer" data-tauri-drag-region>
      <span class="shell__time">{timeLabel}</span>
      <span class="shell__next">
        <span class="shell__next-label">下次刷新</span>
        <span class="shell__next-value">{nextRefreshLabel}</span>
      </span>
      <span class="shell__count">共 {snapshots.length} 个 Provider</span>
    </footer>

    {#if detailSnapshot}
      <div
        class="detail-overlay"
        role="group"
        data-tauri-drag-region={false}
        style={`left:${overlayX}px;top:${overlayY}px;visibility:${overlayReady ? "visible" : "hidden"};`}
        onpointerenter={cancelOverlayHide}
        onpointerleave={scheduleOverlayHide}
        transition:fly={{ y: 6, duration: 120 }}
        bind:this={overlayEl}
      >
        <DetailCard
          snapshot={detailSnapshot}
          burn={burns[detailSnapshot.provider_id] ?? null}
          {lastRefreshAt}
          countdown={displayRemaining}
          accent={accentById[detailSnapshot.provider_id]}
          error={errors[detailSnapshot.provider_id] ?? null}
        />
      </div>
    {/if}
  {/if}
</main>

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    /* Layered glass over a transparent window: a faint accent tint on top of a
       translucent dark base. The window itself is transparent (mica removed),
       so the border-radius corners below are genuinely see-through. */
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
      rgba(24, 26, 30, 0.82);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-lg);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    padding: var(--tum-space-4);
    gap: var(--tum-space-3);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    overflow: hidden;
    cursor: grab;
  }

  /* compact 模式：去掉外壳自身的玻璃与内边距，把整窗交给胶囊层。
     overflow:hidden 负责把滑出窗外的胶囊层裁掉。 */
  .shell--compact {
    position: relative;
    padding: 0;
    gap: 0;
    border: none;
    border-radius: 0;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    background: transparent;
  }

  /* 胶囊整体平移：收起时滑出窗外，唤醒时滑回 (0,0)。
     贴左边时方向镜像。 */
  .pill-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    transition: transform 150ms var(--tum-ease-spring);
  }

  .pill-layer.is-docked[data-side="right"] {
    transform: translateX(100%);
  }

  .pill-layer.is-docked[data-side="left"] {
    transform: translateX(-100%);
  }

  .shell:active {
    cursor: grabbing;
  }

  .shell__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: var(--tum-space-2);
    border-bottom: 1px solid var(--tum-border);
  }

  .shell__brand {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .shell__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--tum-accent);
    box-shadow: 0 0 8px var(--tum-accent-glow), 0 0 2px var(--tum-accent);
  }

  .shell__title {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    font-family: var(--tum-font-mono);
  }

  .shell__actions {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .shell__avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    overflow: hidden;
    border: 1px solid var(--tum-border-strong);
    background: rgba(255, 255, 255, 0.06);
  }

  .shell__btn {
    width: 22px;
    height: 22px;
    border: 1px solid var(--tum-border);
    background: var(--tum-surface);
    color: var(--tum-text-secondary);
    border-radius: var(--tum-radius-sm);
    cursor: pointer;
    font-size: 11px;
    line-height: 1;
    transition: all 0.15s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--tum-font-mono);
  }

  .shell__btn:hover {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .shell__cards {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
    padding: 0 3px;
    scrollbar-width: thin;
    scrollbar-color: var(--tum-border) transparent;
  }

  .shell__cards::-webkit-scrollbar {
    width: 4px;
  }

  .shell__cards::-webkit-scrollbar-thumb {
    background: var(--tum-border);
    border-radius: 2px;
  }

  .shell__card-slot {
    min-width: 0;
    cursor: grab;
  }

  .shell__card-slot:active {
    cursor: grabbing;
  }

  .shell__card-slot--dragging {
    opacity: 0.45;
  }

  .shell__empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    text-align: center;
    gap: 6px;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.4px;
  }

  .shell__hint {
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .shell__heatmap {
    padding: var(--tum-space-2) 0;
    border-top: 1px solid var(--tum-border);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-2);
  }

  .heatmap__head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--tum-space-2);
    flex-wrap: wrap;
  }

  .heatmap__title {
    font-size: var(--tum-font-size-xs);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
  }

  .heatmap__src {
    font-family: var(--tum-font-mono);
    font-size: 9px;
    color: var(--tum-accent, #4cc2ff);
    letter-spacing: 0.3px;
  }

  /* 热力图时间范围切换（31天滚动 / 月历），与 Provider tabs 同语言的
     胶囊分段控件。 */
  .heatmap__view {
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-pill);
    background: rgba(255, 255, 255, 0.04);
  }

  .heatmap__view-btn {
    border: none;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    padding: 2px 9px;
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
  }

  .heatmap__view-btn:hover {
    color: var(--tum-text-primary);
  }

  .heatmap__view-btn.is-active {
    background: rgba(76, 194, 255, 0.18);
    color: var(--tum-text-primary);
  }

  .focus-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  /* C7 多视图切换：总量/趋势 + 未实装占位（工具/模型/设备）。与 Provider
     tabs 同语言的胶囊分段控件，但置于 header 之下、内容区之上。 */
  .view-row {
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-pill);
    background: rgba(255, 255, 255, 0.04);
    align-self: flex-start;
    flex: none;
  }

  .view-tab {
    border: none;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    font-family: var(--tum-font);
    padding: 3px 10px;
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
  }

  .view-tab:hover {
    color: var(--tum-text-primary);
  }

  .view-tab.is-active {
    background: rgba(76, 194, 255, 0.18);
    color: var(--tum-text-primary);
  }

  .shell__footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-top: var(--tum-space-2);
    border-top: 1px solid var(--tum-border);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.4px;
    gap: var(--tum-space-2);
  }

  .shell__time {
    color: var(--tum-accent);
    letter-spacing: 0.8px;
  }

  .shell__next {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--tum-text-muted);
  }

  .shell__next-label {
    text-transform: uppercase;
    letter-spacing: 0.8px;
  }

  .shell__next-value {
    color: var(--tum-text-secondary);
    letter-spacing: 0.4px;
  }

  /* 贴边胶囊：收起态物理窗 168x56（与 Rust set_window_mode 的 LogicalSize 一致），
     展开时按 PILL_EXPANDED_BASE/ROW 增高。
     data-tone 三态：<80% 静默 / 80-95% 琥珀 / >=95% 红色呼吸（spec §5.2 CompactPill）。 */
  .pill {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0 10px;
    border-radius: 22px;
    background: var(--tum-glass);
    /* 不画投影：胶囊填满窗口、四角被圆角切掉，投影的深色像素只会从这四个缺口
       漏出来（表现为「四角没全透明」），而窗口会把窗外的投影全部裁掉，
       等于零立体感换取一处瑕疵。 */
    border: 1px solid var(--tum-border-strong);
    overflow: hidden;
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
    cursor: default;
  }

  .pill[data-tone="warn"] {
    border-color: rgba(255, 200, 61, 0.5);
  }

  .pill[data-tone="crit"] {
    border-color: rgba(255, 95, 86, 0.65);
    animation: pill-breathe 1.6s ease-in-out infinite;
  }

  @keyframes pill-breathe {
    0%,
    100% {
      box-shadow: inset 0 0 0 1px rgba(255, 95, 86, 0.4);
    }
    50% {
      box-shadow:
        0 0 16px 0 rgba(255, 95, 86, 0.5),
        inset 0 0 0 1px rgba(255, 95, 86, 0.85);
    }
  }

  .pill__main {
    /* 高度与脚本常量 PILL_COLLAPSED_H 保持一致（收起态物理窗 168×56）。 */
    height: 56px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .pill__restore {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: none;
    background: transparent;
    appearance: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .pill__restore:focus-visible {
    outline: 2px solid var(--tum-accent);
    outline-offset: -2px;
    border-radius: var(--tum-radius-pill);
  }

  .pill__ring {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .pill__avatar {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    overflow: hidden;
    border: 1px solid var(--tum-border-strong);
    background: rgba(255, 255, 255, 0.06);
  }

  .pill__percent {
    font-size: 11px;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
  }

  .pill__percent--crit {
    color: var(--tum-crit);
  }

  .pill__divider {
    width: 1px;
    height: 24px;
    background: var(--tum-border-strong);
    flex-shrink: 0;
  }

  .pill__badge {
    /* 品牌色芯片：flex:1 填满分隔线到右缘的固定布局（非自适应宽度），
       展示完整 Provider 短名；底色/描边/文字色由内联 style 注入。 */
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 6px;
    border-radius: 11px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 10.5px;
    font-weight: 600;
    font-family: var(--tum-font);
    letter-spacing: 0.2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
  }

  .pill__close {
    /* 关闭钮悬浮右上角（不占布局空间，悬停时淡入），为品牌芯片让出宽度 */
    position: absolute;
    top: 4px;
    right: 4px;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.2s ease,
      background 0.2s ease,
      color 0.2s ease;
  }

  .pill:hover .pill__close {
    opacity: 1;
  }

  .pill__close:hover {
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
  }

  .pill__close:focus-visible {
    opacity: 1;
    outline: 2px solid var(--tum-accent);
    outline-offset: -2px;
  }

  /* Floating detail overlay anchored beside the hovered card: rendered at
     window level so the full detail (rows + chart) is never clipped by a
     small card. Position + width are set inline after measuring the content
     box; it flips above the card when the viewport bottom is tight. Parking
     the pointer on it keeps it pinned so the actions inside are reachable. */
  .detail-overlay {
    position: fixed;
    z-index: 60;
    width: 320px;
    max-width: calc(100vw - 16px);
    pointer-events: auto;
  }

  .detail-overlay :global(.detail) {
    padding: 10px 12px;
  }
</style>
