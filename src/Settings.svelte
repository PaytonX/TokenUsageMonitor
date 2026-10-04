<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    getProviders,
    getSettings,
    saveSettings,
    saveCredentials,
    deleteCredentials,
    testProvider,
    testProxy,
    detectCodexToken,
    upsertAccount,
    removeAccount,
    closeSettings,
    forceRefresh,
    getDeviceReport,
    getExchangeRates,
    refreshExchangeRates,
    getRouterStatus,
    onSettingsChanged,
    fetchUpstreamModels,
    type ProviderCatalog,
    type Preset,
    type PresetSubMode,
    type RouterCandidate,
    type RouterRoute,
    type RouterProtocol,
    type AccountMeta,
    type Settings,
    type Credentials,
    type TestResult,
    type ProxyTestResult,
    type RatesSnapshot,
    type RouterSettings,
    type RouterStatus,
    defaultRouterSettings,
    emitTabsChanged,
    readHiddenTabs,
    readAggMode,
    writeAggMode,
    setTabEnabled,
    type PageTab,
  } from "./lib";
  import { CURRENCIES, USD_RATES, applyRatesSnapshot, type Currency } from "./lib/currency";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";

  type Tab = "general" | "accounts" | "interaction" | "network" | "router" | "about";

  /** 可做精确覆盖的工具标识（须与后端 local::roots 的 key 一致）。 */
  const TOOL_DATA_DIR_KEYS = [".claude", ".codex", ".zcode", ".minimax", "cherry-studio"] as const;
  const TOOL_DATA_LABELS: Record<string, string> = {
    ".claude": "Claude Code（.claude）",
    ".codex": "Codex（.codex）",
    ".zcode": "ZCode（.zcode）",
    ".minimax": "MiniMax Code（.minimax）",
    "cherry-studio": "Cherry Studio（.cherrystudio）",
  };

  /** One account as rendered in the Settings UI. Meta edits are batched into
   * `save_settings`; credential ops hit the backend immediately by
   * `instance_id`. */
  type AccountForm = {
    meta: AccountMeta;
    hasCredentials: boolean;
    live: boolean;
    /** BearerKey fields */
    apiKey: string;
    /** AccessKeySecret fields */
    accessKey: string;
    secretKey: string;
    /** LocalToken field (e.g. Codex ~/.codex/auth.json) */
    token: string;
    credsDirty: boolean;
    testing: boolean;
    testResult: TestResult | null;
    saved: boolean;
    error: string | null;
  };

  let tab = $state<Tab>("general");
  let catalog = $state<ProviderCatalog | null>(null);
  let settings = $state<Settings | null>(null);
  let forms = $state<AccountForm[]>([]);
  /** Current preset whose choice menu is expanded (null = none). */
  let presetMenuFor = $state<Preset | null>(null);
  // 账户页（2026-09 重构）：主视图是紧凑账户列表，完整编辑表单只渲染选中的
  // 那一个账户——此前所有账户的表单全部展开，账户一多页面就无限拉长。
  let selectedAccountId = $state<string | null>(null);
  // 「添加账户」预设网格从常驻改为按需弹层：Provider 数量随版本增长，
  // 常驻网格会把已配置账户列表挤到首屏之外。
  let pickerOpen = $state(false);
  let selectedForm = $derived(
    forms.find((f) => f.meta.instance_id === selectedAccountId) ?? null,
  );
  let pollInterval = $state<number>(0);
  let ringWindow = $state<string>("auto");
  let edgeSnap = $state(true);
  let autostart = $state(false);
  let notifyEnabled = $state(true);
  let notifyWarn = $state<number>(80);
  let notifyCrit = $state<number>(95);
  let countdownMode = $state(false);
  let displayCurrency = $state("auto");
  // 页签显隐：趋势/工具/模型可关闭（总量恒常驻）。立即持久化到 localStorage 并
  // 广播 tabs-changed，让主面板重读。
  let hiddenTabs = $state<PageTab[]>(readHiddenTabs());
  function tabEnabled(t: PageTab): boolean {
    return !hiddenTabs.includes(t);
  }
  function toggleTab(t: PageTab, enabled: boolean) {
    setTabEnabled(t, enabled);
    hiddenTabs = readHiddenTabs();
    void emitTabsChanged();
  }
  // 全端汇总模式：纯前端展示偏好（localStorage），写后广播让面板即时换源。
  let aggMode = $state(readAggMode());
  function toggleAggMode(on: boolean) {
    writeAggMode(on);
    aggMode = readAggMode();
    void emitTabsChanged();
  }
  let hubMode = $state("off");
  let hubPort = $state(43210);
  let hubBase = $state("");
  let reportOn = $state(false);
  let hubToken = $state("");
  // 网络代理（spec §3.4）：空字符串 = 不设置代理（跟随系统环境变量）。
  let proxyEnabled = $state(false);
  let proxyUrl = $state("");
  // 本机工具数据目录（重定位）：额外根目录列表 + 每工具精确覆盖。
  let toolDataRoots = $state<string[]>([]);
  let toolDataDirs = $state<Record<string, string>>({});
  let proxyTesting = $state(false);
  let proxyResult = $state<ProxyTestResult | null>(null);
  // TokenRouter（spec: docs/superpowers/specs/2026-10-04-token-router-design.md）：
  // 编辑态配置 + 运行态快照（健康/候选徽标）。token 由后端保存时补发，前端只展示。
  let routerCfg = $state<RouterSettings>(defaultRouterSettings());
  let routerStatus = $state<RouterStatus | null>(null);
  let tokenCopiedFor = $state<string | null>(null);
  // 汇率（B.6/C6）：编辑态覆盖值 + 最近一次生效快照（供 placeholder/更新时间展示）。
  let rateOverrides = $state<Record<string, number | undefined>>({});
  let ratesSnapshot = $state<RatesSnapshot | null>(null);
  let rateRefreshing = $state(false);
  /** 可覆盖币种（USD 恒为 1，不提供覆盖输入）。 */
  const OVERRIDE_CODES = CURRENCIES.filter((c) => c.code !== "USD");
  let appVersion = $state("");
  let saving = $state(false);
  let savedFlash = $state(false);
  let busy = $state<string | null>(null);
  let genericError = $state<string | null>(null);
  /** Account pending a second "删除" click to confirm. */
  let deleteConfirmId = $state<string>("");

  const presetMap = $derived(
    new Map((catalog?.presets ?? []).map((p) => [p.kind, p])),
  );

  const canSave = $derived(settings !== null);

  function authKindFor(kind: string): "bearer_key" | "access_key_secret" | "local_token" {
    return presetMap.get(kind)?.auth_kind ?? "bearer_key";
  }

  function isAccessKey(kind: string): boolean {
    return authKindFor(kind) === "access_key_secret";
  }

  function isLocalToken(kind: string): boolean {
    return authKindFor(kind) === "local_token";
  }

  function buildForms(): AccountForm[] {
    return (catalog?.accounts ?? []).map((a) => ({
      meta: {
        instance_id: a.instance_id,
        provider_kind: a.provider_kind,
        label: a.label,
        accent_color: a.accent_color,
        enabled: a.enabled,
        note: a.note,
      },
      hasCredentials: a.has_credentials,
      live: a.live,
      apiKey: "",
      accessKey: "",
      secretKey: "",
      token: "",
      credsDirty: false,
      testing: false,
      testResult: null,
      saved: false,
      error: null,
    }));
  }

  async function refreshAll() {
    try {
      const [cat, s] = await Promise.all([getProviders(), getSettings()]);
      catalog = cat;
      settings = s;
      pollInterval = s.poll_interval_seconds ?? 0;
      ringWindow = s.ring_window ?? "auto";
      edgeSnap = s.edge_snap ?? true;
      autostart = s.autostart ?? false;
      notifyEnabled = s.notify_enabled ?? true;
      notifyWarn = s.notify_warn_percent ?? 80;
      notifyCrit = s.notify_crit_percent ?? 95;
      countdownMode = s.countdown_mode ?? false;
      displayCurrency = s.display_currency ?? "auto";
      hubMode = s.hub_mode ?? "off";
      hubPort = s.hub_port ?? 43210;
      hubBase = s.hub_base ?? "";
      reportOn = s.report_on ?? false;
      hubToken = s.hub_token ?? "";
      proxyUrl = s.proxy_url ?? "";
      proxyEnabled = !!s.proxy_url;
      rateOverrides = { ...(s.rate_overrides ?? {}) };
      toolDataRoots = [...(s.tool_data_roots ?? [])];
      toolDataDirs = { ...(s.tool_data_dirs ?? {}) };
      routerCfg = { ...defaultRouterSettings(), ...(s.router ?? {}) };
      void refreshRouterStatus();
      forms = buildForms();
      // 选中项失效（账户被删/首次加载）时回落到第一个账户。
      if (!selectedAccountId || !forms.some((f) => f.meta.instance_id === selectedAccountId)) {
        selectedAccountId = forms[0]?.meta.instance_id ?? null;
      }
      // 生效汇率快照（更新时间 / 告警展示用），并让本窗口立即采用。
      getExchangeRates()
        .then((snap) => {
          ratesSnapshot = snap;
          applyRatesSnapshot(snap);
        })
        .catch(() => {});
      // 拉取后端版本号用于"关于"展示（失败时静默保留空）。
      getDeviceReport()
        .then((d) => (appVersion = d.version))
        .catch(() => {});
    } catch (e) {
      genericError = String(e);
    }
  }

  function findPreset(kind: string): Preset | undefined {
    return presetMap.get(kind);
  }

  function displayNameFor(kind: string): string {
    return findPreset(kind)?.display_name ?? kind;
  }

  function buildCredentials(f: AccountForm): Credentials {
    if (isLocalToken(f.meta.provider_kind)) {
      return { kind: "local_token", token: f.token.trim() };
    }
    if (isAccessKey(f.meta.provider_kind)) {
      return {
        kind: "access_key_secret",
        access_key: f.accessKey.trim(),
        secret_key: f.secretKey.trim(),
      };
    }
    return { kind: "bearer_key", api_key: f.apiKey.trim() };
  }

  function hasCredentialInput(f: AccountForm): boolean {
    if (isLocalToken(f.meta.provider_kind)) {
      return f.token.trim().length > 0;
    }
    if (isAccessKey(f.meta.provider_kind)) {
      return f.accessKey.trim().length > 0 && f.secretKey.trim().length > 0;
    }
    return f.apiKey.trim().length > 0;
  }

  // --- Account lifecycle ----------------------------------------------------

  async function addAccount(preset: Preset) {
    presetMenuFor = null;
    busy = `add-${preset.kind}`;
    genericError = null;
    try {
      const newId = await upsertAccount({
        instance_id: "",
        provider_kind: preset.kind,
        label: preset.display_name,
        accent_color: preset.default_accent,
        enabled: true,
      });
      await refreshAll();
      // 新账户直接进入编辑视图，并收起选择弹层。
      selectedAccountId = newId;
      pickerOpen = false;
      tab = "accounts";
    } catch (e) {
      genericError = String(e);
    } finally {
      busy = null;
    }
  }

  /** 多模式预设卡片 → 弹子模式选择菜单；单模式预设直接添加。 */
  function onPresetClick(preset: Preset) {
    if (preset.sub_modes?.length) {
      presetMenuFor = presetMenuFor?.kind === preset.kind ? null : preset;
    } else {
      void addAccount(preset);
    }
  }

  /** 从已展开菜单的预设，按选定子模式派生 account kind 后添加。 */
  function addSubMode(mode: PresetSubMode) {
    if (!presetMenuFor) return;
    void addAccount({ ...presetMenuFor, kind: mode.kind, display_name: mode.label });
  }

  async function deleteAccount(f: AccountForm) {
    if (deleteConfirmId !== f.meta.instance_id) {
      // First click asks for confirmation with a "再次点击以确认" affordance.
      deleteConfirmId = f.meta.instance_id;
      return;
    }
    deleteConfirmId = "";
    busy = f.meta.instance_id;
    genericError = null;
    try {
      await removeAccount(f.meta.instance_id);
      await refreshAll();
    } catch (e) {
      genericError = String(e);
    } finally {
      busy = null;
    }
  }

  // --- Credential operations (immediate, by instance_id) -------------------

  async function testConnection(f: AccountForm) {
    if (!hasCredentialInput(f)) {
      f.error = "请先填写凭证";
      f.testResult = null;
      return;
    }
    f.testing = true;
    f.error = null;
    f.testResult = null;
    const result = await testProvider(f.meta.provider_kind, buildCredentials(f));
    f.testing = false;
    f.testResult = result;
    if (result.ok) {
      f.credsDirty = true;
    }
  }

  async function saveCredentialsFor(f: AccountForm) {
    if (!hasCredentialInput(f)) {
      f.error = "请先填写凭证";
      return;
    }
    f.error = null;
    try {
      await saveCredentials(f.meta.instance_id, buildCredentials(f));
      f.credsDirty = false;
      f.hasCredentials = true;
      f.saved = true;
      setTimeout(() => (f.saved = false), 2000);
    } catch (e) {
      f.error = String(e);
    }
  }

  async function clearCredentialsFor(f: AccountForm) {
    f.apiKey = "";
    f.accessKey = "";
    f.secretKey = "";
    f.token = "";
    f.testResult = null;
    f.error = null;
    try {
      await deleteCredentials(f.meta.instance_id);
      f.hasCredentials = false;
      f.credsDirty = false;
    } catch (e) {
      f.error = String(e);
    }
  }

  async function detectCodexFor(f: AccountForm) {
    f.error = null;
    try {
      const detected = await detectCodexToken();
      if (!detected) {
        f.error = "未检测到本地 Codex 登录（~/.codex/auth.json）。请先登录 Codex CLI。";
        return;
      }
      f.token = detected.token;
      f.credsDirty = true;
    } catch (e) {
      f.error = String(e);
    }
  }

  async function handleTestProxy() {
    const url = proxyUrl.trim();
    if (!url) return;
    proxyTesting = true;
    proxyResult = null;
    try {
      proxyResult = await testProxy(url);
    } catch (e) {
      proxyResult = { ok: false, status: null, error: String(e) };
    } finally {
      proxyTesting = false;
    }
  }

  /** 线路序号圆点（①..⑤），超出回退为数字。 */
  const CIRCLES = ["①", "②", "③", "④", "⑤"];

  /** 「服务设置」折叠态（低频参数，不占日常视野）。 */
  let svcOpen = $state(false);
  /** 路由链编辑区折叠态：收起时只显示链路摘要 + Key + 当前走谁。 */
  let chainOpen = $state(true);
  /** API Key 复制反馈。 */
  let tokenCopied = $state(false);

  // --- TokenRouter（v2：单路由 + 四层判定） -------------------------------------

  /** 线路账户 kind → 上游根地址预填。选账户即省一次粘贴；随时可改成中转站。 */
  const ROUTER_BASE_PRESETS: Record<string, string> = {
    anthropic: "https://api.anthropic.com",
    openai: "https://api.openai.com",
    deepseek: "https://api.deepseek.com",
    kimi: "https://api.moonshot.cn",
    kimi_global: "https://api.moonshot.ai",
    xai: "https://api.x.ai",
    minimax: "https://api.minimaxi.com",
    minimax_api: "https://api.minimaxi.com",
  };

  let routeSeq = 0;
  function nextRouteId(): string {
    routeSeq += 1;
    return `route-${Date.now().toString(36)}-${routeSeq}`;
  }

  function accountKindOf(instanceId: string): string {
    return (
      catalog?.accounts.find((a) => a.instance_id === instanceId)?.provider_kind ?? ""
    );
  }

  /** 生效路由（v2 单路由：只有第一条）。未配置时为 null。 */
  let route = $derived(routerCfg.routes[0] ?? null);

  function ensureRoute(): RouterRoute {
    let r = routerCfg.routes[0];
    if (!r) {
      r = {
        id: nextRouteId(),
        name: "主力路由",
        protocol: "openai",
        on: true,
        token: "",
        candidates: [],
      };
      routerCfg.routes = [r];
    }
    return r;
  }

  /** 选账户时自动预填该 kind 的官方根地址（选人即选上游）。 */
  function onCandidateAccountChanged(cand: RouterCandidate, instanceId: string) {
    cand.account = instanceId;
    const base = ROUTER_BASE_PRESETS[accountKindOf(instanceId)];
    if (base) cand.base_url = base;
  }

  function addCandidate() {
    ensureRoute().candidates.push({ account: "", model: "", base_url: "" });
  }

  function removeCandidate(index: number) {
    const r = route;
    if (r) r.candidates = r.candidates.filter((_, i) => i !== index);
  }

  function moveCandidate(index: number, dir: -1 | 1) {
    const r = route;
    if (!r) return;
    const to = index + dir;
    if (to < 0 || to >= r.candidates.length) return;
    const [c] = r.candidates.splice(index, 1);
    r.candidates.splice(to, 0, c);
  }

  async function copyRouteToken(token: string) {
    try {
      await navigator.clipboard.writeText(token);
      tokenCopied = true;
      setTimeout(() => (tokenCopied = false), 1500);
    } catch {
      /* 剪贴板不可用：静默，用户可手动选择文本复制。 */
    }
  }

  let routerUrlCopied = $state(false);
  async function copyRouterUrl() {
    try {
      await navigator.clipboard.writeText(`http://127.0.0.1:${routerCfg.port}`);
      routerUrlCopied = true;
      setTimeout(() => (routerUrlCopied = false), 1500);
    } catch {
      /* 剪贴板不可用：静默。 */
    }
  }

  async function refreshRouterStatus() {
    try {
      routerStatus = await getRouterStatus();
    } catch {
      routerStatus = null;
    }
  }

  // 模型列表拉取：按「账户|上游地址」缓存，同一上游的线路共享一份结果。
  let modelOptions = $state<Record<string, string[]>>({});
  let modelFetching = $state<Record<string, boolean>>({});
  let modelFetchError = $state<Record<string, string>>({});
  let modelPickerFor = $state<string | null>(null);

  function modelKey(account: string, baseUrl: string): string {
    return `${account}|${baseUrl}`;
  }

  async function handleFetchModels(cand: RouterCandidate, protocol: RouterProtocol) {
    const key = modelKey(cand.account, cand.base_url);
    if (!cand.account || !cand.base_url) {
      modelFetchError[key] = "先选择账户并填写上游地址，再拉取模型列表";
      return;
    }
    modelFetching[key] = true;
    modelFetchError[key] = "";
    try {
      modelOptions[key] = await fetchUpstreamModels(protocol, cand.account, cand.base_url);
      modelPickerFor = key;
    } catch (e) {
      modelFetchError[key] = String(e);
    } finally {
      modelFetching[key] = false;
    }
  }

  function pickModel(cand: RouterCandidate, model: string) {
    cand.model = model;
    modelPickerFor = null;
  }

  /** 编辑态 → 落盘态：数值夹取 + 去掉缺账户/缺上游地址的线路。**路由本身不
   *  删除**——没配全线路的路由保留（请求会得到明确报错），避免吞掉用户填了
   *  一半的内容。模型名可留空（= 透传工具原始模型名）。token 留空由后端补发。 */
  function sanitizeRouter(cfg: RouterSettings): RouterSettings {
    const clampInt = (v: number, lo: number, hi: number, dflt: number) => {
      const n = Math.round(v);
      return Number.isFinite(n) ? Math.max(lo, Math.min(hi, n)) : dflt;
    };
    return {
      enabled: cfg.enabled,
      port: clampInt(cfg.port, 1, 65535, 43211),
      proactive_threshold_percent: clampInt(cfg.proactive_threshold_percent, 0, 95, 20),
      error_cooldown_secs: clampInt(cfg.error_cooldown_secs, 5, 86400, 300),
      conn_breaker_count: clampInt(cfg.conn_breaker_count, 1, 20, 3),
      probe_start_secs: clampInt(cfg.probe_start_secs, 10, 3600, 60),
      probe_max_attempts: clampInt(cfg.probe_max_attempts, 1, 20, 5),
      routes: cfg.routes.slice(0, 1).map((r) => ({
        id: r.id || nextRouteId(),
        name: r.name.trim() || "未命名路由",
        protocol: r.protocol,
        on: r.on,
        token: r.token.trim(),
        candidates: r.candidates
          .map((c) => ({
            account: c.account.trim(),
            model: c.model.trim(),
            base_url: c.base_url.trim().replace(/\/+$/, ""),
            ...(Number.isFinite(c.plan_limit_tokens_daily) &&
            (c.plan_limit_tokens_daily ?? 0) > 0
              ? { plan_limit_tokens_daily: c.plan_limit_tokens_daily }
              : {}),
            ...(Number.isFinite(c.monthly_cost_limit) &&
            (c.monthly_cost_limit ?? 0) > 0
              ? { monthly_cost_limit: c.monthly_cost_limit }
              : {}),
          }))
          .filter((c) => c.account && c.base_url),
      })),
    };
  }

  const LINE_STATE_LABELS: Record<string, string> = {
    ok: "可用",
    low_quota: "低余量",
    full: "已用尽",
    cooldown: "冷却",
    unusable: "凭据缺失",
  };

  function lineBadge(state: string): string {
    switch (state) {
      case "ok":
        return "line-badge--ok";
      case "low_quota":
      case "cooldown":
        return "line-badge--warn";
      default:
        return "line-badge--bad";
    }
  }

  /** 线路剩余条的颜色：与切走线阈值联动（低余量/告警/正常）。 */
  function remainingColor(pct: number | undefined): string {
    if (pct === undefined || pct === null) return "var(--tx3)";
    const t = routerCfg.proactive_threshold_percent;
    if (pct <= t) return "#ef4444";
    if (pct <= t * 2) return "#f59e0b";
    return "#22c55e";
  }

  function cooldownUntilText(iso?: string): string {
    if (!iso) return "";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const secs = Math.max(0, Math.round((d.getTime() - Date.now()) / 1000));
    return secs > 90 ? `${Math.round(secs / 60)} 分钟后` : `${secs} 秒后`;
  }

  function probeInText(iso?: string): string {
    if (!iso) return "";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const secs = Math.max(0, Math.round((d.getTime() - Date.now()) / 1000));
    return secs > 90 ? `${Math.round(secs / 60)} 分钟后` : `${secs} 秒后`;
  }

  /** 切走线仪表：当前承载流量的线路的最小窗口剩余（无刻度数据时 null）。 */
  let gaugeRemaining = $derived.by(() => {
    const st = routerStatus;
    if (!st?.route || st.route.active_index === null) return null;
    return st.route.candidates[st.route.active_index]?.remaining_percent ?? null;
  });

  function onGaugeInput(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(v)) routerCfg.proactive_threshold_percent = v;
  }

  // --- Bulk save --------------------------------------------------------------

  /** Apply the countdown-mode switch immediately so the dashboard ring flips
   * without waiting for a full save. Only persists countdown_mode. */
  async function applyCountdownNow(e: Event) {
    const checked = (e.currentTarget as HTMLInputElement).checked;
    countdownMode = checked;
    if (!settings) return;
    try {
      await saveSettings({ ...settings, countdown_mode: checked });
      settings = { ...settings, countdown_mode: checked };
    } catch (err) {
      genericError = String(err);
    }
  }

  function clampPercent(n: number): number {
    if (Number.isNaN(n)) return 0;
    return Math.max(0, Math.min(100, Math.round(n)));
  }

  /** 把编辑态覆盖值整理为后端格式：未填 / 非法（≤0、NaN）的项剔除。 */
  function buildRateOverrides(): Record<string, number> {
    const out: Record<string, number> = {};
    for (const { code } of OVERRIDE_CODES) {
      const v = rateOverrides[code];
      if (v !== undefined && Number.isFinite(v) && v > 0) out[code] = v;
    }
    return out;
  }

  /** 覆盖留空时展示的"自动值"提示：live 缓存 > 内置默认。 */
  function autoRateFor(code: string): string {
    const row = ratesSnapshot?.rates.find(
      (r) => r.code === code && r.source !== "override",
    );
    return row ? String(row.rate) : String(USD_RATES[code as Currency]);
  }

  function formatRateTime(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  /** 手动强制拉取一次汇率并落库；结果经后端广播同步到所有窗口。 */
  async function handleRateRefresh() {
    rateRefreshing = true;
    try {
      const snap = await refreshExchangeRates();
      ratesSnapshot = snap;
      applyRatesSnapshot(snap);
      genericError = null;
    } catch (e) {
      genericError = String(e);
    } finally {
      rateRefreshing = false;
    }
  }

  async function persistSettings(): Promise<boolean> {
    if (!settings) return true;
    genericError = null;
    try {
      const next: Settings = {
        ...settings,
        accounts: forms.map((f) => f.meta),
        poll_interval_seconds: pollInterval,
        ring_window: ringWindow,
        edge_snap: edgeSnap,
        autostart,
        notify_enabled: notifyEnabled,
        notify_warn_percent: clampPercent(notifyWarn),
        notify_crit_percent: clampPercent(notifyCrit),
        countdown_mode: countdownMode,
        display_currency: displayCurrency,
        hub_mode: hubMode,
        hub_port: hubPort,
        hub_base: hubBase,
        report_on: reportOn,
        hub_token: hubToken,
        // Mark it as "the user has made a choice about the secret" — including
        // choosing an empty one to run without auth. Without this flag the
        // backend would mint a secret on every hub start, undoing a
        // deliberately cleared field.
        hub_token_configured: true,
        rate_overrides: buildRateOverrides(),
        proxy_url: proxyEnabled && proxyUrl.trim() ? proxyUrl.trim() : null,
        // 本机工具数据目录：空串条目剔除（允许保留一个空输入框）。
        tool_data_roots: toolDataRoots.map((r) => r.trim()).filter(Boolean),
        tool_data_dirs: Object.fromEntries(
          Object.entries(toolDataDirs)
            .map(([k, v]) => [k, v.trim()])
            .filter(([, v]) => v.length > 0),
        ),
        router: sanitizeRouter(routerCfg),
      };
      await saveSettings(next);
      // 后端在落盘前会补发空 token（ensure_route_tokens）：入参 next 里 token
      // 还是空的，必须回读后端的落盘结果，路由卡的 API Key 才显示得出来。
      // 同时刷新运行态（启停/端口变化即时反映）。
      try {
        const fresh = await getSettings();
        settings = fresh;
        routerCfg = { ...defaultRouterSettings(), ...(fresh.router ?? {}) };
        void refreshRouterStatus();
      } catch {
        /* 回读失败不阻塞保存：下次打开设置页会重读。 */
      }
      // Mirror the display-currency choice into localStorage so every window
      // (model panel, detail cards) resolves it instantly without an IPC read.
      try {
        localStorage.setItem("tum.currency", displayCurrency);
      } catch {
        /* ignore */
      }
      settings = next;
      // 汇率：合并新覆盖值并广播其它窗口（force=false，离线不阻塞）；失败忽略。
      // 放在 persistSettings 里，让"保存"与"保存并关闭"两条路径都覆盖。
      try {
        const snap = await getExchangeRates();
        ratesSnapshot = snap;
        applyRatesSnapshot(snap);
      } catch {
        /* ignore */
      }
      return true;
    } catch (e) {
      genericError = String(e);
      return false;
    }
  }

  async function handleSaveAll() {
    saving = true;
    const ok = await persistSettings();
    if (ok) {
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 2000);
      // 触发一次刷新，让主面板立即反映新启用的账户；失败不阻塞保存。
      try {
        await forceRefresh();
      } catch (e) {
        genericError = String(e);
      }
    }
    saving = false;
  }

  async function handleClose() {
    // 关闭只做快速持久化并立即隐藏窗口，不等待网络轮询（save_settings 后端
    // 已发 settings_wake，主面板会自动重新轮询），避免"保存中"卡顿假死感。
    await persistSettings();
    await closeSettings();
  }

  function formatTestResult(r: TestResult | null): string {
    if (!r) return "";
    if (r.ok) return "✓ 连接成功";
    return `✗ ${r.message}`;
  }

  function testResultClass(r: TestResult | null): string {
    if (!r) return "";
    return r.ok ? "test-ok" : "test-fail";
  }

  // Keep the flipped "确认删除" state harmless if the user switches accounts.
  function deleteLabel(f: AccountForm): string {
    return deleteConfirmId === f.meta.instance_id ? "确认删除" : "删除账户";
  }

  // 关键：窗口打开时必须先加载数据，否则 catalog/settings 恒为 null，
  // 会导致账户与预设都不显示、所有设置开关失效、保存按钮禁用。
  onMount(() => {
    refreshAll();
    // 其它窗口（主面板 ⇄ 快速开关）改了设置 → 同步路由编辑态，避免下一次
    // 本窗口保存把外部改动又改回去。token 由后端补发后也经此回填。
    onSettingsChanged((fresh) => {
      settings = fresh;
      routerCfg = { ...defaultRouterSettings(), ...(fresh.router ?? {}) };
      void refreshRouterStatus();
    });
  });
