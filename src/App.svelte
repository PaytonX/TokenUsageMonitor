<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { flip } from "svelte/animate";
  import { fly } from "svelte/transition";
  import { get } from "svelte/store";
  import { highlightKeyForKind, ledgerSeriesForKind } from "./lib/calendar-linkage";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    getUsage,
  writePerfProbe,
    getProviderStates,
    getSettings,
    getRouterStatus,
    onRouterSwitched,
    onUsageUpdated,
    onProviderError,
    onSettingsChanged,
    setWindowMode,
    saveSettings,
    syncPeekWindow,
    dockSideOf,
    setPillDragging,
    emitPeekShow,
    onPeekHover,
    onPeekLeave,
    onPeekReveal,
    onPillDragSettled,
    onPerfDrive,
    emitPerfDriveDone,
    type PerfDriveRequest,
    type PerfDriveDone,
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
    type RouterStatus,
    ringWindowRemaining,
    providerShortName,
    payAsYouGoLabel,
    isPayAsYouGo,
    hexToRgb,
  } from "./lib";
  import {
    markIpc,
    markFrameStart,
    observeLongFrames,
    markSpan,
    markEnd,
    spanList,
    spanDroppedCount,
    loafEntries,
    loafSelfCheck,
    perfSnapshot,
    perfReset,
    PERF_PROBE_ENABLED,
  } from "./lib/perf";
  import {
    initialHoverState,
    reduceHover,
    type HoverEvent,
  } from "./lib/pill-hover-state";
  import {
    DASHBOARD_SIZE,
    PILL_COLLAPSED_SIZE,
    createSizeWriteQueue,
    pillExpandedHeight,
    shouldApplySizeWrite,
  } from "./lib/window-size";
  import ProviderCard from "./lib/components/ProviderCard.svelte";
  import ProgressRing from "./lib/components/ProgressRing.svelte";
  import MiniPanel from "./lib/components/MiniPanel.svelte";
  import DetailCard from "./lib/components/DetailCard.svelte";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";
  import TrendPanel from "./lib/components/TrendPanel.svelte";
  import ToolPanel from "./lib/components/ToolPanel.svelte";
  import ModelPanel from "./lib/components/ModelPanel.svelte";
  import DevicePanel from "./lib/components/DevicePanel.svelte";
  import PillsOrSelect from "./lib/components/PillsOrSelect.svelte";
  import Icon from "./lib/components/atoms/Icon.svelte";
  import { brandColorFor, EXPERIMENTAL_KINDS } from "./lib/brand-glyphs";
  import { t } from "./lib/i18n/store";
  import { durMs } from "./lib/motion/tokens";

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
      { key: "trend", label: "tab.trend" },
      { key: "tools", label: "tab.tools" },
      { key: "models", label: "tab.models" },
      { key: "devices", label: "tab.devices" },
    ];
    return defs
      .filter((tab) => !hidden.has(tab.key))
      .map((tab) => ({ key: tab.key as ViewMode, label: tab.label }));
  });
  const tabs = $derived([
    { key: "overview", label: "tab.overview" },
    ...extraTabs,
  ] as { key: ViewMode; label: string }[]);
  // 当前页签被禁用时回落总量，避免停留在未渲染的页签上。
  $effect(() => {
    if (view !== "overview" && !extraTabs.some((tab) => tab.key === view)) {
      view = "overview";
    }
  });

  let snapshots = $state<UsageSnapshot[]>([]);
  let errors = $state<Record<string, string>>({});
  let burns = $state<Record<string, BurnInfo | null>>({});
  let actives = $state<Record<string, boolean>>({});
  let mode = $state<Mode>("dashboard");
  const FOCUS_FALLBACK_COLOR = "rgb(138,143,152)";
  const FOCUS_KEY = "tum.focus";

  // Global focus: "all" (aggregate min) or one provider_id. Drives the
  // header ring, the chips row and the heatmap panel.
  let focus = $state(localStorage.getItem(FOCUS_KEY) ?? "all");
  // Provider whose card the pointer is currently over; drives the floating
  // detail overlay, which is docked along the window bottom so it covers the
  // calendar-heatmap area rather than the cards themselves.
  let hoveredId = $state<string | null>(null);
  // Overlay box is measured before reveal so the docked position uses the real
  // content height (it varies per provider). Stays hidden until first measure.
  let overlayEl = $state<HTMLElement | null>(null);
  let measuredH = $state(0);
  let overlayY = $state(0);
  let overlayReady = $state(false);
  // Height reserved at the bottom for the status footer, so the docked overlay
  // sits directly above it instead of overlapping the clock/refresh line.
  const FOOTER_CLEARANCE = 48;
  const VIEWPORT_MARGIN = 8;

  // Hiding the detail overlay is deferred by a grace window so the pointer
  // can travel between cards (or off them) without it vanishing mid-move.
  // The overlay is pointer-transparent, so it is never entered directly; the
  // grace window only smooths a fast sweep across the card row.
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleOverlayHide() {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      hoveredId = null;
      hideTimer = null;
    }, 350);
  }
  function cancelOverlayHide() {
    if (hideTimer) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
  }
  function onCardHover(id: string, hovering: boolean) {
    if (hovering) {
      cancelOverlayHide();
      hoveredId = id;
      // Hide until the ResizeObserver re-measures the new content — avoids a
      // one-frame ghost showing the previous card's detail at the wrong height.
      overlayReady = false;
    } else if (hoveredId === id) {
      scheduleOverlayHide();
    }
  }

  // Dock the overlay along the bottom of the window, just above the footer, so
  // hovering a card swaps the calendar-heatmap area for the account's detail
  // without ever covering the cards being scanned. The width is full-bleed via
  // CSS (left/right), so only the vertical position is computed here: sit on the
  // footer line, and if the content is taller than the space above the footer,
  // pin it to the top edge so the top of the detail stays readable and the
  // panel scrolls internally.
  function placeOverlay() {
    const h = measuredH;
    const vh = window.innerHeight;
    const bottomDocked = vh - FOOTER_CLEARANCE - h;
    overlayY = Math.max(VIEWPORT_MARGIN, Math.min(bottomDocked, vh - FOOTER_CLEARANCE - h));
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
        measuredH = box.height;
        overlayReady = true;
      }
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Re-dock whenever the content height changes (each provider's detail is a
  // different height, and the window can be resized).
  $effect(() => {
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
  // 尺寸常量来自 lib/window-size.ts（与 Rust set_window_mode 保持一致，
  // 那边有单测钉住「前端与 Rust 的尺寸不许漂」）。
  const PILL_COLLAPSED_W = PILL_COLLAPSED_SIZE.width;
  const PILL_COLLAPSED_H = PILL_COLLAPSED_SIZE.height;
  /**
   * 全窗态（dashboard）的窗口尺寸。与 Rust `set_window_mode` 的
   * `LogicalSize::new(400u32, 680u32)` 保持一致。
   *
   * 为什么前端也要知道它：尺寸写的**唯一入口**（见 `writeWindowSize`）要能在
   * 模式切换收尾时补写一笔「当前模式应有的尺寸」，否则晚到的那笔胶囊尺寸写
   * 会把全窗态窗口又改回 168 宽。
   */
  const DASHBOARD_W = DASHBOARD_SIZE.width;
  const DASHBOARD_H = DASHBOARD_SIZE.height;
  // 展开高度（60 + 30n）的算式搬到 lib/window-size.ts::pillExpandedHeight
  // ——那边有单测钉住，免得行高/间距改了没人发现窗口高度对不上。
  // 三段式时间线的时点，与 CSS 里的滑出时长**必须同步**（见 tokens.css 的
  // --tum-dur-slide-out）。改一边不改另一边，就会出现「滑出还没播完就缩窗裁剪」
  // 或「把手在面板还在滑的时候就抢回鼠标」的割裂感。
  //
  // PILL_PEEK_LEAD_MS(120) 与滑出的淡出时长是**耦合**的：
  // 淡出走完的时刻正好是把手复现的时刻，两者一旦错开就会看到把手浮在
  // 半透明胶囊上。数值巧合而已，改任一侧都要同时改另一侧。
  const PILL_SLIDE_MS = durMs("--tum-dur-slide-out", 180); // = --tum-dur-slide-out（Glance CLOSE 同样是 180ms）
  const PILL_PEEK_LEAD_MS = durMs("--tum-dur-instant", 120); // 与淡出相结束时点对齐
  const PILL_REVEAL_MINI_MS = durMs("--tum-dur-base", 200); // 滑入的淡入相（140ms）走完后再展开明细
  const PILL_DOCK_DELAY_MS = 320; // 指针离开后停留多久才收起
  /**
   * 滑出结束后再等多久才缩窗（ms）。0 = 与滑出同刻缩（现状）。
   *
   * 探针实验用（issues/05 §2.3）：实测 `collapsePill` 里的 `setSize` 偶发
   * 500ms 尖刺，而缩窗时刻恰好与滑出动画的结束时刻重合。把缩窗推迟到
   * 滑出彻底结束之后，尖刺若消失，就证明成因是「改尺寸与动画合成争用」
   * 而非「改尺寸本身慢」——两者的修法完全不同。
   */
  const PILL_SHRINK_DELAY_MS = 0;
  // 模式恢复判定阈值（逻辑像素）：介于 compact(168) 与 dashboard(400) 之间。
  const PILL_RESTORE_MAX_W = 200;
  let settings = $state<Settings | null>(null);
  // TokenRouter 运行态（快速开关的状态点数据源）。enabled 以 settings 为准，
  // listening/bind_error 来自 get_router_status，切换事件与设置保存都会刷新。
  let routerStatus = $state<RouterStatus | null>(null);
  let routerOn = $derived(settings?.router?.enabled ?? false);
  let routerListening = $derived(routerStatus?.health.listening ?? false);
  let routerBindError = $derived(routerStatus?.health.bind_error ?? null);
  let routerTitle = $derived(
    routerBindError
      ? $t("app.router.bindError", { err: routerBindError })
      : routerOn
        ? routerListening
          ? $t("app.router.onListening")
          : $t("app.router.on")
        : $t("app.router.off"),
  );

  /** 快速开关：翻转 router.enabled 并整体保存，settings-changed 事件会带回新态。 */
  async function toggleRouter() {
    if (!settings) return;
    const next: Settings = {
      ...settings,
      router: {
        ...(settings.router ?? {
          port: 43211,
          proactive_threshold_percent: 20,
          error_cooldown_secs: 300,
          conn_breaker_count: 3,
          probe_start_secs: 60,
          probe_max_attempts: 5,
          routes: [],
        }),
        enabled: !routerOn,
      },
    };
    try {
      await saveSettings(next);
      settings = next;
    } catch {
      /* 保存失败保持原态；设置页里能看到具体错误。 */
    }
    getRouterStatus()
      .then((s) => (routerStatus = s))
      .catch(() => {});
  }

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
    // 帧成本通道：这是唯一与 Glance 的 SLOW_FRAME 语义对等的指标
    // （真正含 style/layout/paint/composite）。不支持的 WebView2 上返回 false，
    // 此时只有 frameStart（更新到帧开始的延迟）可用。别拿它冒充帧成本。
    observeLongFrames();

    // 探针落盘（issues/05 §1）：探针数据只活在 WebView 的 JS 上下文里，
    // 宿主读不到；release 又不带 DevTools 面板。周期性写文件，测量就变成
    // 「应用自己写」，剩下的分析在宿主侧做，不需要任何 GUI 操作。
    // PERF_PROBE_ENABLED 为假时整个定时器不建（默认 release 构建）。
    if (PERF_PROBE_ENABLED) {
      const dump = setInterval(() => {
        const payload = {
          at: new Date().toISOString(),
          loafSupported: loafSelfCheck(),
          spans: spanList(),
          // 被容量淘汰掉的 span 条数：>0 说明日志不完整，判读时必须知道。
          spanDropped: spanDroppedCount(),
          loaf: loafEntries(),
          snapshot: perfSnapshot(),
        };
        void writePerfProbe(JSON.stringify(payload)).catch(() => {});
      }, 2000);
      unlistenFns.push(() => clearInterval(dump));
    }

    // 并行拉取用量 + 状态 + 设置，减少启动等待。
    // 三条通道量纲不同、不可互替：见 ./lib/perf 的文件头。
    const [usage, states] = await markIpc(() =>
      Promise.all([getUsage(), getProviderStates()]),
    );
    snapshots = usage;
    errors = extractErrors(states);
    settings = await getSettings();
    lastRefreshAt = Date.now();
    void markFrameStart();

    // TokenRouter 状态点：初始拉取 + 切换事件刷新。
    getRouterStatus()
      .then((s) => (routerStatus = s))
      .catch(() => {});
    unlistenFns.push(
      await onRouterSwitched(() => {
        getRouterStatus()
          .then((s) => (routerStatus = s))
          .catch(() => {});
      }),
    );


    // Live updates.
    // 这是**主路径**：数据由后端推送，没有 IPC 等待，所以只有 render span 有意义。
    unlistenFns.push(
      await onUsageUpdated((update) => {
        const snap = update.snapshot;
        snapshots = snapshots
          .filter((s) => s.provider_id !== snap.provider_id)
          .concat(snap)
          .sort((a, b) => a.provider_id.localeCompare(b.provider_id));
        burns = { ...burns, [snap.provider_id]: update.burn };
        actives = { ...actives, [snap.provider_id]: update.active };
        errors = { ...errors, [snap.provider_id]: "" };
        lastRefreshAt = Date.now();
        // 量「数据写入 state → 下一帧提交前」。连续推送时后一次会让前一次判为
        // stale 丢弃，不会把两帧串成一条样本（见 ./lib/perf）。
        void markFrameStart();
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
        snapshots = await markIpc(() => getUsage());
        const states = await getProviderStates();
        errors = extractErrors(states);
        lastRefreshAt = Date.now();
        void markFrameStart();
      }),
    );

    // 把手只存在于贴边态（浮动态没有把手窗口），因此这两个全局事件只在贴边时生效。
    unlistenFns.push(
      await onPeekHover(() => {
        void revealOnPeekHover();
      }),
    );
    unlistenFns.push(
      await onPeekLeave(() => {
        // 把手滑入时必然先隐去自己并发一次 peek-leave，而此刻指针已经落在滑入的
        // 胶囊上——这不是「离开」。据此收起会造成「唤出即被收回」。
        if (!pillHovered) scheduleDockPill();
      }),
    );
    unlistenFns.push(await onPeekReveal(() => void revealPill()));
    unlistenFns.push(await onPillDragSettled(() => void settlePillAfterDrag()));

    // 探针驱动器（issues/05）：Rust 发「perf-drive」后，无人值守地跑
    // 「贴边 → 唤出 → 收起」循环，用于复现偶发的贴边卡顿。
    // 生产构建里 PERF_PROBE_ENABLED 为 false —— 整段不注册，监听器不存在、
    // 命令不可达；驱动器模块也不会被下载（动态 import）。
    if (PERF_PROBE_ENABLED) {
      unlistenFns.push(
        await onPerfDrive((req) => {
          void runPerfDrive(req);
        }),
      );
    }

    // 前端可能刚被重载（HMR / WebView 崩溃恢复）：Rust 侧的窗口尺寸与 compact 标志
    // 都还在，但这里的 mode 会回到初始值 —— 若不同步，就会在 168x56 的窗里渲染
    // dashboard 视图（内容被裁掉、头部按钮落在窗外，用户自己点不回来）。
    // 因此按实际窗口宽度对齐模式，并重新同步把手与鼠标捕获。
    try {
      const win = getCurrentWindow();
      const [size, scale, pos, outer] = await Promise.all([
        win.innerSize(),
        win.scaleFactor(),
        win.outerPosition(),
        win.outerSize(),
      ]);
      if (size.width / scale <= PILL_RESTORE_MAX_W) {
        mode = "compact";
        // 贴边状态是**持久**的：Rust setup 会按 config.toml 里的 dock 锚点把窗口
        // 贴死回上次那条边。所以这里必须**问一次 Rust**当前落点贴没贴边，
        // 不能再假定「未贴边」——那是 dock 持久化之前的老前提。
        //
        // 假定错了的后果很隐蔽：窗口停在贴边位置，但按 floating 同步会让 Rust
        // **销毁把手**并把鼠标交回主窗，于是胶囊卡在屏幕边缘、没有把手可唤醒，
        // 看起来就是「贴边了但把手不见了」的半吊子状态。
        const side = await dockSideOf(pos.x, pos.y, outer.width, outer.height, null);
        if (side) {
          // 与退出前一致：胶囊滑出、把手在边缘接管鼠标。
          pillDocked = true;
          pillRevealed = false;
          pillExpanded = false;
        } else {
          pillDocked = false;
          pillRevealed = true;
        }
        await syncPeek();
        if (side) void emitPeekShow(side).catch(() => {});
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

  // 日历高亮焦点：在趋势页（TrendPanel → CalendarSection）消费，跨页共享。
  // 总览页卡片点击会预置它，切到趋势页即生效——这是日历迁走后，卡片↔日历
  // 在 400px 窗口无法同屏时的替代路径。
  let highlightKey = $state<string | null>(null);

  // Provider behind the floating detail overlay (hovered card). Rendered once
  // at window level so small cards (e.g. DeepSeek) can never clip its content.
  let detailSnapshot = $derived(
    hoveredId ? (snapshots.find((s) => s.provider_id === hoveredId) ?? null) : null,
  );

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
      : "app.allSources",
  );
  // 胶囊右侧品牌色芯片：账户强调色的低饱和底 + 同色文字展示完整短名，
  // flex:1 填满分隔线到右缘（固定布局、非自适应），消除右侧空白；
  // 超长自定义标签以省略号兜底，悬停 title 显示完整账户名。
  const badgeStyle = $derived.by(() => {
    const rgb = hexToRgb(focusedColor);
    if (!rgb) {
      return "background: var(--tum-surface-hover); border:1px solid rgba(255,255,255,.2); color:var(--tum-text-primary);";
    }
    return [
      `background: rgba(${rgb}, 0.16)`,
      `border: 1px solid rgba(${rgb}, 0.45)`,
      `color: ${focusedColor}`,
    ].join("; ");
  });
  const providerFullName = $derived(
    focusedSnapshot ? focusedSnapshot.provider_display_name : $t("app.allSources"),
  );

  let pillDragStart: { x: number; y: number; fromControl: boolean } | null = null;
  // 原生拖拽的**窗口**起点（屏幕物理坐标）：松手时用它算位移。指针起点
  // pillDragStart 是窗口内 client 坐标，拖动后指针已不在窗口内，算不出位移。
  // 必须在 pointerdown 里读——那时 startDragging() 还没调（它在 pointermove
  // 里越过阈值才触发），窗口停在原地，读到的就是起点。
  let pillDragOrigin: { x: number; y: number } | null = null;
  let pillDidDrag = false;

  // peek 双窗状态：pillDocked = 胶囊贴边（把手窗口存在）；pillRevealed = 层可见
  // （滑入态/浮动态）。浮动态：pillDocked=false 且 pillRevealed=true（层恒可见、无把手）。
  let pillSide = $state<PeekSide>("right");
  let pillRevealed = $state(false);
  let pillDocked = $state(false);
  // 收起动画进行中：此期间忽略把手 hover，避免刚收边就被立刻唤回（动画结束即恢复，
  // 不像旧的布防闩锁那样要求用户先把指针移开）。
  let pillCollapsing = $state(false);
  // 指针是否在胶囊层内：把手的 hover/leave 会与滑入动画交错，用这一位区分
  // 「擦过把手」与「确实离开胶囊」。
  let pillHovered = $state(false);
  // 原生拖拽进行中：冻结一切 hover 导致的收起判定与在途计时器，否则拖到一半
  // 会被收边链路（collapsePill + syncPeek("docked")）拽回屏幕边缘。
  let pillDragActive = $state(false);
  let pillExpanded = $state(false);

  // ── 按钮条显隐（方案 .scratch/glance-bench/issues/03）───────────────────
  // 原先只靠 CSS 的 `.pill:hover .pill__close { opacity: 1 }`。两个问题：
  //   1. **触摸设备上 `:hover` 永不触发**，关闭按钮永久不可达——不是
  //      「观感差一点」，是功能缺失。
  //   2. 指针一离开胶囊立刻消失，没有宽限期；胶囊顶角上划过去时按钮会闪。
  // 判定交给 pill-hover-state.ts 的纯函数（带 13 例单测），这里只做接线。
  let canHover = $state(true);
  /** 系统是否要求减弱动态效果。Svelte 的 transition 走 WAAPI，
   *  样式表的 `animation-duration !important` 管不到它，必须显式分支。 */
  let reducedMotion = $state(false);
  $effect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const hoverMq = window.matchMedia("(hover: hover)");
    const motionMq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => {
      canHover = hoverMq.matches;
      reducedMotion = motionMq.matches;
    };
    sync();
    hoverMq.addEventListener("change", sync);
    motionMq.addEventListener("change", sync);
    return () => {
      hoverMq.removeEventListener("change", sync);
      motionMq.removeEventListener("change", sync);
    };
  });

  let hoverState = $state(initialHoverState());
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;
  function clearHoverTimer() {
    if (hoverTimer !== null) {
      clearTimeout(hoverTimer);
      hoverTimer = null;
    }
  }
  /** 推一个事件进状态机，并在需要时挂上宽限计时。 */
  function pushHover(event: HoverEvent) {
    // untrack：读当前状态**不能**成为调用方 effect 的依赖。两个 effect 在
    // 同步调用链里调 pushHover，而 pushHover 读 hoverState、写 hoverState——
    // 不 untrack 的话，Svelte 会把 hoverState 算进依赖，而两个 effect 读的不是
    // hoverState（structure 读 pillInteractive、drag 读 pillDragActive/pillHovered），
    // 于是形成「写 hoverState → 触发 effect → 又写 hoverState」。
    //
    // 眼下之所以没死循环，纯粹是因为 reducer 的 no-op 分支返回**同一个引用**，
    // Svelte 5 对同引用赋值不再调度。那是条隐式的、未成文的不变量——任何常规
    // 重构（给 reducer 加个 lastEventAt 字段、no-op 时顺手展开一次）都会让每次
    // effect 运行产出新对象，随即 effect_update_depth_exceeded 把组件搞崩。
    // untrack 让正确性不再依赖它。回归由 pill-hover-state.test.ts 钉住。
    hoverState = reduceHover([event], untrack(() => hoverState));
    clearHoverTimer();
    if (hoverState.hideAt !== null) {
      // 快照进闭包：delay 与被 reduce 的初态必须来自同一时刻，否则
      // 「旧定时器套到新状态上」的时序耦合就藏在闭包里看不见了。
      const at = hoverState.hideAt;
      const snap = $state.snapshot(hoverState);
      hoverTimer = setTimeout(() => {
        hoverTimer = null;
        hoverState = reduceHover([{ type: "tick", at: Date.now() }], snap);
      }, Math.max(0, at - Date.now()));
    }
  }
  // 胶囊是否可交互——**显隐与 hover 判定必须同源**。原先两个表达式各算各的：
  // stripVisible 看 `pillExpanded`，interactive 看 `pillExpanded && !pillDocked`，
  // 后者多一条 `!pillDocked`，于是某些组合下按钮被显示出来、hover 逻辑却认为
  // 它不可交互（enter 被解析成 visible=false，leave 被提前 break）。
  const pillInteractive = $derived(pillExpanded && !pillDocked);
  // 无悬停能力的设备（触屏）一律常驻；展开且未贴边时也常驻（窗口够宽，
  // 藏按钮是倒退）。
  const stripVisible = $derived(!canHover || pillInteractive || hoverState.visible);
  $effect(() => {
    pushHover({ type: "structure", interactive: pillInteractive, at: Date.now() });
  });
  $effect(() => {
    // pointerInside 取既有的 pillHovered：冻结期丢弃的 enter 要靠它在
    // 解冻时补回，否则「拖完停在胶囊上准备关闭」这个最自然的序列会失灵。
    pushHover({
      type: "drag",
      active: pillDragActive,
      pointerInside: pillHovered,
      at: Date.now(),
    });
  });
  // 与既有的 clearDockTimer / clearMiniTimer 同一纪律：在途的宽限计时器
  // 不能跨组件卸载/模式切换存活。
  onDestroy(clearHoverTimer);
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


  /** 与后端同步「谁捕获鼠标 + 胶囊贴哪一边」。三态由 pillDocked/pillRevealed 推出。 */
  async function syncPeek(): Promise<void> {
    if (mode !== "compact") return;
    const nextState = pillDocked
      ? pillRevealed
        ? "revealed"
        : "docked"
      : "floating";
    try {
      pillSide = await syncPeekWindow(nextState);
      // 贴边收起态顺带让把手显形自愈：把手页只在收到 peek-show 时清除自己的
      // is-hidden，若之前因异常时序（例如重载、模式竞态）停在隐藏态，它就
      // 既看不见也点不到 —— 那样胶囊再也唤不出来。这里与停靠状态一并纠正。
      if (pillDocked && !pillRevealed) void emitPeekShow(pillSide).catch(() => {});
    } catch {
      // 窗口正在切换模式 / 已被关闭：保持当前状态即可。
    }
  }

  // ── 窗口尺寸写入（唯一入口）──────────────────────────────────────────
  //
  // 胶囊态的尺寸由前端写（expandPill / collapsePill），全窗态的尺寸由 Rust 写
  // （set_window_mode）。两边的 setSize **不排队**，就可能乱序落地：一次慢的
  // 胶囊 resize 晚于模式切换落地，把 400×680 的全窗态窗口又改回 168 宽 ——
  // 于是 dashboard 内容被塞进胶囊宽度的窗口，表现为「标签栏的中文竖排成一列、
  // 标题被裁掉、下拉框出界」（用户实测截图：168×150 的窗口里渲染着 dashboard，
  // 而 168×150 正是胶囊**展开态**的尺寸 —— 那笔 expandPill 的写入晚到了一步）。
  //
  // 三条规则：
  //
  // 1. **序列化**：所有尺寸写排成一条链，后写的赢；
  // 2. **落地前核对模式**：一笔写只在自己被请求时的模式仍然成立时才落地。
  //    模式一变，在途的旧写自动作废；
  // 3. **模式切换收尾补写**：切换完成后补写一笔当前模式应有的尺寸，
  //    把「排队期间进来、切换之后落地」的旧写纠正回来。
  //
  // 队列本体在 lib/window-size.ts::createSizeWriteQueue（有单测钉住竞态本身）。
  //
  /**
   * 模式切换进行中。
   *
   * 切模式时，**在途的胶囊尺寸写必须立刻作废**：那笔尺寸属于旧形态，
   * 落地就会把刚被 Rust 改成 400×680 的全窗态窗口又改回 168 宽。
   *
   * 为什么不能只看 `mode`：`mode` 是在 Rust 的 IPC **返回之后**才翻的
   * （窗口尺寸要先就位，UI 再切，才不会出现「dashboard 先在胶囊宽度的
   * 窗口里渲染一帧」）。而 Rust 那次 IPC 的往返期间，仍可能有展开计时器
   * 触发、发起一笔新的胶囊尺寸写——那笔写看到 `mode` 还是旧的，就以为
   * 自己还有效。于是要有一个**比 UI 更早**翻转的开关。
   *
   * 它同时驱动 `.shell--switching`：切换期间内容整个藏起来——那一两帧里
   * 窗口尺寸已经变了、形态还没翻，两种内容放进不匹配的窗口都难看
   * （dashboard 挤成竖排一列 / 胶囊玻璃底铺满整窗）。窗口是透明的，
   * 藏起来 = 看不见，比硬塞干净。
   */
  let switchingMode = $state(false);

  /**
   * 尺寸写队列（唯一入口）。
   *
   * 裁决 = 「切模式进行中 → 一律作废」+「请求模式与当前模式不一致 → 作废」。
   * 后一条的判据是 `shouldApplySizeWrite`（有单测钉住）。
   */
  const sizeQueue = createSizeWriteQueue({
    shouldApply: (forMode) =>
      !switchingMode && shouldApplySizeWrite(forMode, mode),
    apply: (size) =>
      getCurrentWindow().setSize(new LogicalSize(size.width, size.height)),
  });

  /**
   * 写窗口尺寸。
   *
   * @param forMode 请求这笔写时的模式。落地前会核对当前模式：不一致就
   *   **跳过**（那笔尺寸属于旧模式，落地就会把窗口改成错的样子）。
   * @param size 请求时刻算出的尺寸（按请求时刻的状态算，展开/收起的中间态
   *   不会被后一笔吃掉）。
   */
  function writeWindowSize(forMode: Mode, size: LogicalSize): Promise<void> {
    return sizeQueue.enqueue(forMode, { width: size.width, height: size.height });
  }

  /** 当前模式应有的窗口尺寸（模式切换收尾时补写用）。 */
  function sizeForMode(m: Mode): LogicalSize {
    return m === "compact"
      ? new LogicalSize(PILL_COLLAPSED_W, PILL_COLLAPSED_H)
      : new LogicalSize(DASHBOARD_W, DASHBOARD_H);
  }

  async function expandPill() {
    if (mode !== "compact" || snapshots.length === 0) return;
    const gen = ++pillResizeGen;
    const height = pillExpandedHeight(snapshots.length);
    // ⚠️ 顺序要紧：先挂内容、**再**扩窗，不要 await setSize 之后才置位。
    // 原先是「窗口先长高 → 空窗一拍 → 内容再浮现」，割裂感正是本次要消除的。
    // 先置位后，fly 的 opacity:0 起点会盖住窗口尚矮时内容被压扁的那几帧，
    // 展开动画与扩窗两条时间线并行——与本次滑入改动用的是同一套方法。
    pillExpanded = true;
    // 改尺寸单独打点（issues/05 §2.3）：`dockPill.shrink` 只说明「收起这一段慢」，
    // 拆出 setSize 才能分清「改尺寸本身偶发慢」与「改尺寸撞上了动画合成」。
    const sizeSpan = markSpan("expandPill.setSize");
    try {
      await writeWindowSize("compact", new LogicalSize(PILL_COLLAPSED_W, height));
    } catch {
      // IPC 失败：回退到折叠，避免「内容在、窗口没长高」的压扁态。
      if (gen === pillResizeGen) pillExpanded = false;
    } finally {
      markEnd(sizeSpan);
    }
  }

  async function collapsePill(): Promise<void> {
    const gen = ++pillResizeGen;
    pillExpanded = false;
    // 同上：把 setSize 从「collapsePill 整体」里单独拆出来量。
    const sizeSpan = markSpan("collapsePill.setSize");
    try {
      await writeWindowSize(
        "compact",
        new LogicalSize(PILL_COLLAPSED_W, PILL_COLLAPSED_H),
      );
    } catch {
      // 忽略：窗口可能正处于模式切换中。
    } finally {
      markEnd(sizeSpan);
    }
  }

  /** 层可见时延迟展开明细（与滑入/悬浮动画错开 120ms）。 */
  function scheduleExpandPill() {
    if (mode !== "compact" || pillExpanded) return;
    clearMiniTimer();
    const gen = peekGen;
    miniTimer = setTimeout(() => {
      miniTimer = null;
      if (gen !== peekGen) return;
      void expandPill();
    }, PILL_REVEAL_MINI_MS);
  }

  /**
   * 唤出守卫：把手 hover 能唤出胶囊的三个前提（已贴边 / 没在收起 / 没在拖拽）。
   *
   * 抽成函数是为了让**真人路径与探针驱动器共用同一份判定**——驱动器若另抄一份，
   * 将来改了守卫两份就会悄悄漂移，测出来的数据便不再代表真实交互。
   */
  function canRevealPill(): boolean {
    return pillDocked && !pillCollapsing && !pillDragActive;
  }

  /**
   * 把手 hover → 唤出胶囊。探针驱动器走的是**同一个函数**（issues/05），
   * 所以驱动出来的数据与手点同源。返回 true = 真的唤出了。
   */
  async function revealOnPeekHover(): Promise<boolean> {
    if (!canRevealPill()) return false;
    await revealPill();
    return true;
  }

  /** 把手（或已滑入的胶囊）被唤醒：解除穿透 → 滑入 → 120ms 后展开明细。 */
  async function revealPill() {
    if (mode !== "compact") return;
    const gen = ++peekGen;
    clearDockTimer();
    clearMiniTimer();
    const span = markSpan("revealPill");
    pillRevealed = true;
    try {
      await syncPeek(); // 贴边滑入态：主窗接管鼠标，把手退出交互
    } finally {
      markEnd(span);
    }
    if (gen !== peekGen) return;
    scheduleExpandPill();
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

  /** 收起采用「时间线重叠」三段式：t=0 置滑出态，胶囊立刻开始 200ms 滑出
   *  （此时窗口尚未缩小，滑出全程可见）；t=120ms 滑出未结束就让把手提前接回
   *  鼠标并从屏幕边缘复现；t=200ms 滑出播完后再缩窗裁剪。三段互相重叠，
   *  消除「胶囊瞬消 → 空窗 → 把手突现」的割裂感。
   *
   *  ⚠️ 不要在这里加 `if (!pillRevealed) return` 之类的短路：即使胶囊从未滑入
   *  （指针 60ms 内刷过把手就离开，把手已把自己隐去，而 peek-hover 被防抖抑制
   *  或被随后的 peek-leave 取代），也必须照常走到 emitPeekShow —— 这是把手唯一
   *  的「复活」路径。少了它，把手会永久停在 opacity:0 + pointer-events:none，
   *  胶囊再也无法被唤醒（等于应用不可达）。 */
  async function dockPill() {
    // 拖拽期间绝不允许收起链路介入：collapsePill/syncPeek 会把窗口拽回边缘。
    if (pillDragActive) return;
    const gen = ++peekGen;
    clearMiniTimer();
    // 时间线探针（issues/05 §1.3）：只打标记，不改时序、不改行为。
    // span 测的是**宿主侧 wall-clock**，与 LoAF（主窗渲染进程主线程）互补——
    // 把手窗创建发生在宿主事件循环线程，可能占住宿主却不产生任何主窗 LoAF。
    const span = markSpan("dockPill");
    pillCollapsing = true;
    pillRevealed = false; // t=0：立即触发 .pill-layer.is-docked 的 180ms 滑出
    setTimeout(async () => {
      const lead = markSpan("dockPill.peekLead");
      // ⚠️ 世代守卫早退也必须 markEnd（放在 finally）：快速悬停/收起的 churn
      // 高频触发 gen 冲突，而「有时会卡顿」恰恰最可能伴随 churn——
      // 只在正常路径闭合会系统性丢弃最相关的样本，测量结果偏向顺利路径。
      try {
        if (gen !== peekGen) return;
        // t=120ms：滑出未结束就把主窗切回 docked 布局，让把手提前接管。
        // 先取回最新贴靠边（用户可能刚把胶囊拖到屏幕另一侧），再把边随
        // peek-show 发给把手页，让它纠正圆角朝向并复现自己。
        await syncPeek(); // docked=true：把手接回鼠标
        if (gen !== peekGen) return;
        void emitPeekShow(pillSide).catch(() => {});
      } finally {
        markEnd(lead);
      }
    }, PILL_PEEK_LEAD_MS);
    setTimeout(async () => {
      const shrink = markSpan("dockPill.shrink");
      try {
        // t=180ms：滑出播完再缩窗。先清锁再校验代际：即便本代已被唤醒取代，
        // 也不能让 pillCollapsing 永久卡在 true（否则 revealPill 被永久抑制）。
        pillCollapsing = false;
        if (gen !== peekGen) return;
        await collapsePill();
      } finally {
        markEnd(shrink);
        markEnd(span);
      }
    }, PILL_SLIDE_MS + PILL_SHRINK_DELAY_MS);
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

  /** 指针进入胶囊层：取消待收起的停靠计时；层可见时安排明细展开（浮动态也生效）。 */
  function onPillPointerEnter() {
    if (mode !== "compact") return;
    pillHovered = true;
    clearDockTimer();
    if (!pillDocked || pillRevealed) scheduleExpandPill();
  }

  /** 指针离开胶囊层：贴边态沿用 320ms 收起链路；浮动态只收明细、不隐藏窗口。 */
  function onPillPointerLeave() {
    if (mode !== "compact") return;
    pillHovered = false;
    // 拖拽中窗口跟着指针走，中途的 leave/enter 事件不可信，一律忽略。
    if (pillDragActive) return;
    if (pillDocked) {
      scheduleDockPill();
      return;
    }
    // 浮动态层恒可见：延迟后仅把明细收回，窗口留在原地。
    clearDockTimer();
    const gen = peekGen;
    dockTimer = setTimeout(() => {
      dockTimer = null;
      if (gen !== peekGen) return;
      void collapsePill();
    }, PILL_DOCK_DELAY_MS);
  }

  function onPillPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    // 按下即冻结收边链路：否则刚按下就可能被 320ms 的收起计时器收走。
    clearDockTimer();
    pillDragStart = {
      x: event.clientX,
      y: event.clientY,
      fromControl: (event.target as HTMLElement).closest("button") !== null,
    };
    pillDidDrag = false;
    void (async () => {
      try {
        const p = await getCurrentWindow().outerPosition();
        pillDragOrigin = { x: p.x, y: p.y };
      } catch {
        pillDragOrigin = null;
      }
    })();
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
      pillDragActive = true;
      clearDockTimer();
      void setPillDragging(true);
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
      // pointerup 能收到时的快路径；Rust 侧「左键已松开 + 停稳」是可靠兜底。
      pillDragActive = false;
      void setPillDragging(false);
      void settlePillAfterDrag();
    }
  }

  /** 拖拽结束：贴哪条边由 Rust 的 `dock_side_of` 判（四条边，角落按拖拽主方向）；
   *  判为 None（离四条边都远）→ 恢复常态浮动。 */
  async function settlePillAfterDrag() {
    if (mode !== "compact") return;
    pillDragActive = false;
    void setPillDragging(false);
    clearDockTimer();
    // 落点判定统一交给 Rust 的 nearest_side（四条边 + 角落按拖拽主方向裁决）。
    // 前端只负责把「拖了多远」报上去——自己再算一遍必然和 Rust 的阈值/坐标
    // 空间对不上，这正是旧实现只有 min(距左, 距右) 的原因。
    let side: PeekSide | null = null;
    try {
      const win = getCurrentWindow();
      const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
      const drag = pillDragOrigin
        ? ([pos.x - pillDragOrigin.x, pos.y - pillDragOrigin.y] as [number, number])
        : null;
      side = await dockSideOf(pos.x, pos.y, size.width, size.height, drag);
    } catch {
      side = null; // 读不到窗口几何：按浮动处理，至少保证胶囊可交互
    }
    pillDragOrigin = null;

    if (side) {
      // 贴边：先置收起态并同步 docked —— Rust 立刻贴死边缘 + 建把手 + 主窗穿透，
      // 随后进入正常收起链路滑出，因此胶囊滑出时窗口已经贴边。
      pillDocked = true;
      pillRevealed = false;
      void syncPeek().then(() => dockPill());
    } else {
      // 浮动：层保持可见、主窗接管鼠标，把手窗口由 Rust 销毁。
      pillDocked = false;
      pillRevealed = true;
      pillCollapsing = false;
      void syncPeek();
    }
  }

  // ── 探针驱动器（issues/05）────────────────────────────────────────────
  // 无人值守地跑「贴边 → 唤出 → 收起」循环，用于复现偶发的贴边卡顿。
  // 生产构建（VITE_PERF 未设）下整段不注册：监听器不存在、命令不可达。

  /** 驱动器是否正在跑。并发两轮循环会互相踩状态机，必须拒绝。 */
  let perfDriving = false;

  /**
   * 把胶囊置为 docked 态（驱动器的前置条件）。
   *
   * 为什么**不**直接调 settlePillAfterDrag()：它要去问 Rust「该贴哪条边」
   * （dock_side_of），依据是真实窗口几何与拖拽位移。驱动器既没有拖拽，也不该
   * 改动用户当前的贴靠位置；而若窗口离四条边都远，side 会是 null，它反而把
   * 胶囊打回浮动态、销毁把手窗口——驱动器的前提直接没了。
   *
   * 为什么这样置位不与世代号打架：这里**不碰** peekGen / pillResizeGen，只改
   * pillDocked / pillRevealed 两个布尔，再调 syncPeek() + dockPill()——后两者
   * 各自按自己的规则推进世代号（dockPill 会 ++peekGen），因此不会与在途的
   * 世代守卫冲突。
   */
  async function ensureDockedForDrive(): Promise<void> {
    if (pillDocked) return;
    pillDocked = true;
    pillRevealed = false;
    await syncPeek();
    await dockPill();
  }

  /**
   * 跑一轮无人值守的「贴边 → 唤出 → 收起」循环，返回给 Rust 的回执。
   *
   * 驱动器只像「按按钮」一样调用现有函数（revealPill / dockPill），
   * 不重写状态机、不碰世代号、不动 pillDragActive 冻结机制。
   */
  async function doPerfDrive(req: PerfDriveRequest): Promise<PerfDriveDone> {
    // 只在 compact（胶囊）态下才有意义：dashboard 态没有把手窗口，
    // revealPill / syncPeek 都是空操作，测出来的数据毫无意义。
    if (mode !== "compact") {
      return { rounds: 0, error: "主窗不在 compact 模式，贴边交互不存在" };
    }
    const rounds = Number.isFinite(req.rounds) ? Math.floor(req.rounds) : 0;
    if (rounds < 1) {
      return { rounds: 0, error: `rounds 非法：${String(req.rounds)}` };
    }
    const rawInterval = Number(req.intervalMs);
    const intervalMs =
      Number.isFinite(rawInterval) && rawInterval > 0
        ? Math.floor(rawInterval)
        : 1000; // 缺省 1s（方案 §1.4「每轮间隔 1 秒」）

    await ensureDockedForDrive();

    // 动态 import：生产构建里这段根本不会被执行，驱动器模块也不会被下载。
    const { drivePeekCycles } = await import("./lib/perf-drive");
    const result = await drivePeekCycles({
      rounds,
      intervalMs,
      // 必须与真实「指针离开」链路一致：scheduleDockPill 的宽限期。
      dockDelayMs: PILL_DOCK_DELAY_MS,
      // 等 dockPill 的收起时间线走完，下一轮的唤出守卫才可能成立。
      slideMs: PILL_SLIDE_MS,
      hooks: {
        // 与真人 hover 走**同一个函数**。
        reveal: () => revealOnPeekHover(),
        // 与 scheduleDockPill 的定时器到期效果同源。
        dock: () => dockPill(),
        ready: () => canRevealPill(),
      },
      // warmup 轮之后清空采样：冷启动（首次建 peek 窗口）不进统计。
      resetStats: () => perfReset(),
    });
    return result.error
      ? { rounds: result.rounds, error: result.error }
      : { rounds: result.rounds };
  }

  /** 「perf-drive」事件的处理器：跑驱动器，然后把结果回执给 Rust。 */
  async function runPerfDrive(req: PerfDriveRequest): Promise<void> {
    if (perfDriving) {
      await emitPerfDriveDone({
        rounds: 0,
        error: "驱动器已在运行，已忽略本次请求",
      }).catch(() => {});
      return;
    }
    perfDriving = true;
    let ack: PerfDriveDone;
    try {
      ack = await doPerfDrive(req);
    } catch (err) {
      ack = {
        rounds: 0,
        error: err instanceof Error ? err.message : String(err),
      };
    } finally {
      perfDriving = false;
    }
    await emitPerfDriveDone(ack).catch(() => {});
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
    // ⓪ 先竖起「切换中」的闸门：此刻起，任何在途的胶囊尺寸写都作废。
    //    必须早于 UI 切换（`mode` 要等 Rust 改完尺寸才翻），否则那几毫秒里
    //    仍可能冒出一笔新的胶囊尺寸写。
    switchingMode = true;
    try {
      // ① 先排空已排队的尺寸写：让它们**先**落地，Rust 的改尺寸才是最后一笔
      //    物理写入。反过来的话，一笔在途的胶囊尺寸写可能晚于 Rust 落地，
      //      把全窗态窗口又改回胶囊宽度（见 writeWindowSize 处的现场描述）。
      await sizeQueue.drain();
      // ② Rust 改尺寸 + 按锚点重新贴边。
      await setWindowMode(next);
      // ③ UI 切模式。必须排在 ② 之后：窗口尺寸已经就位，才不会出现
      //    「dashboard 先在 168 宽的胶囊窗口里渲染一帧」。
      mode = next;
      // compact：常态浮动且层可见（Rust 侧不建把手，只把鼠标交回主窗）；
      // dashboard：两者复位（set_window_mode 已在 Rust 里销毁把手并恢复鼠标交互）。
      pillDocked = false;
      pillRevealed = next === "compact";
      pillCollapsing = false;
      pillExpanded = false;
      pillResizeGen++;
      peekGen++;
      void setPillDragging(false);
      clearDockTimer();
      clearMiniTimer();
    } finally {
      // 闸门必须**无条件**放下：留着它会让之后所有尺寸写都被跳过，
      // 窗口再也改不动大小（比原 bug 更糟，且更难察觉）。
      switchingMode = false;
    }
    // ④ 补写当前模式的尺寸：①排空之前、②之后进来的任何旧尺寸写（例如
    //    展开计时器刚好在这中间触发）都靠这一笔纠正回来。
    await writeWindowSize(mode, sizeForMode(mode));
    await syncPeek();
    // 记住显示形态：Rust 下次启动会在显示窗口前按此设定尺寸并拉回屏内，
    // 因此这里必须持久化，否则「常态即胶囊」在重启后会丢。
    if (settings) {
      settings = { ...settings, compact_mode: next === "compact" };
      try {
        await saveSettings(settings);
      } catch {
        // 持久化失败不影响本次切换本身。
      }
    }
  }

  async function refresh() {
    await forceRefresh();
    snapshots = await markIpc(() => getUsage());
    lastRefreshAt = Date.now();
    void markFrameStart();
  }

  async function closeApp() {
    await getCurrentWindow().close();
  }

  function formatError(err: ProviderError): string {
    const tr = get(t);
    if (err.Network) return tr("app.error.network", { msg: err.Network.message });
    if (err.Auth) return tr("app.error.auth", { msg: err.Auth.message });
    if (err.Parse) return tr("app.error.parse", { msg: err.Parse.message });
    if (err.RateLimited) return tr("app.error.rateLimited");
    if (err.NotConfigured) return tr("app.error.notConfigured");
    if (err.Internal) return tr("app.error.internal", { msg: err.Internal.message });
    return tr("app.error.unknown");
  }

  function extractErrors(states: Record<string, ProviderState>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [id, st] of Object.entries(states)) {
      if (st.last_error) out[id] = formatError(st.last_error);
    }
    return out;
  }
</script>

<main
  class="shell"
  class:shell--compact={mode === "compact"}
  class:shell--switching={switchingMode}
  data-tauri-drag-region
  oncontextmenu={(e) => e.preventDefault()}
>
  {#if mode === "compact"}
    <div
      class="pill-layer"
      class:is-docked={pillDocked && !pillRevealed}
      data-side={pillSide}
      role="presentation"
      onpointerenter={onPillPointerEnter}
      onpointerleave={onPillPointerLeave}
    >
      <div
        class="pill"
        data-tone={pillTone}
        class:is-strip-visible={stripVisible}
        onpointerdown={onPillPointerDown}
        onpointermove={onPillPointerMove}
        onpointerup={onPillPointerUp}
        onpointerenter={() => pushHover({ type: "enter", at: Date.now() })}
        onpointerleave={() => pushHover({ type: "leave", at: Date.now() })}
        oncontextmenu={(e) => e.preventDefault()}
        role="group"
        aria-label={$t("app.pill.ariaLabel")}
      >
        <div class="pill__main">
          <button
            type="button"
            class="pill__restore"
            aria-label={$t("app.pill.restore")}
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
              >{$t(focusedName)}</span
            >
          </button>
          <button
            type="button"
            class="pill__close"
            title={$t("app.pill.close")}
            aria-label={$t("app.pill.close")}
            onpointerdown={(e) => e.stopPropagation()}
            onclick={(e) => {
              e.stopPropagation();
              void closeApp();
            }}
          ><Icon name="x" size={11} /></button>
        </div>
        {#if pillExpanded}
          <!-- 明细原本是硬切：{#if} 挂上即出现、消失即消失。
               只给**入场**加动画（`in:` 而非 `transition:`）：后者是双向的，
               收起时会播 160ms 飞出才卸载，而 collapsePill() 同期在缩窗——
               内容仍占位就被窗口边缘硬裁，比硬切更糟。
               减弱动效时退化为原地淡入，不做位移。 -->
          <div
            class="pill__mini"
            in:fly={{ y: reducedMotion ? 0 : -6, duration: reducedMotion ? 120 : 160 }}
          >
            <MiniPanel {snapshots} {actives} countdown={displayRemaining} accentFor={(id) => accentById[id]} />
          </div>
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
        <button class="shell__btn" onclick={refresh} title={$t("app.refreshNow")}><Icon name="refresh" size={13} /></button>
        <button
          type="button"
          class="shell__btn shell__btn--router"
          class:is-on={routerOn}
          onclick={toggleRouter}
          title={routerTitle}
        >
          <span
            class="router-dot"
            class:router-dot--on={routerOn && routerListening}
            class:router-dot--warn={routerOn && !routerListening && !routerBindError}
            class:router-dot--err={!!routerBindError}
          ></span>⇄
        </button>
        <button class="shell__btn" onclick={() => openSettings()} title={$t("app.settings")}><Icon name="settings" size={13} /></button>
        <button class="shell__btn" onclick={toggleMode} title={$t("app.collapseToMini")}><Icon name="maximize" size={13} /></button>
        <button class="shell__btn shell__btn--close" onclick={closeApp} title={$t("common.close")}><Icon name="x" size={13} /></button>
      </div>
    </header>

    <div class="view-row" data-tauri-drag-region={false}>
      {#each tabs as tab (tab.key)}
        <button
          type="button"
          class="view-tab"
          class:is-active={view === tab.key}
          onclick={() => (view = tab.key)}
        >{$t(tab.label)}</button>
      {/each}
    </div>

    {#if view === "overview"}
      {#if snapshots.length > 0}
      <div class="focus-row" data-tauri-drag-region={false}>
        <PillsOrSelect
          items={[{ id: "all", label: $t("common.all") }, ...snapshots.map((s) => ({ id: s.provider_id, label: s.provider_display_name }))]}
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
          <p>{$t("app.loadingUsage")}</p>
          <p class="shell__hint">{$t("app.loadingHint")}</p>
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
              now={now.getTime()}
              {pollIntervalSec}
              focused={focus === snap.provider_id}
              accent={accentById[snap.provider_id]}
              countdown={displayRemaining}
              onHover={onCardHover}
              onSelect={() => {
                const kind = snap.provider_id.split("-")[0];
                const series = ledgerSeriesForKind(kind, snap.provider_id);
                // 无按日数据的 kind（差分类）：不猜、不静默改视图，交给
                // DetailCard 说明。点击既不改焦点也不改高亮。
                if (!series) return;
                // 再点一次已聚焦的卡片 = 取消高亮，让焦点可撤销。
                if (focus === snap.provider_id && highlightKey !== null) {
                  focus = "all";
                  highlightKey = null;
                  return;
                }
                focus = snap.provider_id;
                // 日历已迁至趋势页，卡片点击不再直接驱动它；但仍把高亮焦点
                // 预置到趋势页，切过去即可看到该 provider 的逐日构成。
                // 日历恒为合并口径（服务端优先），**所有**注册 provider 都能
                // 在日历里出现——火山走自己的 tokens 日账，OpenAI/xAI 走本机
                // 工具归因（它们的 USD 日账不进 token 日历）。故判据是"有没有
                // provider key"，不再是"有没有账户日账口径"。
                highlightKey = highlightKeyForKind(kind);
              }}
            />
          </div>
        {/each}
      {/if}
    </section>

    {:else if view === "trend"}
      {#if snapshots.length > 0}
        <!-- 日历已迁入 TrendPanel（bind:highlightKey）：总览卡片点击预置的
             高亮焦点，切到趋势页即生效——卡片↔日历的跨页联动由此保留。 -->
        <TrendPanel bind:highlightKey />
      {:else}
        <div class="shell__empty"><p>{$t("app.noUsageData")}</p></div>
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
        <span class="shell__next-label">{$t("app.nextRefresh")}</span>
        <span class="shell__next-value">{nextRefreshLabel}</span>
      </span>
      <span class="shell__count">{$t("app.providerCount", { n: snapshots.length })}</span>
    </footer>

    {#if detailSnapshot}
      <div
        class="detail-overlay"
        role="group"
        data-tauri-drag-region={false}
        style={`top:${overlayY}px;visibility:${overlayReady ? "visible" : "hidden"};`}
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

  /* 模式切换进行中：内容**整个藏起来**。
     窗口是透明的，所以「藏起来」= 看不见；「硬塞」= 一团挤扁的 UI。
     这一两帧里窗口尺寸已经变了、而形态还没翻（或反过来），两种内容放进
     不匹配的窗口都难看：
     - 全窗态的 dashboard 挤进 168 宽 → 标签栏中文竖排成一列、标题被裁；
     - 胶囊态的玻璃底铺满 400 宽的整窗 → 一大块半透明圆角矩形。
     藏起来反而干净：切换只有一两帧，藏掉的那一瞬本来就看不见。 */
  .shell--switching {
    visibility: hidden;
  }

  /* 胶囊整体平移：收起时滑出窗外，唤醒时滑回 (0,0)。
     贴左边时方向镜像。 */
  .pill-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    /* 兜底值：token 缺失时退化成原行为，而不是过渡整体静默消失
       （未定义的自定义属性会让 transition-duration 回落 0s）。 */
    /* 贴边滑入（对标 Glance OPEN）：
       - 位置 260ms 强 ease-out
       - 透明度 140ms，只有位置的一半——面板滑到一半就已经「在那儿了」。
         两者等长会读成「空白很久，然后啪一下出现」，这是滑入不丝滑的主因。 */
    opacity: 1;
    /* 强制独立合成层。透明大窗上若不提升层，opacity + transform 的过渡会退回
       主线程逐帧重绘——实测结果是「滑到底突然显形」，淡入完全看不出来。 */
    will-change: transform, opacity;
    transition:
      transform var(--tum-dur-slide-in, 260ms) var(--tum-ease-slide-in, cubic-bezier(0.16, 1, 0.3, 1)),
      opacity var(--tum-dur-slide-in-fade, 140ms) linear;
  }

  /* 贴边收起（对标 Glance CLOSE）：180ms ease-in，起步慢、迅速钻边。
     淡出时长与位移**等长**（此前是 120ms vs 180ms）：淡出走完时 ease-in 的位移
     才走了约一半，前半程几乎没动就已经看不见，读成「原地消散」。
     等长之后「一边移动一边变淡」同时发生，钻边感才成立。 */
  .pill-layer.is-docked {
    opacity: 0;
    transition:
      transform var(--tum-dur-slide-out, 180ms) var(--tum-ease-slide-out, cubic-bezier(0.4, 0, 1, 1)),
      opacity var(--tum-dur-slide-out, 180ms) linear;
  }

  .pill-layer.is-docked[data-side="right"] {
    transform: translateX(100%);
  }

  .pill-layer.is-docked[data-side="left"] {
    transform: translateX(-100%);
  }

  .pill-layer.is-docked[data-side="bottom"] {
    transform: translateY(100%);
  }

  .pill-layer.is-docked[data-side="top"] {
    transform: translateY(-100%);
  }

  /* 「减弱动态效果」：tokens.css 里的全局规则把所有过渡压成 0.001ms 瞬时——
     那比不开动画更硬。贴边滑入/滑出单独降级为**纯淡入淡出**：位移瞬时到位
     （不滑动），透明度保留 120ms 柔和过渡。与 Glance 的 PLAIN_FADE 同思路
     （panel.rs:99）。

     ⚠️ 必须带 `!important`：全局那条 `transition-duration: 0.001ms !important`
     是 important 声明，而本条展开出的 transition-duration 是普通声明——
     级联里 important 无条件压过普通声明，与特异性无关（`*` 的零特异性救不了）。
     不加 `!important` 的话这段降级完全失效，而 svelte-check 与单测都测不出
     CSS 级联。两条同为 important 时才比特异性：Svelte 的作用域类让
     `.pill-layer.svelte-xxx` 稳压 `*`。

     这里**只**保留淡入。不要顺手把离屏 transform 也覆盖成 translateX(0)：
     opacity:0 的元素仍参与命中测试，那会让停靠态有一层透明层原地吃掉指针，
     胶囊再也唤不回来。位移交给上面的 0.001ms 瞬时即可。 */
  @media (prefers-reduced-motion: reduce) {
    .pill-layer,
    .pill-layer.is-docked {
      transition: opacity var(--tum-dur-reduced-fade, 120ms) ease !important;
    }
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
    /* 不折行：窗口被意外压窄时（模式切换竞态的现场）标题被裁已经够糟，
       折成竖排一列则是「看不懂这是什么」。宁可裁掉尾巴。 */
    white-space: nowrap;
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
    background: var(--tum-surface);
  }

  .shell__btn {
    width: 22px;
    height: 22px;
    border: 1px solid var(--tum-border);
    background: var(--tum-surface);
    color: var(--tum-text-secondary);
    border-radius: var(--tum-radius-sm);
    cursor: pointer;
    font-size: var(--tum-font-size-sm);
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

  /* TokenRouter 快速开关：⇄ + 状态点（绿=服务中 / 琥珀=启用未监听 / 红=端口异常）。 */
  .shell__btn--router.is-on {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
  }
  .router-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--tum-text-muted);
    margin-right: 2px;
  }
  .router-dot--on {
    background: var(--tum-ok);
  }
  .router-dot--warn {
    background: var(--tum-warn);
  }
  .router-dot--err {
    background: var(--tum-crit);
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
    font-size: var(--tum-font-size-xs);
    font-family: var(--tum-font);
    padding: 3px 10px;
    border-radius: var(--tum-radius-pill);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
    /* 不折行：中文标签在窄容器里会「一个字一行」竖排（实测：窗口被压到
       168px 时「趋势/工具/模型/设备」全成了竖排的一列，完全读不出来）。
       宁可横向溢出被裁，也不要竖排。 */
    white-space: nowrap;
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
     展开时按 lib/window-size.ts 的 pillExpandedHeight（60 + 30n）增高。
     半径 28 = 收起态高 56 的一半：端部为完整半圆，与内层徽章（22px 高、
     11px 半径）的胶囊端同族；展开态内容缩进 10/8px，28px 也不会裁到明细行。
     data-tone 三态：<80% 静默 / 80-95% 琥珀 / >=95% 红色呼吸（spec §5.2 CompactPill）。 */
  .pill {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0 10px;
    border-radius: 28px;
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

  /* 明细的入场容器：只负责承载过渡，不改变布局。
     时长引用 token，缓动用 spring（与滑入同一族的手感）。 */
  .pill__mini {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
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
    background: var(--tum-surface);
  }

  .pill__percent {
    font-size: var(--tum-font-size-sm);
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
    border-radius: var(--tum-radius-pill);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--tum-font-size-xs);
    font-weight: 600;
    font-family: var(--tum-font);
    letter-spacing: 0.2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
  }

  .pill__close {
    /* 关闭钮悬浮右上角（不占布局空间）。显隐由 .is-strip-visible 驱动，
       **不再交给 CSS `:hover`**——触摸设备上它永不触发，按钮会永久不可达，
       那是功能缺失而不是观感问题。时长引用 token，不写字面量（issues/01）。 */
    position: absolute;
    top: 4px;
    right: 4px;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-xs);
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity var(--tum-dur-fast, 150ms) var(--tum-ease-spring),
      background var(--tum-dur-fast, 150ms) ease,
      color var(--tum-dur-fast, 150ms) ease;
  }

  .pill.is-strip-visible .pill__close {
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

  /* Floating detail overlay: rendered at window level so the full detail
     (rows + chart) is never clipped by a small card. It is docked along the
     bottom (above the footer) so it covers the calendar-heatmap area — hovering
     a card swaps that area for the account's detail — and, crucially, does NOT
     cover the provider cards being scanned. Hit-testing passes through to what
     is underneath, so the overlay can never cause hover flicker.
     Height is still measured before reveal (overlayReady) so there is no
     one-frame flash; the width is full-bleed so the detail reads as a panel
     rather than a floating chip. */
  .detail-overlay {
    position: fixed;
    left: 14px;
    right: 14px;
    z-index: 60;
    max-height: calc(100vh - 96px);
    pointer-events: none;
  }

  .detail-overlay :global(.detail) {
    padding: 10px 12px;
    max-height: calc(100vh - 96px);
    overflow-y: auto;
  }

  /* The overlay itself is pointer-transparent so hovering it passes through to
     the cards below and can never cause flicker. The action row opts back in so
     "refresh this account" stays clickable. */
  .detail-overlay :global(.detail__actions) {
    pointer-events: auto;
  }
</style>