async function handleMinimize() {
    await getCurrentWindow().minimize();
  }
  async function handleToggleMaximize() {
    await getCurrentWindow().toggleMaximize();
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && (pickerOpen || presetMenuFor)) {
      pickerOpen = false;
      presetMenuFor = null;
    }
  }}
/>

<main class="settings" oncontextmenu={(e) => e.preventDefault()}>
  <header class="settings__header">
    <div class="settings__brand">
      <span class="settings__dot"></span>
      <span class="settings__title">TokenUsageMonitor</span>
    </div>
    <div class="settings__win">
      <button type="button" class="settings__win-btn" aria-label="最小化" onclick={handleMinimize} onpointerdown={(e) => e.stopPropagation()}>—</button>
      <button type="button" class="settings__win-btn" aria-label="最大化/还原" onclick={handleToggleMaximize} onpointerdown={(e) => e.stopPropagation()}>▢</button>
      <button class="settings__close" onclick={handleClose} aria-label="保存并关闭">
        ✕
      </button>
    </div>
  </header>

  <div class="settings__body">
    <nav class="settings__nav">
      <button
        class="nav-item"
        class:is-active={tab === "general"}
        onclick={() => (tab = "general")}
      >
        通用
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "accounts"}
        onclick={() => (tab = "accounts")}
      >
        账户与额度
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "interaction"}
        onclick={() => (tab = "interaction")}
      >
        交互与通知
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "network"}
        onclick={() => (tab = "network")}
      >
        网络
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "router"}
        onclick={() => {
          tab = "router";
          void refreshRouterStatus();
        }}
      >
        路由
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "about"}
        onclick={() => (tab = "about")}
      >
        关于与诊断
      </button>
    </nav>

    <section class="settings__content">
      {#if genericError}
        <div class="global-error">{genericError}</div>
      {/if}

      {#if tab === "general"}
        <div class="pane">
          <h2 class="pane__title">通用</h2>

          <div class="section">
            <h3 class="section__title">轮询间隔</h3>
            <label class="interval">
              <input
                type="number"
                min="0"
                max="3600"
                step="30"
                bind:value={pollInterval}
              />
              <span class="interval__hint">秒 · 0 = 使用默认（5 分钟）</span>
            </label>
          </div>

          <div class="section">
            <h3 class="section__title">环形用量窗口</h3>
            <p class="hint">主面板与迷你胶囊的百分比圆环基于哪个窗口计算。</p>
            <label class="interval">
              <select class="interval-select" bind:value={ringWindow}>
                <option value="auto">自动（各来源最紧张窗口）</option>
                <option value="five_hour">5 小时窗口</option>
                <option value="daily">当日窗口</option>
                <option value="weekly">周用量窗口</option>
                <option value="monthly">月度窗口</option>
              </select>
            </label>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">倒计时模式（显示剩余量）</span>
                <span class="behavior-hint">关闭时圆环显示已用量；开启后显示剩余量，便于估算还能用多久。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" onchange={applyCountdownNow} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">展示币种</h3>
            <p class="hint">模型页签与详情卡中估算成本的显示币种。“自动”默认人民币（CNY）。</p>
            <label class="interval">
              <select class="interval-select" bind:value={displayCurrency}>
                <option value="auto">自动（人民币 CNY）</option>
                <option value="CNY">人民币 (CNY)</option>
                <option value="USD">美元 (USD)</option>
                <option value="TWD">新台币 (TWD)</option>
                <option value="HKD">港币 (HKD)</option>
                <option value="JPY">日元 (JPY)</option>
                <option value="EUR">欧元 (EUR)</option>
                <option value="GBP">英镑 (GBP)</option>
              </select>
            </label>
          </div>

          <div class="section">
            <h3 class="section__title">汇率覆盖</h3>
            <p class="hint">应用每 6 小时检查一次自动汇率（缓存 24 小时）；在此可将某币种固定为手动值（每 1 USD 兑该币种），留空使用自动值，离线时回退内置默认。</p>
            <div class="rate-grid">
              {#each OVERRIDE_CODES as c (c.code)}
                <label class="field">
                  <span class="field__label">{c.label}</span>
                  <input
                    class="field__input"
                    type="number"
                    step="any"
                    min="0"
                    placeholder={autoRateFor(c.code)}
                    bind:value={rateOverrides[c.code]}
                  />
                </label>
              {/each}
            </div>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">
                  {#if ratesSnapshot?.fetched_at}
                    自动汇率更新于 {formatRateTime(ratesSnapshot.fetched_at)}
                  {:else}
                    尚未拉取自动汇率（使用内置默认值）
                  {/if}
                </span>
                <span class="behavior-hint">
                  {ratesSnapshot?.warning ?? "覆盖值在保存设置后生效，并同步到所有窗口。"}
                </span>
              </div>
              <button class="btn btn--ghost" type="button" disabled={rateRefreshing} onclick={handleRateRefresh}>
                {rateRefreshing ? "刷新中…" : "立即刷新"}
              </button>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">页签显示</h3>
            <p class="hint">「总量」页签始终显示；其余页签可在此关闭，关闭后该页签不显示、相关功能不启用。</p>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">全端汇总模式</span>
                <span class="behavior-hint">
                  趋势 / 工具 / 模型页签改为显示全部设备的合并用量（按设备去重，成本不参与聚合）。需多端同步在线。
                </span>
              </div>
              <label class="toggle">
                <input
                  type="checkbox"
                  checked={aggMode}
                  onchange={(e) => toggleAggMode((e.target as HTMLInputElement).checked)}
                />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
            {#each [
              { key: "trend", label: "趋势", hint: "跨 Provider 逐日用量看板" },
              { key: "tools", label: "工具", hint: "本机 AI 工具（Claude Code / MiniMax Code / Hermes…）用量" },
              { key: "models", label: "模型", hint: "按模型维度聚合的 token / 成本统计" },
              { key: "devices", label: "设备", hint: "本机设备信息与多端同步" },
            ] as t (t.key)}
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">{t.label}</span>
                  <span class="behavior-hint">{t.hint}</span>
                </div>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={tabEnabled(t.key as PageTab)}
                    onchange={(e) => toggleTab(t.key as PageTab, (e.currentTarget as HTMLInputElement).checked)}
                  />
                  <span class="toggle__track"><span class="toggle__thumb"></span></span>
                </label>
              </div>
            {/each}
          </div>
        </div>

      {:else if tab === "accounts"}
        <div class="pane">
          <h2 class="pane__title">账户与额度</h2>
          <p class="hint">
            每个账户独立轮询、独立启停；凭证保存在 Windows 凭据管理器（DPAPI
            加密），不写磁盘明文。点击账户行展开编辑，支持添加多个同一来源的账户。
          </p>

          {#if presetMenuFor}
            <div
              class="preset-menu-backdrop"
              onclick={() => (presetMenuFor = null)}
              aria-hidden="true"
            ></div>
          {/if}

          <div class="section">
            <h3 class="section__title">
              已配置账户
              <span class="section__count">{forms.length}</span>
            </h3>

            {#if forms.length === 0}
              <div class="empty">还没有账户。点击下方「添加账户」从预设创建。</div>
            {:else}
              <div class="acct-list">
                {#each forms as f (f.meta.instance_id)}
                  <div
                    class="acct-row"
                    class:acct-row--active={f.meta.instance_id === selectedAccountId}
                    style="--acct-accent: {f.meta.accent_color}"
                    role="button"
                    tabindex="0"
                    aria-pressed={f.meta.instance_id === selectedAccountId}
                    data-enabled={f.meta.enabled}
                    onclick={(e) => {
                      // 点在启停开关上时不触发选中（开关有自己的语义）。
                      if ((e.target as HTMLElement).closest(".toggle")) return;
                      selectedAccountId = f.meta.instance_id;
                    }}
                    onkeydown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        selectedAccountId = f.meta.instance_id;
                      }
                    }}
                  >
                    <span class="acct-row__logo" aria-hidden="true">
                      <ProviderLogo kind={f.meta.provider_kind} size={18} accent={f.meta.accent_color} />
                    </span>
                    <span class="acct-row__label" title={f.meta.note || f.meta.label}>
                      {f.meta.label || "未命名账户"}
                    </span>
                    <span class="acct-row__kind">{displayNameFor(f.meta.provider_kind)}</span>
                    {#if f.hasCredentials}
                      <span class="acct-row__dot" title="凭证已存" aria-label="凭证已存"></span>
                    {/if}
                    <span
                      class="account__badge acct-row__live"
                      class:account__badge--live={f.live}
                      class:account__badge--off={!f.live}
                    >
                      {f.live ? "轮询中" : "已停止"}
                    </span>
                    <label class="toggle" title={f.meta.enabled ? "已启用" : "已停用"}>
                      <input
                        type="checkbox"
                        checked={f.meta.enabled}
                        onchange={(e) => (f.meta.enabled = (e.target as HTMLInputElement).checked)}
                      />
                      <span class="toggle__track"><span class="toggle__thumb"></span></span>
                    </label>
                  </div>
                {/each}
              </div>
            {/if}

            <button
              class="btn btn--ghost acct-add"
              disabled={busy?.startsWith("add-") ?? false}
              onclick={() => {
                presetMenuFor = null;
                pickerOpen = true;
              }}
            >
              ＋ 添加账户
            </button>
          </div>

          {#if selectedForm}
            <article
              class="account"
              style="--acct-accent: {selectedForm.meta.accent_color}"
              data-enabled={selectedForm.meta.enabled}
            >
              <div class="account__body">
                  <div class="account__meta-row">
                    <label class="field field--flex">
                      <span class="field__label">标签</span>
                      <input
                        class="field__input"
                        type="text"
                        placeholder="显示名称"
                        bind:value={selectedForm.meta.label}
                      />
                    </label>
                    <label class="field field--swatch">
                      <span class="field__label">强调色</span>
                      <span class="colorpicker">
                        <input
                          type="color"
                          bind:value={selectedForm.meta.accent_color}
                          aria-label="强调色"
                        />
                        <span class="colorpicker__hex">{selectedForm.meta.accent_color}</span>
                      </span>
                    </label>
                  </div>

                  <label class="field">
                    <span class="field__label">备注（可选）</span>
                    <input
                      class="field__input"
                      type="text"
                      placeholder="例如：主账号 / 备用额度 …"
                      bind:value={selectedForm.meta.note}
                    />
                  </label>

                  {#if selectedForm.hasCredentials && !selectedForm.credsDirty}
                    <div class="account__stored">
                      <span class="account__badge account__badge--ok">已保存</span>
                      <button class="btn btn--ghost" onclick={() => (selectedForm.credsDirty = true)}>修改</button>
                      <button class="btn btn--ghost" onclick={() => clearCredentialsFor(selectedForm)}>清除</button>
                    </div>
                  {:else}
                    {#if isAccessKey(selectedForm.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">Access Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="AK..."
                          bind:value={selectedForm.accessKey}
                          oninput={() => (selectedForm.credsDirty = true)}
                        />
                      </label>
                      <label class="field">
                        <span class="field__label">Secret Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="SK..."
                          bind:value={selectedForm.secretKey}
                          oninput={() => (selectedForm.credsDirty = true)}
                        />
                      </label>
                    {:else if isLocalToken(selectedForm.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">本地登录凭证</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="粘贴 Codex 登录令牌，或点击右侧自动检测"
                          bind:value={selectedForm.token}
                          oninput={() => (selectedForm.credsDirty = true)}
                        />
                      </label>
                      <div class="account__actions">
                        <button class="btn btn--ghost" onclick={() => detectCodexFor(selectedForm)}>
                          自动检测 ~/.codex
                        </button>
                      </div>
                    {:else}
                      <label class="field">
                        <span class="field__label">API Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="sk-..."
                          bind:value={selectedForm.apiKey}
                          oninput={() => (selectedForm.credsDirty = true)}
                        />
                      </label>
                    {/if}

                    <div class="account__actions">
                      <button
                        class="btn btn--ghost"
                        disabled={selectedForm.testing || !hasCredentialInput(selectedForm)}
                        onclick={() => testConnection(selectedForm)}
                      >
                        {selectedForm.testing ? "测试中…" : "测试连接"}
                      </button>
                      <button
                        class="btn btn--primary"
                        disabled={!hasCredentialInput(selectedForm)}
                        onclick={() => saveCredentialsFor(selectedForm)}
                      >
                        {selectedForm.saved ? "已保存 ✓" : "保存凭证"}
                      </button>
                      {#if selectedForm.hasCredentials}
                        <button class="btn btn--ghost" onclick={() => clearCredentialsFor(selectedForm)}>清除</button>
                      {/if}
                    </div>
                  {/if}

                  {#if selectedForm.testResult}
                    <div class="account__test {testResultClass(selectedForm.testResult)}">
                      {formatTestResult(selectedForm.testResult)}
                    </div>
                  {/if}
                  {#if selectedForm.error}
                    <div class="account__error">{selectedForm.error}</div>
                  {/if}

                  <div class="account__danger">
                    <button
                      class="btn btn--danger"
                      class:btn--confirm={deleteConfirmId === selectedForm.meta.instance_id}
                      disabled={busy === selectedForm.meta.instance_id}
                      onclick={() => deleteAccount(selectedForm)}
                    >
                      {busy === selectedForm.meta.instance_id
                        ? "删除中…"
                        : deleteLabel(selectedForm)}
                    </button>
                    {#if deleteConfirmId === selectedForm.meta.instance_id}
                      <span class="account__danger-hint">再次点击将删除该账户及凭证</span>
                    {/if}
                  </div>
                </div>
            </article>
          {/if}

          {#if pickerOpen}
            <div
              class="preset-menu-backdrop"
              onclick={() => {
                pickerOpen = false;
                presetMenuFor = null;
              }}
              aria-hidden="true"
            ></div>
            <div class="preset-picker" role="dialog" aria-modal="true" aria-label="添加账户">
              <header class="preset-picker__head">
                <span class="preset-picker__title">选择来源</span>
                <button
                  class="preset-picker__close"
                  onclick={() => {
                    pickerOpen = false;
                    presetMenuFor = null;
                  }}
                  aria-label="关闭"
                >×</button>
              </header>
              <div class="preset-grid">
                {#each catalog?.presets ?? [] as preset (preset.kind)}
                  <div class="preset-card-wrap">
                    <button
                      class="preset-card"
                      class:preset-card--menu-open={presetMenuFor?.kind === preset.kind}
                      style="--preset-accent: {preset.default_accent}"
                      disabled={busy?.startsWith("add-")}
                      onclick={() => onPresetClick(preset)}
                    >
                      <span class="preset-card__logo">
                        <ProviderLogo kind={preset.kind} size={22} accent={preset.default_accent} />
                      </span>
                      <span class="preset-card__text">
                        <span class="preset-card__name">
                          {preset.display_name}
                          {#if preset.experimental}
                            <span class="preset-card__exp">实验</span>
                          {/if}
                        </span>
                        <span class="preset-card__auth">
                          {preset.auth_kind === "access_key_secret"
                            ? "Access Key + Secret"
                            : preset.auth_kind === "local_token"
                              ? "本地登录凭证"
                              : "Bearer API Key"}
                        </span>
                      </span>
                      <span class="preset-card__add">
                        {busy === `add-${preset.kind}`
                          ? "添加中…"
                          : preset.sub_modes?.length
                            ? "选择计费方式 ▾"
                            : "＋ 添加"}
                      </span>
                    </button>
                    {#if presetMenuFor?.kind === preset.kind && preset.sub_modes?.length}
                      <div class="preset-menu" role="menu" aria-label={`选择 ${preset.display_name} 计费方式`}>
                        {#each preset.sub_modes as m (m.kind)}
                          <button
                            class="preset-menu__item"
                            class:is-limited={m.limited}
                            role="menuitem"
                            title={m.note || m.label}
                            disabled={m.limited || busy?.startsWith("add-")}
                            onclick={() => addSubMode(m)}
                          >
                            <span class="preset-menu__label">{m.label}</span>
                            {#if m.limited}
                              <span class="preset-menu__badge">未实装</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </div>

      {:else if tab === "interaction"}
        <div class="pane">
          <h2 class="pane__title">交互与通知</h2>

          <div class="section">
            <h3 class="section__title">行为</h3>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">靠边吸附</span>
                <span class="behavior-hint">拖动面板贴近屏幕边缘时自动吸附。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={edgeSnap} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>

            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">开机自启</span>
                <span class="behavior-hint">开机后自动运行本程序（写入当前用户注册表，点保存后生效）。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={autostart} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">多端同步</h3>
            <p class="hint">本机作为 hub 接收其它设备上报；或以 agent 把本机用量上报到指定 hub；或用局域网模式自动发现同网段设备并互相同步。改动需重启应用生效。</p>
            <label class="interval">
              <select class="interval-select" bind:value={hubMode}>
                <option value="off">关闭</option>
                <option value="lan">局域网模式（自动发现同网段设备）</option>
                <option value="hub">本机作为 hub（接收上报）</option>
                <option value="agent">作为 agent（上报到远端 hub）</option>
              </select>
            </label>
            {#if hubMode === "hub" || hubMode === "lan"}
              <label class="interval">
                <input type="number" min="1024" max="65535" bind:value={hubPort} />
                <span class="interval__hint">监听端口（默认 43210）</span>
              </label>
            {/if}
            {#if hubMode === "lan"}
              <p class="hint">
                同一局域网内的实例会通过 mDNS 自动互相发现并互相同步，无需填写任何地址。
                每台机器都会在设备页看到全部同网段设备。跨网段（不同路由器 / 办公网与家中）无法通过 mDNS 发现，
                请改用「作为 agent」手动填写 hub 地址，或把其中一台设为 hub。
              </p>
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">上报本机用量</span>
                  <span class="behavior-hint">每 30 秒向每个已发现的同网段设备上报；本机同时也会接收它们的用量。</span>
                </div>
                <label class="toggle">
                  <input type="checkbox" bind:checked={reportOn} />
                  <span class="toggle__track"><span class="toggle__thumb"></span></span>
                </label>
              </div>
              <p class="hint">
                首次启用时若共享密钥为空，会自动生成并保存。其它设备需填写<strong>同一密钥</strong>才能互相同步。
              </p>
            {/if}
            {#if hubMode === "agent"}
              <label class="interval">
                <input type="text" placeholder="http://192.168.1.20:43210" bind:value={hubBase} />
                <span class="interval__hint">hub 地址</span>
              </label>
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">上报本机用量</span>
                  <span class="behavior-hint">每 30 秒向 hub 上报本机工具 token 与 Provider 数量。</span>
                </div>
                <label class="toggle">
                  <input type="checkbox" bind:checked={reportOn} />
                  <span class="toggle__track"><span class="toggle__thumb"></span></span>
                </label>
              </div>
            {/if}
            {#if hubMode !== "off"}
              <label class="interval">
                <input
                  type="password"
                  placeholder="（留空则不鉴权）"
                  bind:value={hubToken}
                />
                <span class="interval__hint">共享密钥 · hub 与 agent 两端须一致，Bearer 鉴权</span>
              </label>
            {/if}
          </div>

          <div class="section">
            <h3 class="section__title">用量告急通知</h3>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">启用通知</span>
                <span class="behavior-hint">用量跨越阈值时弹出系统通知。关闭后仅保留界面警示。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={notifyEnabled} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
            <div class="threshold-grid">
              <label class="field">
                <span class="field__label">警告阈值（%）</span>
                <input
                  class="field__input"
                  type="number"
                  min="0"
                  max="100"
                  bind:value={notifyWarn}
                />
              </label>
              <label class="field">
                <span class="field__label">告急阈值（%）</span>
                <input
                  class="field__input"
                  type="number"
                  min="0"
                  max="100"
                  bind:value={notifyCrit}
                />
              </label>
            </div>
            <p class="hint">设为 0 可关闭对应级别。建议告急阈值高于警告阈值。</p>
          </div>
        </div>

      {:else if tab === "network"}
        <div class="pane">
          <h2 class="pane__title">网络</h2>

          <div class="section">
            <h3 class="section__title">代理</h3>
            <p class="hint">
              为所有 Provider 请求与多端同步配置 HTTP / SOCKS5 代理。保存后生效，已连接的账号会自动重建。
              留空则不显式设置代理（跟随系统环境变量）。
            </p>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">启用代理</span>
                <span class="behavior-hint">支持 http:// 与 socks5://，用户名密码可写在 URL 中。</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={proxyEnabled} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
            {#if proxyEnabled}
              <label class="interval">
                <input
                  type="text"
                  placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080"
                  bind:value={proxyUrl}
                />
                <span class="interval__hint">代理地址</span>
              </label>
              <div class="account__actions">
                <button class="btn btn--ghost" disabled={proxyTesting || proxyUrl.trim().length === 0} onclick={() => handleTestProxy()}>
                  {proxyTesting ? "测试中…" : "测试连接"}
                </button>
              </div>
              {#if proxyResult}
                <div class="account__test {proxyResult.ok ? "test-ok" : "test-fail"}">
                  {proxyResult.ok
                    ? `✓ 连接成功（HTTP ${proxyResult.status ?? "?"}）`
                    : `✗ ${proxyResult.error ?? "连接失败"}`}
                </div>
              {/if}
            {/if}
          </div>

          <div class="section">
            <h3 class="section__title">本机工具数据目录</h3>
            <p class="hint">
              Claude Code / ZCode / MiniMax Code 等工具默认把数据写在用户主目录的
              点目录下（<code>.claude</code>、<code>.zcode</code>、<code>.minimax</code>…）。
              若你把这些目录整体搬到了别处，在这里填一个<strong>额外数据根目录</strong>即可——
              其下的所有点目录会被自动命中，无需逐个工具填写。
              搬迁后旧位置常残留停更的副本，程序会自动选择<strong>最近活跃</strong>的那个。
            </p>

            <div class="roots-list">
              {#each toolDataRoots as root, i (i)}
                <div class="roots-row">
                  <input
                    class="field__input roots-input"
                    type="text"
                    placeholder="D:\Lab\.agentdata"
                    bind:value={toolDataRoots[i]}
                  />
                  <button
                    type="button"
                    class="btn btn--ghost"
                    aria-label="删除该根目录"
                    onclick={() => toolDataRoots = toolDataRoots.filter((_, j) => j !== i)}
                  >×</button>
                </div>
              {/each}
            </div>
            <div class="account__actions">
              <button
                class="btn btn--ghost"
                onclick={() => toolDataRoots = [...toolDataRoots, ""]}
              >＋ 添加根目录</button>
            </div>

            <h3 class="section__title" style="margin-top: 14px;">单工具精确覆盖</h3>
            <p class="hint">仅当上面的根目录规则不适用时使用，直接指定某个工具的数据目录。</p>
            {#each TOOL_DATA_DIR_KEYS as key (key)}
              <label class="field">
                <span class="field__label">{TOOL_DATA_LABELS[key]}</span>
                <input
                  class="field__input"
                  type="text"
                  placeholder="留空 = 使用默认位置"
                  value={toolDataDirs[key] ?? ""}
                  oninput={(e) => toolDataDirs = { ...toolDataDirs, [key]: e.currentTarget.value }}
                />
              </label>
            {/each}
          </div>
        </div>

      {:else if tab === "router"}
        <!-- ═══════ 服务卡 ═══════ -->
        <div class="pane">
          <section class="rt-card" class:rt-card--off={!routerCfg.enabled}>
            <div class="rt-card__hd">
              <h2 class="rt-card__title">TokenRouter</h2>
              {#if routerStatus?.health.bind_error}
                <span class="line-badge line-badge--bad"><i class="rt-dot rt-dot--err"></i>端口异常</span>
              {:else if routerCfg.enabled && routerStatus?.health.listening}
                <span class="line-badge line-badge--ok"><i class="rt-dot rt-dot--live"></i>服务中</span>
              {:else if routerCfg.enabled}
                <span class="line-badge"><i class="rt-dot"></i>已启用</span>
              {:else}
                <span class="line-badge"><i class="rt-dot"></i>已停用</span>
              {/if}
              <span class="spacer"></span>
              <label class="rt-switch" title="总开关：停用后工具的请求会收到明确错误">
                <input type="checkbox" bind:checked={routerCfg.enabled} />
                <span class="rt-switch__track"><span class="rt-switch__thumb"></span></span>
              </label>
            </div>
            <p class="rt-desc">本机 API 代理：工具指向下面的地址，线路配额告急自动切换，长任务不中断</p>
            <div class="rt-addr-row">
              <code class="rt-addr">http://127.0.0.1:{routerCfg.port}</code>
              <button type="button" class="btn btn--ghost" onclick={copyRouterUrl}>
                {routerUrlCopied ? "已复制 ✓" : "复制地址"}
              </button>
              <span class="rt-hint">填到工具的「API 地址」（带不带 /v1 均可）</span>
              <span class="spacer"></span>
              <button type="button" class="rt-link" onclick={() => (svcOpen = !svcOpen)}>
                {svcOpen ? "收起服务设置 ▲" : "服务设置 ▾"}
              </button>
            </div>

            {#if svcOpen}
              <div class="rt-sect rt-sect--in">
                <h3 class="rt-sect__title">服务设置 <span class="rt-sect__sub">多数情况不用改</span></h3>
                <div class="rt-grid">
                  <label class="rt-field">
                    <span class="rt-label">主动预警阈值（最小窗口剩余 %，0 = 关闭）</span>
                    <input class="rt-input" type="number" min="0" max="95" bind:value={routerCfg.proactive_threshold_percent} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">配额错误冷却（秒，尊重上游 Retry-After）</span>
                    <input class="rt-input" type="number" min="5" max="86400" bind:value={routerCfg.error_cooldown_secs} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">连接异常熔断（连续 N 次）</span>
                    <input class="rt-input" type="number" min="1" max="20" bind:value={routerCfg.conn_breaker_count} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">切回探视起始间隔（秒，×2 退避）</span>
                    <input class="rt-input" type="number" min="10" max="3600" bind:value={routerCfg.probe_start_secs} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">切回探视最大次数（超过后等窗口重置）</span>
                    <input class="rt-input" type="number" min="1" max="20" bind:value={routerCfg.probe_max_attempts} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">端口</span>
                    <input class="rt-input" type="number" min="1" max="65535" bind:value={routerCfg.port} />
                  </label>
                </div>

                <!-- 切走线仪表：当前承载流量的线路的最小窗口剩余 vs 阈值 -->
                <div class="rt-gauge">
                  <div class="rt-gauge__track">
                    <div class="rt-gauge__zone rt-gauge__zone--low" style="width:{routerCfg.proactive_threshold_percent}%"></div>
                    <div class="rt-gauge__zone rt-gauge__zone--hi" style="width:{100 - routerCfg.proactive_threshold_percent}%"></div>
                  </div>
                  <div class="rt-gauge__line" style="left:{routerCfg.proactive_threshold_percent}%">
                    <span>切走线 {routerCfg.proactive_threshold_percent}%</span>
                  </div>
                  {#if gaugeRemaining !== null}
                    <div class="rt-gauge__bal" style="left:{gaugeRemaining}%"></div>
                  {/if}
                  <input
                    class="rt-gauge__range"
                    type="range"
                    min="0"
                    max="60"
                    step="1"
                    value={routerCfg.proactive_threshold_percent}
                    oninput={onGaugeInput}
                    aria-label="主动预警阈值（最小窗口剩余 %）"
                  />
                </div>
                <div class="rt-gauge__scale"><span>0%</span><span>25%</span><span>50%</span><span>75%</span><span>100%</span></div>
                <div class="rt-gauge__state" class:rt-gauge__state--warn={gaugeRemaining !== null && gaugeRemaining <= routerCfg.proactive_threshold_percent}>
                  {#if gaugeRemaining === null}
                    当前承载流量的线路<strong>没有可用的配额刻度</strong>（provider 不报窗口，或未设额度上限）——
                    主动预警对它不生效，将完全依赖限流错误与硬墙兜底切换。
                  {:else if gaugeRemaining <= routerCfg.proactive_threshold_percent}
                    当前承载流量的线路最小窗口还剩 <strong>{Math.round(gaugeRemaining)}%</strong>，
                    已低于切走线 <strong>{routerCfg.proactive_threshold_percent}%</strong>，新流量已导向备用线路。
                  {:else}
                    当前承载流量的线路最小窗口还剩 <strong>{Math.round(gaugeRemaining)}%</strong>，
                    距离切走线还有 <strong>{Math.round(gaugeRemaining - routerCfg.proactive_threshold_percent)}%</strong> 余量。
                  {/if}
                </div>

                <!-- 切回探视时间线 -->
                {#if routerStatus?.route?.probe}
                  {@const pr = routerStatus.route.probe}
                  <div class="rt-probe rt-sect--in">
                    <div class="rt-probe__t">
                      切回探视中 · 第 {pr.attempts} / {routerCfg.probe_max_attempts} 次
                      <span class="line-badge line-badge--ok"><i class="rt-dot rt-dot--live"></i>主线路资格满足，等待真实请求确认</span>
                    </div>
                    <div class="rt-probe__steps">
                      {#each Array.from({ length: routerCfg.probe_max_attempts }, (_, i) => i + 1) as n (n)}
                        <span
                          class="rt-stp"
                          class:rt-stp--done={n < pr.attempts}
                          class:rt-stp--now={n === pr.attempts}
                        >{n < pr.attempts ? `✓ ${n}` : n}</span>
                      {/each}
                    </div>
                    <p class="rt-hint">下一次探视：{probeInText(pr.next_probe_at)}（失败按 ×2 退避；达上限后等窗口重置）</p>
                  </div>
                {/if}

                <div class="rt-howto">
                  <strong>四层判定：</strong>① 主动预警（可设，0 = 关闭）以最小（最短周期）在报窗口的剩余 % 为准，
                  月总量剩 5% 但 5h 窗剩 98% 时不会误切；② 硬墙兜底（恒开）任一窗口 / 上限打到 100% → 立即不可用；
                  ③ 被动观测（恒开）限流 / 配额错误立即换线路并冷却，连接异常原地重试、连续熔断才标记；
                  ④ 切回 = 资格判定（无窗口打满 + 最小窗口有余量）通过后用真实请求探测，×2 退避，达上限等 reset_at。
                </div>
              </div>
            {/if}
          </section>

          <!-- ═══════ 路由链卡 ═══════ -->
          {#if !route}
            <section class="rt-card rt-empty">
              <h2 class="rt-card__title">还没有配置路由</h2>
              <p class="rt-desc">路由是一条「同协议线路的有序链」：链头是主线路，其余是备用。配额告急时自动换下一条，主线路恢复后探视切回。</p>
              <button type="button" class="btn btn--primary" onclick={ensureRoute}>＋ 新建路由</button>
            </section>
          {:else}
            {@const st = routerStatus?.route}
            <section class="rt-card" class:rt-card--off={!route.on}>
              <div class="rt-card__hd">
                <input class="rt-card__name" type="text" bind:value={route.name} placeholder="路由名称" />
                <span class="line-badge">{route.protocol === "openai" ? "OpenAI 兼容" : "Anthropic"}</span>
                <span class="spacer"></span>
                <label class="rt-switch" title="路由链开关：停用后该链的 token 请求返回明确错误（配置保留）">
                  <input type="checkbox" bind:checked={route.on} />
                  <span class="rt-switch__track"><span class="rt-switch__thumb"></span></span>
                </label>
                <button type="button" class="rt-link" onclick={() => (chainOpen = !chainOpen)}>
                  当前走 {route.candidates[st?.active_index ?? 0]?.model || "—"} {chainOpen ? "▼" : "▲"}
                </button>
              </div>

              <div class="rt-strip">
                {#each route.candidates as cand, i (i)}
                  <button
                    type="button"
                    class="rt-chip"
                    class:rt-chip--on={st?.active_index === i}
                    onclick={() => (chainOpen = true)}
                  >
                    <i class="rt-dot" style="background:{remainingColor(st?.candidates[i]?.remaining_percent)}"></i>
                    {cand.model || "（未设模型）"}
                  </button>
                  {#if i < route.candidates.length - 1}<span class="rt-arrow">→</span>{/if}
                {/each}
                <span class="spacer"></span>
                <code class="rt-key" title={route.token}>{route.token ? `${route.token.slice(0, 7)}…${route.token.slice(-6)}` : "待生成"}</code>
                <button type="button" class="btn btn--ghost btn--sm" onclick={() => copyRouteToken(route.token)} disabled={!route.token}>
                  {tokenCopied ? "已复制 ✓" : "复制"}
                </button>
              </div>

              <div class="rt-collapse" class:rt-collapse--open={chainOpen}>
                <div>
                  <div class="rt-sect">
                    <div class="rt-grid">
                      <label class="rt-field">
                        <span class="rt-label">路由名称</span>
                        <input class="rt-input" type="text" bind:value={route.name} />
                      </label>
                      <label class="rt-field">
                        <span class="rt-label">协议（线路上游需同协议）</span>
                        <select class="rt-input" bind:value={route.protocol}>
                          <option value="openai">OpenAI 兼容</option>
                          <option value="anthropic">Anthropic</option>
                        </select>
                      </label>
                    </div>
                    <label class="rt-field" style="margin-top:12px">
                      <span class="rt-label">API Key（保存时自动生成；工具端以此作 API 密钥）</span>
                      <span class="rt-url-row">
                        <input class="rt-input rt-mono" type="text" value={route.token || "（保存后自动生成）"} readonly />
                        <button type="button" class="btn btn--ghost" onclick={() => copyRouteToken(route.token)} disabled={!route.token}>复制</button>
                      </span>
                    </label>
                  </div>

                  <div class="rt-sect">
                    <h3 class="rt-sect__title">
                      链路 <span class="rt-sect__sub">按顺序尝试：① 不可用时自动换 ②，主线路恢复后探视切回</span>
                    </h3>
                    <div class="rt-lines" class:rt-lines--single={route.candidates.length < 2}>
                      {#each route.candidates as cand, i (i)}
                        {@const cs = st?.candidates[i]}
                        <div
                          class="rt-line"
                          class:rt-line--on={st?.active_index === i}
                          class:rt-line--low={(cs?.remaining_percent ?? 100) <= routerCfg.proactive_threshold_percent}
                          style="animation-delay:{i * 70}ms"
                        >
                          <span class="rt-line__num">{CIRCLES[i] ?? String(i + 1)}</span>
                          <div class="rt-line__hd">
                            <span class="rt-line__ttl">{i === 0 ? "主线路" : `备用 ${i}`}</span>
                            {#if cs}
                              <span class="line-badge {lineBadge(cs.state)}" title="剩余按最小（最短周期）在报窗口计算">
                                <i class="rt-dot" style="background:{remainingColor(cs.remaining_percent)}"></i>
                                {LINE_STATE_LABELS[cs.state] ?? cs.state}{cs.remaining_percent !== undefined ? ` · 剩 ${Math.round(cs.remaining_percent)}%` : ""}
                              </span>
                            {/if}
                            <span class="spacer"></span>
                            <span class="ops">
                              <button type="button" class="rt-op" disabled={i === 0} onclick={() => moveCandidate(i, -1)} title="上移">↑</button>
                              <button type="button" class="rt-op" disabled={i === route.candidates.length - 1} onclick={() => moveCandidate(i, 1)} title="下移">↓</button>
                              <button type="button" class="rt-op rt-op--del" onclick={() => removeCandidate(i)} title="移除该线路">✕</button>
                            </span>
                          </div>
                          <div class="rt-line__row">
                            <label class="rt-field">
                              <span class="rt-label">账户</span>
                              <select
                                class="rt-input"
                                value={cand.account}
                                onchange={(e) => onCandidateAccountChanged(cand, (e.currentTarget as HTMLSelectElement).value)}
                              >
                                <option value="">选择账户…</option>
                                {#each catalog?.accounts ?? [] as acct (acct.instance_id)}
                                  <option value={acct.instance_id}>{acct.label}</option>
                                {/each}
                              </select>
                            </label>
                            <label class="rt-field">
                              <span class="rt-label">模型（工具端可任填，实际由此决定；留空 = 透传）</span>
                              <span class="rt-model-wrap">
                                <input class="rt-input rt-mono" type="text" bind:value={cand.model} placeholder="留空 = 透传" />
                                <button
                                  type="button"
                                  class="rt-op"
                                  title="从上游拉取模型列表"
                                  disabled={modelFetching[modelKey(cand.account, cand.base_url)] || !cand.account || !cand.base_url}
                                  onclick={() => handleFetchModels(cand, route.protocol)}
                                >{modelFetching[modelKey(cand.account, cand.base_url)] ? "…" : "▼"}</button>
                                {#if modelPickerFor === modelKey(cand.account, cand.base_url) && (modelOptions[modelKey(cand.account, cand.base_url)]?.length ?? 0) > 0}
                                  <span class="rt-models">
                                    <b>从上游拉取 · {modelOptions[modelKey(cand.account, cand.base_url)].length} 个模型</b>
                                    {#each modelOptions[modelKey(cand.account, cand.base_url)] ?? [] as m (m)}
                                      <button type="button" class:rt-models--sel={m === cand.model} onclick={() => pickModel(cand, m)}>{m}</button>
                                    {/each}
                                  </span>
                                {/if}
                              </span>
                            </label>
                          </div>
                          <label class="rt-field">
                            <span class="rt-label">上游地址（选账户自动带出，可改为中转站）</span>
                            <span class="rt-url-row">
                              <input class="rt-input rt-mono" type="text" bind:value={cand.base_url} placeholder="选择账户后自动预填" />
                              {#if cand.base_url === (ROUTER_BASE_PRESETS[accountKindOf(cand.account)] ?? "")}
                                <span class="rt-tag">自动带出</span>
                              {/if}
                            </span>
                          </label>
                          {#if modelFetchError[modelKey(cand.account, cand.base_url)]}
                            <p class="rt-line__err">{modelFetchError[modelKey(cand.account, cand.base_url)]}</p>
                          {/if}
                          {#if cs?.last_error}
                            <p class="rt-line__err" title={cs.last_error}>{cs.last_error}</p>
                          {/if}
                          {#if cs?.state === "cooldown" && cs.cooldown_until}
                            <p class="rt-hint">冷却至 {cooldownUntilText(cs.cooldown_until)}恢复（{cs.cooldown_reason === "quota" ? "配额/限流" : cs.cooldown_reason === "auth" ? "凭据" : "连接异常熔断"}）</p>
                          {/if}
                          <details class="rt-cap">
                            <summary>
                              额度上限（可选，达到即提前切换）·
                              {#if cand.plan_limit_tokens_daily || cand.monthly_cost_limit}
                                日 <b>{cand.plan_limit_tokens_daily || "未设"}</b>{cand.plan_limit_tokens_daily ? " tokens" : ""} ·
                                月 <b>{cand.monthly_cost_limit || "未设"}</b>{cand.monthly_cost_limit ? " 金额" : ""}
                              {:else}
                                未设置
                              {/if}
                            </summary>
                            <div class="rt-cap__body">
                              <label class="rt-field">
                                <span class="rt-label">日上限（tokens / 自然日，路由自记账）</span>
                                <input class="rt-input" type="number" min="0" bind:value={cand.plan_limit_tokens_daily} placeholder="未设" />
                              </label>
                              <label class="rt-field">
                                <span class="rt-label" title="仅对有金额统计的账户生效（余额差分类：DeepSeek / Kimi 等）；订阅类账户无金额数据，设置不生效">月上限（账户币种金额，按量付费用）</span>
                                <input class="rt-input" type="number" min="0" bind:value={cand.monthly_cost_limit} placeholder="如 20 = ¥20/月" />
                              </label>
                            </div>
                          </details>
                        </div>
                      {/each}
                    </div>
                    <div class="rt-foot">
                      <button type="button" class="btn btn--ghost" onclick={addCandidate}>＋ 添加备用线路</button>
                      <span class="spacer"></span>
                      <span class="rt-hint">工具端模型名可任填（推荐 auto）——实际模型由线路决定；线路模型留空 = 透传工具端原名</span>
                    </div>
                  </div>
                </div>
              </div>
            </section>
          {/if}
        </div>

      {:else}
        <div class="pane">
          <h2 class="pane__title">关于与诊断</h2>

          <div class="section">
            <h3 class="section__title">
              TokenUsageMonitor {#if appVersion}<span class="about-ver">v{appVersion}</span>{/if}
            </h3>
            <p class="hint">
              一个轻量的透明桌面 AI 用量监控面板。边栏小窗 + 热力图 + 消耗速率估算，
              让你在开发 AI 应用时随时掌握各家的额度余量。支持 MiniMax Token
              Plan、DeepSeek API 与火山引擎 AgentPlan。
            </p>
            <ul class="about-list">
              <li>常驻后台：仅托盘图标与数据轮询，无弹窗打扰</li>
              <li>支持多账户：同一来源可添加多个独立账户</li>
              <li>凭证由 Windows 凭据管理器（DPAPI）加密保存</li>
              <li>热力图历史数据存储于本地 SQLite</li>
              <li>本地工具用量采集 + 多端同步（hub）</li>
            </ul>
          </div>

          <div class="section">
            <h3 class="section__title">开源致谢</h3>
            <p class="hint">本项目建立在以下优秀开源项目之上，衷心感谢各项目与维护者的贡献：</p>
            <ul class="about-list">
              <li><b>Tauri</b> —— 桌面应用框架（Rust 后端 + Web 前端）</li>
              <li><b>Svelte</b> —— 响应式前端框架</li>
              <li><b>tokio</b> —— 异步运行时</li>
              <li><b>serde</b> —— 序列化框架</li>
              <li><b>chrono</b> —— 时间与日期处理</li>
              <li><b>reqwest / rustls</b> —— HTTP 客户端与 TLS</li>
              <li><b>rusqlite</b> —— SQLite 绑定</li>
              <li><b>keyring</b> —— 基于 Windows DPAPI 的安全凭证存储</li>
            </ul>
            <p class="hint">用量监控与多端同步的思路借鉴自 tokscale。</p>
          </div>

          <div class="section">
            <h3 class="section__title">诊断</h3>
            <p class="hint">
              共 {forms.length} 个账户，其中
              {forms.filter((f) => f.meta.enabled).length || 0} 个已启用。
            </p>
            <div class="account__actions">
              <button class="btn btn--ghost" onclick={() => forceRefresh()}>
                手动刷新全部
              </button>
            </div>
          </div>
        </div>
      {/if}
    </section>
  </div>

  <footer class="settings__footer">
    <button class="btn btn--ghost" onclick={handleClose}>保存并关闭</button>
    <button class="btn btn--primary" disabled={!canSave || saving} onclick={handleSaveAll}>
      {saving ? "保存中…" : savedFlash ? "已保存 ✓" : "保存设置"}
    </button>
  </footer>
</main>

<style>
  .settings {
    height: 100%;
    display: flex;
    flex-direction: column;
    /* 与主界面一致的分层玻璃：强调色径向光晕 + 半透明暗底 + 背景模糊。 */
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(76, 194, 255, 0.07), transparent 42%),
      rgba(24, 26, 30, 0.82);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-lg);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    box-sizing: border-box;
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    overflow: hidden;
  }

  .settings__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--tum-space-3) var(--tum-space-5);
    border-bottom: 1px solid var(--tum-border);
    -webkit-app-region: drag;
  }

  .settings__brand {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .settings__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--tum-amber);
    box-shadow: 0 0 8px var(--tum-amber-glow), 0 0 2px var(--tum-amber);
  }

  .settings__title {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-primary);
  }

  .settings__close {
    all: unset;
    cursor: pointer;
    color: var(--tum-text-muted);
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--tum-radius-xs);
    -webkit-app-region: no-drag;
    transition: color 0.15s ease, background 0.15s ease;
  }

  .settings__close:hover {
    color: var(--tum-danger);
    background: var(--tum-danger-fill);
  }

  .settings__win {
    display: flex;
    align-items: center;
    gap: 2px;
    -webkit-app-region: no-drag;
  }

  .settings__win-btn {
    all: unset;
    cursor: pointer;
    color: var(--tum-text-muted);
    width: 26px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--tum-radius-xs);
    font-size: 12px;
    line-height: 1;
    transition: color 0.15s ease, background 0.15s ease;
  }

  .settings__win-btn:hover {
    color: var(--tum-text-primary);
    background: var(--tum-surface-hover);
  }

  .settings__body {
    flex: 1;
    /* 关键：flex 子元素默认 min-height:auto 会撑开父级；0 才允许内部滚动 */
    min-height: 0;
    display: flex;
  }

  /* --- Left rail nav ---------------------------------------------------- */
  .settings__nav {
    width: 148px;
    flex: 0 0 148px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--tum-space-3) var(--tum-space-2);
    border-right: 1px solid var(--tum-border);
    background: rgba(15, 23, 42, 0.35);
    overflow-y: auto;
    min-height: 0;
  }

  .nav-item {
    all: unset;
    cursor: pointer;
    padding: 9px 12px;
    border-radius: var(--tum-radius-sm);
    font-size: var(--tum-font-size-base);
    font-weight: 500;
    color: var(--tum-text-secondary);
    letter-spacing: 0.3px;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease, color 0.15s ease;
    white-space: nowrap;
  }

  .nav-item:hover {
    color: var(--tum-text-primary);
    background: var(--tum-surface-hover);
  }

  .nav-item.is-active {
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    box-shadow: inset 2px 0 0 var(--tum-accent);
  }

  .settings__content {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    /* 加大横向内边距，让右侧内容与左侧导航明确分离开，避免内容紧贴导航栏 */
    padding: 22px 34px 32px;
    scrollbar-width: thin;
    scrollbar-color: var(--tum-border) transparent;
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-5);
    max-width: 680px;
  }

  .pane__title {
    font-size: var(--tum-font-size-lg);
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--tum-text-primary);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-3);
  }

  .section__title {
    font-size: var(--tum-font-size-sm);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
  }

  .section__count {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-accent);
    background: var(--tum-accent-fill);
    border-radius: 999px;
    padding: 1px 8px;
    font-family: var(--tum-font-mono);
  }

  .hint {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-secondary);
    line-height: 1.5;
  }

  .empty {
    padding: var(--tum-space-4);
    border: 1px dashed var(--tum-border);
    border-radius: var(--tum-radius-md);
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-sm);
    text-align: center;
  }

  .global-error {
    padding: 8px 12px;
    margin-bottom: var(--tum-space-3);
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
    border-radius: var(--tum-radius-xs);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-sm);
    word-break: break-all;
  }

  .about-list {
    margin: 0;
    padding-left: var(--tum-space-4);
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--tum-text-secondary);
    font-size: var(--tum-font-size-sm);
    line-height: 1.5;
  }

  .about-ver {
    font-size: 12px;
    font-weight: 600;
    color: var(--tum-accent, #4cc2ff);
    background: rgba(76, 194, 255, 0.12);
    border-radius: 999px;
    padding: 1px 8px;
    vertical-align: middle;
  }

  /* --- Interval / select ------------------------------------------------ */
  .interval {
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
    flex-wrap: wrap;
  }

  .interval input {
    width: 100%;
    max-width: 160px;
    min-width: 80px;
    box-sizing: border-box;
    padding: 8px 10px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-accent);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-base);
    font-variant-numeric: tabular-nums;
    -webkit-app-region: no-drag;
  }

  .interval input:focus,
  .interval-select:focus {
    outline: none;
    border-color: var(--tum-accent-stroke);
    box-shadow: 0 0 0 2px var(--tum-accent-fill);
  }

  .interval-select {
    width: 100%;
    max-width: 260px;
    min-width: 160px;
    padding: 8px 10px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    font-size: var(--tum-font-size-base);
    -webkit-app-region: no-drag;
    cursor: pointer;
  }

  .interval__hint {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }

  /* --- Behavior row ------------------------------------------------------ */
  .behavior-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--tum-space-3);
    -webkit-app-region: no-drag;
    padding: var(--tum-space-2) 0;
  }

  .behavior-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .behavior-label {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
  }

  .behavior-hint {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-muted);
    line-height: 1.4;
  }

  .threshold-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--tum-space-3);
  }

  .threshold-grid .field__input {
    width: 100%;
    max-width: 120px;
  }

  .rate-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--tum-space-3);
  }

  .rate-grid .field__input {
    width: 100%;
  }

  /* --- Preset cards ------------------------------------------------------ */
  .preset-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--tum-space-3);
  }

  .preset-card {
    all: unset;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
    padding: var(--tum-space-3) var(--tum-space-4);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    background: var(--tum-surface);
    -webkit-app-region: no-drag;
    transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.1s ease;
  }

  .preset-card:hover:not(:disabled) {
    border-color: var(--preset-accent);
    box-shadow: 0 0 12px color-mix(in srgb, var(--preset-accent) 35%, transparent);
  }

  .preset-card:active:not(:disabled) {
    transform: translateY(1px);
  }

  .preset-card:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* 单卡片容器：撑满网格格，作为其下方弹出菜单的相对定位锚点 */
  .preset-card-wrap {
    position: relative;
  }

  .preset-card--menu-open {
    border-color: var(--preset-accent);
    box-shadow: 0 0 12px color-mix(in srgb, var(--preset-accent) 35%, transparent);
  }

  /* xiaomi 下拉二选一菜单 */
  .preset-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    padding: 4px;
    gap: 2px;
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-md);
    background: rgba(30, 32, 36, 0.98);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
  }

  .preset-menu__item {
    all: unset;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 10px;
    border-radius: var(--tum-radius-sm);
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
    font-family: var(--tum-font);
    -webkit-app-region: no-drag;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .preset-menu__item:hover:not(:disabled) {
    background: var(--tum-accent-fill);
    color: var(--tum-accent);
  }

  .preset-menu__item:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* 骨架 / 未实装模式：整体更暗淡 */
  .preset-menu__item.is-limited {
    color: var(--tum-text-muted);
  }

  .preset-menu__label {
    flex: 1 1 auto;
    min-width: 0;
  }

  /* “未实装”警示徽标 */
  .preset-menu__badge {
    flex: none;
    margin-left: 8px;
    padding: 0 6px;
    font-size: var(--tum-font-size-xs);
    line-height: 16px;
    color: var(--tum-warning);
    border: 1px solid color-mix(in srgb, var(--tum-warning) 45%, transparent);
    border-radius: var(--tum-radius-pill);
    letter-spacing: 0.3px;
  }

  /* 点击菜单外任意处关闭（透明遮罩） */
  .preset-menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
  }

  /* —— 账户紧凑列表（主视图）—— */
  .acct-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .acct-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-sm);
    background: var(--tum-surface);
    cursor: pointer;
    user-select: none;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .acct-row:hover {
    background: var(--tum-surface-hover);
  }
  .acct-row:focus-visible {
    outline: 1px solid var(--tum-accent-stroke);
    outline-offset: -1px;
  }
  .acct-row--active {
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }
  .acct-row__logo {
    flex: none;
    display: inline-flex;
    align-items: center;
  }
  .acct-row__label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-primary);
  }
  .acct-row--active .acct-row__label {
    font-weight: 600;
  }
  .acct-row__kind {
    flex: none;
    font-size: 10px;
    font-family: var(--tum-font-mono);
    color: var(--tum-text-muted);
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 凭证已存指示点（绿） */
  .acct-row__dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--tum-ok);
  }
  .acct-row__live {
    flex: none;
  }
  .acct-add {
    margin-top: 8px;
    width: 100%;
  }

  /* —— 添加账户弹层选择器（预设网格按需弹出）—— */
  .preset-picker {
    position: fixed;
    z-index: 30;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(600px, calc(100vw - 48px));
    max-height: min(560px, calc(100vh - 96px));
    display: flex;
    flex-direction: column;
    background: var(--tum-bg);
    border: 1px solid var(--tum-border-strong);
    border-radius: var(--tum-radius-md);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
    overflow: hidden;
  }
  .preset-picker__head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--tum-border);
  }
  .preset-picker__title {
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    color: var(--tum-text-primary);
  }
  .preset-picker__close {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--tum-radius-xs);
    background: transparent;
    color: var(--tum-text-muted);
    font-size: var(--tum-font-size-base);
    line-height: 1;
    cursor: pointer;
  }
  .preset-picker__close:hover {
    color: var(--tum-text-primary);
    background: var(--tum-surface-hover);
  }
  .preset-picker .preset-grid {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 12px 14px;
  }

  .preset-card__logo {
    width: 26px;
    height: 26px;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .preset-card__exp {
    margin-left: 6px;
    font-size: var(--tum-font-size-xs);
    font-weight: 500;
    color: var(--tum-warn);
    background: rgba(255, 200, 61, 0.12);
    border: 1px solid rgba(255, 200, 61, 0.4);
    padding: 0 5px;
    border-radius: var(--tum-radius-xs);
    letter-spacing: 0.5px;
    vertical-align: middle;
  }

  .preset-card__text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .preset-card__name {
    font-size: var(--tum-font-size-sm);
    font-weight: 600;
    color: var(--tum-text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .preset-card__auth {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-family: var(--tum-font-mono);
  }

  .preset-card__add {
    margin-left: auto;
    font-size: var(--tum-font-size-xs);
    color: var(--preset-accent);
    font-family: var(--tum-font-mono);
    white-space: nowrap;
  }

  /* --- Account cards ------------------------------------------------------ */
  .account {
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    background: var(--tum-surface);
    overflow: hidden;
    transition: border-color 0.15s ease;
  }

  .account[data-enabled="true"] {
    border-color: var(--acct-accent);
    border-left: 2px solid var(--acct-accent);
  }

  .account__header {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
    padding: var(--tum-space-3) var(--tum-space-4);
    background: rgba(15, 23, 42, 0.4);
  }

  .account__identity {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
    min-width: 0;
    flex: 1;
  }

  .account__swatch {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex: 0 0 auto;
    box-shadow: 0 0 6px currentColor;
  }

  .account__label {
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--tum-radius-xs);
    color: var(--tum-text-primary);
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    letter-spacing: 0.3px;
    padding: 2px 6px;
    min-width: 0;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease, border-color 0.15s ease;
  }

  .account__label:hover,
  .account__label:focus {
    background: rgba(15, 23, 42, 0.6);
    border-color: var(--tum-border);
    outline: none;
  }

  .account__badges {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .account__kind {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-family: var(--tum-font-mono);
  }

  .account__badge {
    font-size: var(--tum-font-size-xs);
    padding: 2px 8px;
    border-radius: var(--tum-radius-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .account__badge--ok {
    background: var(--tum-success-fill);
    color: var(--tum-success);
  }

  .account__badge--live {
    background: var(--tum-accent-fill);
    color: var(--tum-accent);
  }

  .account__badge--off {
    background: var(--tum-surface-hover);
    color: var(--tum-text-muted);
  }

  .account__body {
    padding: var(--tum-space-3) var(--tum-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-3);
  }

  .account__meta-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--tum-space-3);
    align-items: end;
  }

  .account__stored {
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
  }

  .account__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--tum-space-2);
  }

  .account__test {
    padding: 6px 10px;
    font-size: var(--tum-font-size-sm);
    border-radius: var(--tum-radius-sm);
    border-left: 3px solid var(--tum-border);
    word-break: break-all;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }

  .test-ok {
    color: var(--tum-success);
    border-left-color: var(--tum-success);
    background: var(--tum-success-fill);
  }

  .test-fail {
    color: var(--tum-danger);
    border-left-color: var(--tum-danger);
    background: var(--tum-danger-fill);
  }

  .account__error {
    padding: 6px 10px;
    font-size: var(--tum-font-size-sm);
    color: var(--tum-danger);
    background: var(--tum-danger-fill);
    border-radius: var(--tum-radius-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
    word-break: break-all;
  }

  .account__danger {
    display: flex;
    align-items: center;
    gap: var(--tum-space-2);
    border-top: 1px solid var(--tum-border);
    padding-top: var(--tum-space-2);
  }

  .account__danger-hint {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
  }

  /* --- Fields ------------------------------------------------------------- */
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .field--flex {
    flex: 1;
  }

  .field--swatch {
    flex: 0 0 auto;
  }

  .field__label {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.8px;
    font-family: var(--tum-font-mono);
  }

  .field__input {
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
    padding: 10px 12px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-text-primary);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-base);
    line-height: 1.4;
    word-break: break-all;
    -webkit-app-region: no-drag;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .field__input:focus {
    outline: none;
    border-color: var(--tum-accent-stroke);
    box-shadow: 0 0 0 2px var(--tum-accent-fill);
  }

  .field__input::placeholder {
    color: var(--tum-text-muted);
  }

  .colorpicker {
    display: inline-flex;
    align-items: center;
    gap: var(--tum-space-2);
    padding: 4px 4px 4px 6px;
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    background: rgba(15, 23, 42, 0.6);
    -webkit-app-region: no-drag;
  }

  .colorpicker input[type="color"] {
    all: unset;
    width: 26px;
    height: 20px;
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
    -webkit-app-region: no-drag;
  }

  .colorpicker input[type="color"]::-webkit-color-swatch-wrapper {
    padding: 0;
  }

  .colorpicker input[type="color"]::-webkit-color-swatch {
    border: none;
    border-radius: 4px;
  }

  .colorpicker__hex {
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-secondary);
  }

  /* --- Buttons ------------------------------------------------------------- */
  .btn {
    padding: 7px 12px;
    border: 1px solid var(--tum-border);
    background: var(--tum-surface);
    color: var(--tum-text-secondary);
    border-radius: var(--tum-radius-xs);
    cursor: pointer;
    font-size: var(--tum-font-size-sm);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
    transition: all 0.15s ease;
    -webkit-app-region: no-drag;
  }

  .btn:hover:not(:disabled) {
    color: var(--tum-accent);
    border-color: var(--tum-accent-stroke);
    background: var(--tum-accent-fill);
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn--primary {
    background: var(--tum-accent);
    color: #0a0e1a;
    border-color: transparent;
    font-weight: 600;
    letter-spacing: 0.4px;
  }

  .btn--primary:hover:not(:disabled) {
    color: #0a0e1a;
    background: var(--tum-accent-hover);
    box-shadow: 0 0 12px var(--tum-accent-glow);
  }

  .btn--ghost {
    background: transparent;
  }

  .btn--danger {
    color: var(--tum-danger);
    border-color: var(--tum-border);
    background: transparent;
  }

  .btn--danger:hover:not(:disabled) {
    color: var(--tum-danger);
    border-color: var(--tum-danger);
    background: var(--tum-danger-fill);
  }

  .btn--confirm {
    border-color: var(--tum-danger);
    background: var(--tum-danger-fill);
    color: var(--tum-danger);
    font-weight: 600;
  }

  /* --- Toggle switch ------------------------------------------------------- */
  .toggle {
    position: relative;
    display: inline-flex;
    cursor: pointer;
    flex: 0 0 auto;
    -webkit-app-region: no-drag;
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle__track {
    width: 36px;
    height: 20px;
    background: var(--tum-surface-hover);
    border-radius: 10px;
    transition: background 0.2s ease;
    display: inline-flex;
    align-items: center;
    padding: 2px;
  }

  .toggle__thumb {
    width: 16px;
    height: 16px;
    background: var(--tum-text-secondary);
    border-radius: 50%;
    transition: transform 0.2s ease, background 0.2s ease;
  }

  .toggle input:checked + .toggle__track {
    background: var(--tum-accent-fill-strong);
  }

  .toggle input:checked + .toggle__track .toggle__thumb {
    transform: translateX(16px);
    background: var(--tum-accent);
    box-shadow: 0 0 6px var(--tum-accent-glow);
  }

  .settings__footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--tum-space-3);
    padding: var(--tum-space-4) var(--tum-space-5);
    border-top: 1px solid var(--tum-border);
  }
  .roots-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .roots-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .roots-input {
    flex: 1;
    min-width: 0;
  }

  /* ---- TokenRouter 路由 pane（v2：单路由 + 四层判定）---- */
  .spacer { flex: 1; }

  .rt-card {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    border-radius: 14px;
    margin-bottom: 14px;
    overflow: hidden;
    animation: rtCardIn 0.32s cubic-bezier(0.21, 0.8, 0.35, 1) both;
  }
  @keyframes rtCardIn {
    from { opacity: 0; transform: translateY(10px) scale(0.99); }
    to { opacity: 1; transform: none; }
  }
  .rt-card--off { opacity: 0.5; }
  .rt-card__hd {
    display: flex; align-items: center; gap: 11px;
    padding: 15px 17px;
  }
  .rt-card__title { margin: 0; font-size: 15.5px; font-weight: 650; }
  .rt-card__name {
    background: transparent; border: 1px solid transparent; color: var(--tum-text-primary, #e6edf3);
    font-size: 15.5px; font-weight: 650; padding: 4px 8px; border-radius: 8px;
    min-width: 0; flex: none; max-width: 260px; transition: 0.15s;
  }
  .rt-card__name:hover { border-color: var(--tum-border, rgba(255, 255, 255, 0.1)); }
  .rt-card__name:focus { border-color: var(--tum-accent, #4cc2ff); background: rgba(0, 0, 0, 0.2); }
  .rt-card .btn { white-space: nowrap; flex: none; }
  .rt-desc { padding: 0 17px 12px; margin: 0; font-size: 12.5px; color: var(--tum-text-muted, #8b949e); line-height: 1.6; }
  .rt-hint { font-size: 12px; color: var(--tum-text-muted, #8b949e); }
  .rt-empty { padding: 18px; }
  .rt-empty .rt-card__title { margin-bottom: 8px; }
  .rt-empty .rt-desc { padding: 0 0 14px; }

  .rt-addr-row {
    display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 0 17px 14px;
  }
  .rt-addr {
    font-family: var(--tum-font-mono, monospace); font-size: 13px; font-weight: 600;
    background: rgba(0, 0, 0, 0.28); border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    border-radius: 8px; padding: 7px 11px; color: var(--tum-accent, #4cc2ff);
  }
  .rt-link {
    background: none; border: none; color: var(--tum-accent, #4cc2ff);
    font-size: 12.5px; font-weight: 600; padding: 4px; cursor: pointer; white-space: nowrap;
  }
  .rt-link:hover { text-decoration: underline; }

  /* 开关（弹性滑块） */
  .rt-switch { position: relative; width: 42px; height: 23px; flex: none; cursor: pointer; }
  .rt-switch input { display: none; }
  .rt-switch__track {
    position: absolute; inset: 0; border-radius: 999px;
    background: rgba(255, 255, 255, 0.12); transition: background 0.18s;
  }
  .rt-switch__thumb {
    position: absolute; top: 2px; left: 2px; width: 17px; height: 17px;
    border-radius: 50%; background: #8b949e;
    transition: left 0.18s cubic-bezier(0.34, 1.4, 0.5, 1), background 0.18s;
  }
  .rt-switch input:checked + .rt-switch__track { background: var(--tum-accent, #4cc2ff); }
  .rt-switch input:checked + .rt-switch__track .rt-switch__thumb { left: 21px; background: #fff; }

  /* 分节（折叠区） */
  .rt-sect {
    border-top: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    padding: 15px 17px;
    background: rgba(255, 255, 255, 0.015);
    animation: rtCardIn 0.28s ease both;
  }
  .rt-sect__title {
    margin: 0 0 3px; font-size: 13px; font-weight: 650;
    display: flex; align-items: baseline; gap: 8px;
  }
  .rt-sect__sub { font-size: 11.5px; color: var(--tum-text-muted, #8b949e); font-weight: 400; }

  .rt-grid {
    display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 12px 14px; margin-top: 12px;
  }
  .rt-field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .rt-label { font-size: 11.5px; font-weight: 600; color: var(--tum-text-muted, #8b949e); }
  .rt-input {
    width: 100%; background: rgba(0, 0, 0, 0.26);
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
    color: var(--tum-text-primary, #e6edf3); border-radius: 9px;
    padding: 8px 10px; font-size: 13px; outline: none; transition: 0.14s;
  }
  .rt-input:focus {
    border-color: var(--tum-accent, #4cc2ff);
    box-shadow: 0 0 0 3px rgba(76, 194, 255, 0.14);
  }
  select.rt-input { cursor: pointer; }
  .rt-mono { font-family: var(--tum-font-mono, monospace); font-size: 12.5px; }
  .rt-url-row { display: flex; align-items: center; gap: 8px; }
  .rt-tag {
    flex: none; font-size: 10.5px; font-weight: 650; color: var(--tum-text-muted, #8b949e);
    background: rgba(0, 0, 0, 0.25); border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    padding: 2px 7px; border-radius: 6px; white-space: nowrap;
  }

  /* 切走线仪表 */
  .rt-gauge { position: relative; height: 44px; margin: 18px 2px 0; }
  .rt-gauge__track {
    position: absolute; top: 14px; left: 0; right: 0; height: 10px; border-radius: 999px;
    overflow: hidden; display: flex; border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
  }
  .rt-gauge__zone { height: 100%; transition: width 0.3s cubic-bezier(0.4, 0, 0.2, 1); }
  .rt-gauge__zone--low { background: #ef4444; opacity: 0.55; }
  .rt-gauge__zone--hi { background: #22c55e; opacity: 0.42; }
  .rt-gauge__line {
    position: absolute; top: 8px; width: 2px; height: 22px; background: #ef4444;
    transform: translateX(-1px); transition: left 0.3s cubic-bezier(0.4, 0, 0.2, 1); pointer-events: none;
  }
  .rt-gauge__line span {
    position: absolute; top: -19px; left: 50%; transform: translateX(-50%);
    white-space: nowrap; font-size: 10.5px; font-weight: 700; color: #ef4444;
    background: var(--tum-surface, #161b22); padding: 1px 6px; border-radius: 5px;
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
  }
  .rt-gauge__bal {
    position: absolute; top: 10px; width: 3px; height: 18px; background: var(--tum-accent, #4cc2ff);
    transform: translateX(-1.5px); box-shadow: 0 0 0 3px rgba(76, 194, 255, 0.22);
    transition: left 0.6s cubic-bezier(0.34, 1.2, 0.4, 1); pointer-events: none;
  }
  .rt-gauge__range {
    position: absolute; top: 5px; left: 0; right: 0; width: 100%; height: 28px;
    margin: 0; opacity: 0; cursor: grab;
  }
  .rt-gauge__range:active { cursor: grabbing; }
  .rt-gauge__scale {
    display: flex; justify-content: space-between; font-size: 10.5px;
    color: var(--tum-text-muted, #64748b); margin-top: 3px;
  }
  .rt-gauge__state {
    margin-top: 12px; padding: 11px 13px; border-radius: 9px;
    background: rgba(0, 0, 0, 0.22); border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    font-size: 12.5px; color: var(--tum-text-muted, #8b949e); line-height: 1.6; transition: 0.3s;
  }
  .rt-gauge__state strong { color: var(--tum-text-primary, #e6edf3); font-weight: 650; }
  .rt-gauge__state--warn {
    background: rgba(239, 68, 68, 0.1);
    border-color: rgba(239, 68, 68, 0.32);
    color: var(--tum-text-primary, #e6edf3);
  }

  /* 切回探视时间线 */
  .rt-probe {
    margin-top: 12px; padding: 12px 13px; border-radius: 9px;
    border: 1px solid var(--tum-accent, #4cc2ff);
    background: rgba(76, 194, 255, 0.08);
    animation: rtCardIn 0.3s ease both;
  }
  .rt-probe__t {
    font-size: 12.5px; font-weight: 640; color: var(--tum-text-primary, #e6edf3);
    margin-bottom: 8px; display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  }
  .rt-probe__steps { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .rt-stp {
    font-size: 10.5px; font-weight: 650; padding: 3px 8px; border-radius: 6px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
    color: var(--tum-text-muted, #64748b);
  }
  .rt-stp--done { color: #34d399; border-color: rgba(52, 211, 153, 0.4); }
  .rt-stp--now {
    color: var(--tum-accent, #4cc2ff); border-color: var(--tum-accent, #4cc2ff);
    animation: rtStepPop 0.3s cubic-bezier(0.34, 1.5, 0.5, 1) both;
  }
  @keyframes rtStepPop { from { transform: scale(0.7); } to { transform: scale(1); } }

  .rt-howto {
    margin-top: 12px; padding: 10px 12px; border-radius: 9px;
    border: 1px dashed var(--tum-border, rgba(255, 255, 255, 0.14));
    font-size: 12px; color: var(--tum-text-muted, #8b949e); line-height: 1.7;
  }
  .rt-howto strong { color: var(--tum-text-primary, #e6edf3); }

  /* 链路摘要条 */
  .rt-strip {
    display: flex; align-items: center; gap: 8px; padding: 0 17px 14px; flex-wrap: wrap;
  }
  .rt-chip {
    display: inline-flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600;
    padding: 4px 10px; border-radius: 999px; cursor: pointer;
    background: rgba(0, 0, 0, 0.25); border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
    color: var(--tum-text-muted, #8b949e); transition: 0.15s;
    animation: rtChipIn 0.3s cubic-bezier(0.34, 1.35, 0.55, 1) both;
    font-family: var(--tum-font-mono, monospace);
  }
  @keyframes rtChipIn { from { opacity: 0; transform: translateX(-8px) scale(0.92); } to { opacity: 1; transform: none; } }
  .rt-chip:hover { border-color: var(--tum-accent, #4cc2ff); }
  .rt-chip--on {
    border-color: var(--tum-accent, #4cc2ff); color: var(--tum-accent, #4cc2ff);
    background: rgba(76, 194, 255, 0.1);
    box-shadow: 0 0 12px rgba(76, 194, 255, 0.2);
  }
  .rt-arrow { color: var(--tum-text-muted, #64748b); font-size: 12px; }
  .rt-key {
    font-family: var(--tum-font-mono, monospace); font-size: 11.5px;
    padding: 4px 9px; border-radius: 8px; color: var(--tum-text-muted, #8b949e);
    background: rgba(0, 0, 0, 0.25); border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
  }
  .btn--sm { padding: 3px 9px; font-size: 11.5px; white-space: nowrap; flex: none; }

  /* 折叠（grid-template-rows 高度动画） */
  .rt-collapse { display: grid; grid-template-rows: 0fr; transition: grid-template-rows 0.32s cubic-bezier(0.4, 0, 0.2, 1); }
  .rt-collapse--open { grid-template-rows: 1fr; }
  .rt-collapse > div { overflow: hidden; min-height: 0; }

  /* 线路卡（step 圆点 + 入场动画 + 激活呼吸光晕） */
  .rt-lines { display: flex; flex-direction: column; position: relative; }
  /* 连接线是**整条链共用的一根导轨**：从第一个圆点中心连到最后一个圆点中心，
   * 端点 = 卡片上/下内边距 + 圆点半径，与各卡片高度无关。逐卡各画一段的做法
   * 会让每段的终点落在自己高度的中线，卡片高度不一时线就断了。 */
  .rt-lines::before {
    content: "";
    position: absolute;
    left: 24px;
    top: 29px;
    bottom: 29px;
    width: 2px;
    background: var(--tum-border, rgba(255, 255, 255, 0.14));
    opacity: 0.85;
    pointer-events: none;
  }
  .rt-lines--single::before { display: none; }
  .rt-line {
    position: relative; margin: 0 17px 12px; padding: 13px 14px 14px 52px;
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
    border-radius: 10px; background: rgba(0, 0, 0, 0.18);
    transition: border-color 0.25s, box-shadow 0.25s;
    animation: rtLineIn 0.34s cubic-bezier(0.21, 0.8, 0.35, 1) both;
  }
  @keyframes rtLineIn { from { opacity: 0; transform: translateX(-14px); } to { opacity: 1; transform: none; } }
  .rt-line--on {
    border-color: var(--tum-accent, #4cc2ff);
    animation: rtLineIn 0.34s cubic-bezier(0.21, 0.8, 0.35, 1) both, rtGlow 2.6s ease-in-out infinite;
  }
  @keyframes rtGlow {
    0%, 100% { box-shadow: 0 0 0 3px rgba(76, 194, 255, 0.13); }
    50% { box-shadow: 0 0 0 3px rgba(76, 194, 255, 0.26); }
  }
  .rt-line--low { border-color: rgba(245, 158, 11, 0.35); }
  .rt-line__num {
    position: absolute; left: 9px; top: 13px; width: 32px; height: 32px; border-radius: 50%;
    display: grid; place-items: center; font-size: 13px; font-weight: 700; z-index: 2;
    background: rgba(0, 0, 0, 0.3);
    border: 2px solid var(--tum-border, rgba(255, 255, 255, 0.16));
    color: var(--tum-text-muted, #8b949e); transition: 0.25s;
  }
  .rt-line--on .rt-line__num {
    background: var(--tum-accent, #4cc2ff); border-color: var(--tum-accent, #4cc2ff); color: #fff;
  }
  .rt-line__hd { display: flex; align-items: center; gap: 9px; margin-bottom: 12px; flex-wrap: wrap; }
  .rt-line__hd .line-badge { flex: none; }
  .rt-line__ttl { font-size: 13.5px; font-weight: 650; }
  .rt-line--on .rt-line__ttl { color: var(--tum-accent, #4cc2ff); }
  .rt-line__row {
    display: grid; grid-template-columns: 1fr 1.25fr; gap: 12px; margin-bottom: 12px;
  }
  @media (max-width: 720px) { .rt-line__row { grid-template-columns: 1fr; } }
  .rt-line__err { margin: 8px 0 0; font-size: 11.5px; color: #f87171; }
  .ops { display: flex; gap: 4px; }
  .rt-op {
    width: 27px; height: 27px; display: grid; place-items: center;
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.09));
    background: rgba(255, 255, 255, 0.04); border-radius: 7px;
    color: var(--tum-text-muted, #8b949e); font-size: 11px; cursor: pointer; transition: 0.14s;
    flex: none;
  }
  .rt-op:hover:not(:disabled) { color: var(--tum-text-primary, #e6edf3); transform: translateY(-1px); }
  .rt-op--del:hover:not(:disabled) { color: #f87171; border-color: #f87171; }
  .rt-op:disabled { opacity: 0.3; cursor: default; }

  /* 模型下拉 */
  .rt-model-wrap { position: relative; display: flex; gap: 6px; }
  .rt-models {
    position: absolute; top: calc(100% + 4px); right: 0; z-index: 30;
    width: 280px; max-height: 230px; overflow: auto;
    background: var(--tum-surface, #161b22);
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.16));
    border-radius: 10px; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5); padding: 5px;
    animation: rtCardIn 0.16s cubic-bezier(0.21, 0.8, 0.35, 1) both;
  }
  .rt-models b {
    display: block; font-size: 11px; color: var(--tum-text-muted, #8b949e);
    padding: 4px 8px 6px; border-bottom: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    margin-bottom: 4px; font-weight: 500;
  }
  .rt-models button {
    display: block; width: 100%; text-align: left; background: none; border: 0;
    padding: 6px 9px; border-radius: 6px; cursor: pointer; font-size: 12.5px;
    font-family: var(--tum-font-mono, monospace); color: var(--tum-text-muted, #8b949e);
  }
  .rt-models button:hover { background: rgba(76, 194, 255, 0.12); color: var(--tum-accent, #4cc2ff); }
  .rt-models--sel { color: var(--tum-accent, #4cc2ff) !important; }

  /* 额度上限折叠 */
  .rt-cap { border-top: 1px dashed var(--tum-border, rgba(255, 255, 255, 0.09)); margin-top: 10px; padding-top: 10px; }
  .rt-cap summary {
    cursor: pointer; font-size: 12px; color: var(--tum-text-muted, #8b949e);
    list-style: none; display: flex; align-items: center; gap: 7px;
  }
  .rt-cap summary::-webkit-details-marker { display: none; }
  .rt-cap summary::before { content: "▸"; color: var(--tum-text-muted, #64748b); transition: transform 0.16s; }
  .rt-cap[open] summary::before { transform: rotate(90deg); }
  .rt-cap summary:hover { color: var(--tum-text-primary, #e6edf3); }
  .rt-cap summary b { color: var(--tum-text-primary, #e6edf3); font-weight: 600; }
  .rt-cap__body { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 11px; }
  @media (max-width: 720px) { .rt-cap__body { grid-template-columns: 1fr; } }

  .rt-foot {
    display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
    margin-top: 4px; padding-top: 12px;
  }

  /* 状态徽标 / 圆点 */
  .line-badge {
    display: inline-flex; align-items: center; gap: 5px; font-size: 11.5px; font-weight: 600;
    padding: 2px 9px; border-radius: 999px; white-space: nowrap;
    background: rgba(255, 255, 255, 0.05); color: var(--tum-text-muted, #8b949e);
    border: 1px solid var(--tum-border, rgba(255, 255, 255, 0.08));
    transition: background 0.3s, color 0.3s;
  }
  .line-badge--ok { background: rgba(34, 197, 94, 0.14); color: #34d399; border-color: transparent; }
  .line-badge--warn { background: rgba(245, 158, 11, 0.14); color: #fbbf24; border-color: transparent; }
  .line-badge--bad { background: rgba(239, 68, 68, 0.16); color: #f87171; border-color: transparent; }
  .rt-dot {
    width: 6px; height: 6px; border-radius: 50%; flex: none;
    background: currentColor; color: var(--tum-text-muted, #8b949e);
  }
  .rt-dot--live { animation: rtPulse 1.7s ease-in-out infinite; }
  .rt-dot--err { background: #f87171; }
  @keyframes rtPulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  @media (prefers-reduced-motion: reduce) {
    .rt-card, .rt-line, .rt-chip, .rt-models, .rt-probe, .rt-sect {
      animation: none;
    }
    .rt-line--on { animation: none; box-shadow: 0 0 0 3px rgba(76, 194, 255, 0.16); }
    .rt-collapse { transition: none; }
  }

</style>