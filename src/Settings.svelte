<script lang="ts">
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    getProviders,
    getSettings,
    saveSettings,
    saveCredentials,
    getCredentials,
    testRoutingChannel,
    deleteCredentials,
    testProvider,
    testProxy,
    detectCodexToken,
    upsertAccount,
    removeAccount,
    closeSettings,
    forceRefresh,
    getDeviceReport,
    getAppMeta,
    openUrl,
    checkAppUpdate,
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
    type RoutingProbeResult,
    type AccountMeta,
    type Settings,
    type Credentials,
    type TestResult,
    type ProxyTestResult,
    type RatesSnapshot,
    type UpdateInfo,
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
  import { applyLocaleSetting, t, type LangSetting } from "./lib/i18n/store";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";

  type Tab = "general" | "accounts" | "interaction" | "network" | "router" | "about";

  /** 可做精确覆盖的工具标识（须与后端 local::roots 的 key 一致）。 */
  const TOOL_DATA_DIR_KEYS = [".claude", ".codex", ".zcode", ".minimax", "cherry-studio"] as const;
  const TOOL_DATA_LABELS: Record<string, string> = {
    ".claude": "settings.tools.claude",
    ".codex": "settings.tools.codex",
    ".zcode": "settings.tools.zcode",
    ".minimax": "settings.tools.minimax",
    "cherry-studio": "settings.tools.cherryStudio",
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
    /** AccessKeySecret 账户的可选推理 API Key（仅 TokenRouter 路由用；
     *  监控用量仍走 AK/SK 签名，留空即「仅监控、不可路由」）。 */
    routeApiKey: string;
    /** 路由通道探测结果（测试连接通过后自动追加一次）。 */
    routingProbe: RoutingProbeResult | null;
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
  // 界面语言：切换立即生效（applyLocaleSetting），并随保存持久化到后端。
  let language = $state<LangSetting>("auto");
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
  // 关于页：仓库地址（后端单处常量下发）+ 更新检查状态。
  let repoUrl = $state("");
  let checkUpdatesOnStart = $state(false);
  let updateInfo = $state<UpdateInfo | null>(null);
  let updateChecking = $state(false);
  let updateError = $state<string | null>(null);
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
      routeApiKey: "",
      routingProbe: null,
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
      language = ["auto", "zh-CN", "en"].includes(s.language)
        ? (s.language as LangSetting)
        : "auto";
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
      // 仓库地址与更新检查设置（关于页）。
      getAppMeta()
        .then((m) => (repoUrl = m.repo_url))
        .catch(() => {});
      checkUpdatesOnStart = s.check_updates_on_start ?? false;
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
      const routeKey = f.routeApiKey.trim();
      return {
        kind: "access_key_secret",
        access_key: f.accessKey.trim(),
        secret_key: f.secretKey.trim(),
        ...(routeKey ? { api_key: routeKey } : {}),
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
      f.error = get(t)("settings.accounts.errorFillCreds");
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
      // 路由通道探测（免费 /models）：监控与推理分离的 Provider（火山等）
      // 上，监控通过不代表推理接口可达——两条通道分别显示。
      const creds = buildCredentials(f);
      const baseUrl = ROUTER_BASE_PRESETS[f.meta.provider_kind] ?? "";
      const noKey =
        creds.kind === "access_key_secret"
          ? !creds.api_key
          : creds.kind === "bearer_key"
            ? !creds.api_key.trim()
            : true;
      if (noKey) {
        f.routingProbe = {
          ok: false,
          status: null,
          url: baseUrl,
          error: get(t)("settings.accounts.routeKeyMissing"),
        };
      } else if (baseUrl) {
        const protocol = f.meta.provider_kind === "anthropic" ? "anthropic" : "openai";
        try {
          f.routingProbe = await testRoutingChannel(protocol, baseUrl, creds);
        } catch (e) {
          f.routingProbe = {
            ok: false,
            status: null,
            url: baseUrl,
            error: String(e),
          };
        }
      } else {
        f.routingProbe = null;
      }
    }
  }

  /** 「修改」已保存的凭据：从凭据管理器回填已存值到表单（密码态掩码显示）。
   *  没有这一步，密钥只写不读——用户只想补一个 TokenRouter 的可选路由 Key，
   *  却被迫重填 AK/SK，漏填旧字段还会在保存时把已存值清空。 */
  async function modifyCredentialsFor(f: AccountForm) {
    try {
      const stored = await getCredentials(f.meta.instance_id);
      if (stored) {
        if (stored.kind === "bearer_key") {
          f.apiKey = stored.api_key;
        } else if (stored.kind === "access_key_secret") {
          f.accessKey = stored.access_key;
          f.secretKey = stored.secret_key;
          f.routeApiKey = stored.api_key ?? "";
        } else if (stored.kind === "local_token") {
          f.token = stored.token;
        }
      }
    } catch (e) {
      // 静默吞掉会让「没有回读」变成无头案：把原因亮出来。
      f.error = get(t)("settings.accounts.credsReadBackError", { err: String(e) });
    }
    f.credsDirty = true;
  }

  /** 取消凭据编辑：丢弃未保存的修改，回到「已保存」摘要态。
   *  已存值不受影响；下次「修改」会重新从凭据管理器回填。 */
  function cancelCredentialsEdit(f: AccountForm) {
    f.error = null;
    f.testResult = null;
    if (f.hasCredentials) {
      f.credsDirty = false;
    } else {
      f.apiKey = "";
      f.accessKey = "";
      f.secretKey = "";
      f.routeApiKey = "";
      f.token = "";
      f.credsDirty = false;
    }
  }

  async function saveCredentialsFor(f: AccountForm) {
    if (!hasCredentialInput(f)) {
      f.error = get(t)("settings.accounts.errorFillCreds");
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
    f.routeApiKey = "";
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
        f.error = get(t)("settings.accounts.codexNotDetected");
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
    // 方舟 AgentPlan 的推理端点在 /api/plan/v3 下（查询用量的 volcengineapi.com
    // 是另一个 host）；路由器会自适应版本段，最终请求 .../v3/chat/completions。
    volcengine: "https://ark.cn-beijing.volces.com/api/plan/v3",
    volcengine_api: "https://ark.cn-beijing.volces.com/api/plan/v3",
    xiaomi_plan: "https://api.xiaomimimo.com",
    xiaomi_api: "https://api.xiaomimimo.com",
  };

  let routeSeq = 0;
  function nextRouteId(): string {
    routeSeq += 1;
    return `route-${Date.now().toString(36)}-${routeSeq}`;
  }

  /** 可加入路由链的账户：排除本地登录态（Codex 等——ChatGPT 后端不是开放
   *  API，无法转发）。AK/SK 账户保留（填了可选推理 Key 即可路由，见账户表单）。 */
  let routableAccounts = $derived(
    (catalog?.accounts ?? []).filter((a) => authKindFor(a.provider_kind) !== "local_token"),
  );

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
        name: get(t)("settings.router.defaultRouteName"),
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
      modelFetchError[key] = get(t)("settings.router.modelFetchNeedSetup");
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
        name: r.name.trim() || get(t)("settings.router.unnamedRoute"),
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
    ok: "settings.router.lineOk",
    low_quota: "settings.router.lineLowQuota",
    full: "settings.router.lineFull",
    cooldown: "settings.router.lineCooldown",
    unusable: "settings.router.lineUnusable",
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
    return secs > 90
      ? get(t)("settings.router.inMinutes", { n: Math.round(secs / 60) })
      : get(t)("settings.router.inSeconds", { n: secs });
  }

  function probeInText(iso?: string): string {
    if (!iso) return "";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const secs = Math.max(0, Math.round((d.getTime() - Date.now()) / 1000));
    return secs > 90
      ? get(t)("settings.router.inMinutes", { n: Math.round(secs / 60) })
      : get(t)("settings.router.inSeconds", { n: secs });
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

  /** 语言切换立即生效：先更新本地镜像与状态，再落盘后端（settings-changed
   *  会让其余窗口跟随）。落盘失败不回滚界面语言——下次保存会再写入。 */
  async function applyLanguageNow(e: Event) {
    const value = (e.currentTarget as HTMLSelectElement).value as LangSetting;
    language = value;
    applyLocaleSetting(value);
    if (!settings) return;
    try {
      await saveSettings({ ...settings, language: value });
      settings = { ...settings, language: value };
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

  /** 手动检查 GitHub Releases 新版本；结果内联展示，错误走内联提示。 */
  async function handleUpdateCheck() {
    updateChecking = true;
    updateError = null;
    try {
      updateInfo = await checkAppUpdate();
    } catch (e) {
      updateError = String(e);
    } finally {
      updateChecking = false;
    }
  }

  /** 仓库直达：URL 由后端常量下发，open_url 命令里再做一次白名单校验。 */
  async function handleOpenRepo() {
    if (!repoUrl) return;
    try {
      await openUrl(repoUrl);
      genericError = null;
    } catch (e) {
      genericError = String(e);
    }
  }

  async function handleOpenUpdateUrl(url: string) {
    try {
      await openUrl(url);
      genericError = null;
    } catch (e) {
      genericError = String(e);
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
        language,
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
        check_updates_on_start: checkUpdatesOnStart,
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
    if (r.ok) return get(t)("settings.accounts.testOk");
    return `✗ ${r.message}`;
  }

  function testResultClass(r: TestResult | null): string {
    if (!r) return "";
    return r.ok ? "test-ok" : "test-fail";
  }

  // Keep the flipped "确认删除" state harmless if the user switches accounts.
  function deleteLabel(f: AccountForm): string {
    return deleteConfirmId === f.meta.instance_id
      ? get(t)("settings.accounts.confirmDelete")
      : get(t)("settings.accounts.deleteTitle");
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
      <button type="button" class="settings__win-btn" aria-label={$t("settings.win.minimize")} onclick={handleMinimize} onpointerdown={(e) => e.stopPropagation()}>—</button>
      <button type="button" class="settings__win-btn" aria-label={$t("settings.win.maximize")} onclick={handleToggleMaximize} onpointerdown={(e) => e.stopPropagation()}>▢</button>
      <button class="settings__close" onclick={handleClose} aria-label={$t("settings.footer.saveClose")}>
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
        {$t("settings.nav.general")}
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "accounts"}
        onclick={() => (tab = "accounts")}
      >
        {$t("settings.nav.accounts")}
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "interaction"}
        onclick={() => (tab = "interaction")}
      >
        {$t("settings.nav.interaction")}
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "network"}
        onclick={() => (tab = "network")}
      >
        {$t("settings.nav.network")}
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "router"}
        onclick={() => {
          tab = "router";
          void refreshRouterStatus();
        }}
      >
        {$t("settings.nav.router")}
      </button>
      <button
        class="nav-item"
        class:is-active={tab === "about"}
        onclick={() => (tab = "about")}
      >
        {$t("settings.nav.about")}
      </button>
    </nav>

    <section class="settings__content">
      {#if genericError}
        <div class="global-error">{genericError}</div>
      {/if}

      {#if tab === "general"}
        <div class="pane">
          <h2 class="pane__title">{$t("settings.nav.general")}</h2>

          <div class="section">
            <h3 class="section__title">{$t("settings.general.language")}</h3>
            <p class="hint">{$t("settings.general.languageHint")}</p>
            <label class="interval">
              <select class="interval-select" value={language} onchange={applyLanguageNow}>
                <option value="auto">{$t("settings.general.languageAuto")}</option>
                <option value="zh-CN">{$t("settings.general.languageZh")}</option>
                <option value="en">{$t("settings.general.languageEn")}</option>
              </select>
            </label>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.polling.title")}</h3>
            <label class="interval">
              <input
                type="number"
                min="0"
                max="3600"
                step="30"
                bind:value={pollInterval}
              />
              <span class="interval__hint">{$t("settings.polling.secondsHint")}</span>
            </label>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.ring.title")}</h3>
            <p class="hint">{$t("settings.ring.hint")}</p>
            <label class="interval">
              <select class="interval-select" bind:value={ringWindow}>
                <option value="auto">{$t("settings.ring.auto")}</option>
                <option value="five_hour">{$t("settings.ring.fiveHour")}</option>
                <option value="daily">{$t("settings.ring.daily")}</option>
                <option value="weekly">{$t("settings.ring.weekly")}</option>
                <option value="monthly">{$t("settings.ring.monthly")}</option>
              </select>
            </label>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.ring.countdownMode")}</span>
                <span class="behavior-hint">{$t("settings.ring.countdownHint")}</span>
              </div>
              <label class="toggle">
                <input type="checkbox" onchange={applyCountdownNow} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.currency.title")}</h3>
            <p class="hint">{$t("settings.currency.hint")}</p>
            <label class="interval">
              <select class="interval-select" bind:value={displayCurrency}>
                <option value="auto">{$t("settings.currency.auto")}</option>
                <option value="CNY">{$t("currency.CNY")}</option>
                <option value="USD">{$t("currency.USD")}</option>
                <option value="TWD">{$t("currency.TWD")}</option>
                <option value="HKD">{$t("currency.HKD")}</option>
                <option value="JPY">{$t("currency.JPY")}</option>
                <option value="EUR">{$t("currency.EUR")}</option>
                <option value="GBP">{$t("currency.GBP")}</option>
              </select>
            </label>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.rates.title")}</h3>
            <p class="hint">{$t("settings.rates.hint")}</p>
            <div class="rate-grid">
              {#each OVERRIDE_CODES as c (c.code)}
                <label class="field">
                  <span class="field__label">{$t(c.label)}</span>
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
                    {$t("settings.rates.updatedAt", { time: formatRateTime(ratesSnapshot.fetched_at) })}
                  {:else}
                    {$t("settings.rates.notFetched")}
                  {/if}
                </span>
                <span class="behavior-hint">
                  {ratesSnapshot?.warning ?? $t("settings.rates.overrideHint")}
                </span>
              </div>
              <button class="btn btn--ghost" type="button" disabled={rateRefreshing} onclick={handleRateRefresh}>
                {rateRefreshing ? $t("settings.rates.refreshing") : $t("settings.rates.refreshNow")}
              </button>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.tabs.title")}</h3>
            <p class="hint">{$t("settings.tabs.hint")}</p>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.tabs.aggMode")}</span>
                <span class="behavior-hint">
                  {$t("settings.tabs.aggModeHint")}
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
              { key: "trend", label: "tab.trend", hint: "settings.tabs.trendHint" },
              { key: "tools", label: "tab.tools", hint: "settings.tabs.toolsHint" },
              { key: "models", label: "tab.models", hint: "settings.tabs.modelsHint" },
              { key: "devices", label: "tab.devices", hint: "settings.tabs.devicesHint" },
            ] as item (item.key)}
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">{$t(item.label)}</span>
                  <span class="behavior-hint">{$t(item.hint)}</span>
                </div>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={tabEnabled(item.key as PageTab)}
                    onchange={(e) => toggleTab(item.key as PageTab, (e.currentTarget as HTMLInputElement).checked)}
                  />
                  <span class="toggle__track"><span class="toggle__thumb"></span></span>
                </label>
              </div>
            {/each}
          </div>
        </div>

      {:else if tab === "accounts"}
        <div class="pane">
          <h2 class="pane__title">{$t("settings.nav.accounts")}</h2>
          <p class="hint">
            {$t("settings.accounts.hint")}
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
              {$t("settings.accounts.configured")}
              <span class="section__count">{forms.length}</span>
            </h3>

            {#if forms.length === 0}
              <div class="empty">{$t("settings.accounts.empty")}</div>
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
                      {f.meta.label || $t("settings.accounts.unnamed")}
                    </span>
                    <span class="acct-row__kind">{displayNameFor(f.meta.provider_kind)}</span>
                    {#if f.hasCredentials}
                      <span class="acct-row__dot" title={$t("settings.accounts.credsStored")} aria-label={$t("settings.accounts.credsStored")}></span>
                    {/if}
                    <span
                      class="account__badge acct-row__live"
                      class:account__badge--live={f.live}
                      class:account__badge--off={!f.live}
                    >
                      {f.live ? $t("settings.accounts.live") : $t("settings.accounts.stopped")}
                    </span>
                    <label class="toggle" title={f.meta.enabled ? $t("settings.common.enabled") : $t("settings.common.disabled")}>
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
              {$t("settings.accounts.add")}
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
                      <span class="field__label">{$t("settings.accounts.label")}</span>
                      <input
                        class="field__input"
                        type="text"
                        placeholder={$t("settings.accounts.labelPlaceholder")}
                        bind:value={selectedForm.meta.label}
                      />
                    </label>
                    <label class="field field--swatch">
                      <span class="field__label">{$t("settings.accounts.accent")}</span>
                      <span class="colorpicker">
                        <input
                          type="color"
                          bind:value={selectedForm.meta.accent_color}
                          aria-label={$t("settings.accounts.accent")}
                        />
                        <span class="colorpicker__hex">{selectedForm.meta.accent_color}</span>
                      </span>
                    </label>
                  </div>

                  <label class="field">
                    <span class="field__label">{$t("settings.accounts.note")}</span>
                    <input
                      class="field__input"
                      type="text"
                      placeholder={$t("settings.accounts.notePlaceholder")}
                      bind:value={selectedForm.meta.note}
                    />
                  </label>

                  {#if selectedForm.hasCredentials && !selectedForm.credsDirty}
                    <div class="account__stored">
                      <span class="account__badge account__badge--ok">{$t("settings.accounts.savedBadge")}</span>
                      <button class="btn btn--ghost" onclick={() => modifyCredentialsFor(selectedForm)}>{$t("settings.accounts.edit")}</button>
                      <button class="btn btn--ghost" onclick={() => clearCredentialsFor(selectedForm)}>{$t("settings.accounts.clear")}</button>
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
                      <div class="creds-optional">
                        <div class="creds-optional__title">
                          {$t("settings.accounts.routeKeyTitle")}
                        </div>
                        <p class="creds-optional__desc">
                          {$t("settings.accounts.routeKeyDesc1")}<strong>{$t("settings.accounts.routeKeyQuota")}</strong>{$t("settings.accounts.routeKeyDesc2")}<strong>{$t("settings.accounts.routeKeyInference")}</strong>{$t("settings.accounts.routeKeyDesc3")}<strong>{$t("settings.accounts.routeKeyNoRoute")}</strong>{$t("settings.accounts.routeKeyDesc4")}
                        </p>
                        <label class="field">
                          <span class="field__label">{$t("settings.accounts.routeKeyLabel")}</span>
                          <input
                            class="field__input"
                            type="password"
                            placeholder={$t("settings.accounts.routeKeyPlaceholder")}
                            bind:value={selectedForm.routeApiKey}
                            oninput={() => (selectedForm.credsDirty = true)}
                          />
                        </label>
                      </div>
                    {:else if isLocalToken(selectedForm.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">{$t("settings.accounts.localToken")}</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder={$t("settings.accounts.tokenPlaceholder")}
                          bind:value={selectedForm.token}
                          oninput={() => (selectedForm.credsDirty = true)}
                        />
                      </label>
                      <div class="account__actions">
                        <button class="btn btn--ghost" onclick={() => detectCodexFor(selectedForm)}>
                          {$t("settings.accounts.detectCodex")}
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
                        {selectedForm.testing ? $t("settings.common.testing") : $t("settings.common.testConnection")}
                      </button>
                      <button
                        class="btn btn--primary"
                        disabled={!hasCredentialInput(selectedForm)}
                        onclick={() => saveCredentialsFor(selectedForm)}
                      >
                        {selectedForm.saved ? $t("settings.common.savedCheck") : $t("settings.accounts.saveCreds")}
                      </button>
                      {#if selectedForm.hasCredentials}
                        <button class="btn btn--ghost" onclick={() => clearCredentialsFor(selectedForm)}>{$t("settings.accounts.clear")}</button>
                        <button class="btn btn--ghost" onclick={() => cancelCredentialsEdit(selectedForm)}>{$t("common.cancel")}</button>
                      {/if}
                    </div>
                  {/if}

                  {#if selectedForm.testResult}
                    <div class="account__test {testResultClass(selectedForm.testResult)}">
                      {formatTestResult(selectedForm.testResult)}
                    </div>
                  {/if}
                  {#if selectedForm.routingProbe}
                    {@const p = selectedForm.routingProbe}
                    <div
                      class="account__test"
                      class:test-ok={p.ok}
                      class:test-fail={!p.ok && p.status !== 404}
                    >
                      {#if p.ok}
                        {$t("settings.router.probeOk")}{p.models ? $t("settings.router.probeModels", { n: p.models }) : ""}
                      {:else if p.status === 404}
                        {$t("settings.router.probeNotImplemented")}
                      {:else}
                        {$t("settings.router.probeFail", { err: p.error ?? $t("settings.router.probeUnreachable") })}
                      {/if}
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
                        ? $t("settings.accounts.deleting")
                        : deleteLabel(selectedForm)}
                    </button>
                    {#if deleteConfirmId === selectedForm.meta.instance_id}
                      <span class="account__danger-hint">{$t("settings.accounts.deleteHint")}</span>
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
            <div class="preset-picker" role="dialog" aria-modal="true" aria-label={$t("settings.accounts.addTitle")}>
              <header class="preset-picker__head">
                <span class="preset-picker__title">{$t("settings.accounts.pickSource")}</span>
                <button
                  class="preset-picker__close"
                  onclick={() => {
                    pickerOpen = false;
                    presetMenuFor = null;
                  }}
                  aria-label={$t("common.close")}
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
                            <span class="preset-card__exp">{$t("settings.accounts.experimental")}</span>
                          {/if}
                        </span>
                        <span class="preset-card__auth">
                          {preset.auth_kind === "access_key_secret"
                            ? "Access Key + Secret"
                            : preset.auth_kind === "local_token"
                              ? $t("settings.accounts.localToken")
                              : "Bearer API Key"}
                        </span>
                      </span>
                      <span class="preset-card__add">
                        {busy === `add-${preset.kind}`
                          ? $t("settings.accounts.adding")
                          : preset.sub_modes?.length
                            ? $t("settings.accounts.pickBilling")
                            : $t("settings.accounts.addShort")}
                      </span>
                    </button>
                    {#if presetMenuFor?.kind === preset.kind && preset.sub_modes?.length}
                      <div class="preset-menu" role="menu" aria-label={$t("settings.accounts.pickBillingFor", { name: preset.display_name })}>
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
                              <span class="preset-menu__badge">{$t("settings.accounts.notImplemented")}</span>
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
          <h2 class="pane__title">{$t("settings.nav.interaction")}</h2>

          <div class="section">
            <h3 class="section__title">{$t("settings.behavior.title")}</h3>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.behavior.edgeSnap")}</span>
                <span class="behavior-hint">{$t("settings.behavior.edgeSnapHint")}</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={edgeSnap} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>

            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.behavior.autostart")}</span>
                <span class="behavior-hint">{$t("settings.behavior.autostartHint")}</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={autostart} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.sync.title")}</h3>
            <p class="hint">{$t("settings.sync.hint")}</p>
            <label class="interval">
              <select class="interval-select" bind:value={hubMode}>
                <option value="off">{$t("settings.sync.off")}</option>
                <option value="lan">{$t("settings.sync.lan")}</option>
                <option value="hub">{$t("settings.sync.hub")}</option>
                <option value="agent">{$t("settings.sync.agent")}</option>
              </select>
            </label>
            {#if hubMode === "hub" || hubMode === "lan"}
              <label class="interval">
                <input type="number" min="1024" max="65535" bind:value={hubPort} />
                <span class="interval__hint">{$t("settings.sync.listenPort")}</span>
              </label>
            {/if}
            {#if hubMode === "lan"}
              <p class="hint">
                {$t("settings.sync.lanHint")}
              </p>
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">{$t("settings.sync.reportUsage")}</span>
                  <span class="behavior-hint">{$t("settings.sync.reportLanHint")}</span>
                </div>
                <label class="toggle">
                  <input type="checkbox" bind:checked={reportOn} />
                  <span class="toggle__track"><span class="toggle__thumb"></span></span>
                </label>
              </div>
              <p class="hint">
                {$t("settings.sync.secretHint1")}<strong>{$t("settings.sync.sameSecret")}</strong>{$t("settings.sync.secretHint2")}
              </p>
            {/if}
            {#if hubMode === "agent"}
              <label class="interval">
                <input type="text" placeholder="http://192.168.1.20:43210" bind:value={hubBase} />
                <span class="interval__hint">{$t("settings.sync.hubAddress")}</span>
              </label>
              <div class="behavior-row">
                <div class="behavior-info">
                  <span class="behavior-label">{$t("settings.sync.reportUsage")}</span>
                  <span class="behavior-hint">{$t("settings.sync.reportAgentHint")}</span>
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
                  placeholder={$t("settings.sync.tokenPlaceholder")}
                  bind:value={hubToken}
                />
                <span class="interval__hint">{$t("settings.sync.tokenHint")}</span>
              </label>
            {/if}
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.notify.title")}</h3>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.notify.enable")}</span>
                <span class="behavior-hint">{$t("settings.notify.enableHint")}</span>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={notifyEnabled} />
                <span class="toggle__track"><span class="toggle__thumb"></span></span>
              </label>
            </div>
            <div class="threshold-grid">
              <label class="field">
                <span class="field__label">{$t("settings.notify.warn")}</span>
                <input
                  class="field__input"
                  type="number"
                  min="0"
                  max="100"
                  bind:value={notifyWarn}
                />
              </label>
              <label class="field">
                <span class="field__label">{$t("settings.notify.crit")}</span>
                <input
                  class="field__input"
                  type="number"
                  min="0"
                  max="100"
                  bind:value={notifyCrit}
                />
              </label>
            </div>
            <p class="hint">{$t("settings.notify.hint")}</p>
          </div>
        </div>

      {:else if tab === "network"}
        <div class="pane">
          <h2 class="pane__title">{$t("settings.nav.network")}</h2>

          <div class="section">
            <h3 class="section__title">{$t("settings.proxy.title")}</h3>
            <p class="hint">
              {$t("settings.proxy.hint")}
            </p>
            <div class="behavior-row">
              <div class="behavior-info">
                <span class="behavior-label">{$t("settings.proxy.enable")}</span>
                <span class="behavior-hint">{$t("settings.proxy.enableHint")}</span>
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
                  placeholder={$t("settings.proxy.urlPlaceholder")}
                  bind:value={proxyUrl}
                />
                <span class="interval__hint">{$t("settings.proxy.address")}</span>
              </label>
              <div class="account__actions">
                <button class="btn btn--ghost" disabled={proxyTesting || proxyUrl.trim().length === 0} onclick={() => handleTestProxy()}>
                  {proxyTesting ? $t("settings.common.testing") : $t("settings.common.testConnection")}
                </button>
              </div>
              {#if proxyResult}
                <div class="account__test {proxyResult.ok ? "test-ok" : "test-fail"}">
                  {proxyResult.ok
                    ? $t("settings.proxy.testOk", { status: proxyResult.status ?? "?" })
                    : `✗ ${proxyResult.error ?? $t("settings.proxy.testFailDefault")}`}
                </div>
              {/if}
            {/if}
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.tools.title")}</h3>
            <p class="hint">
              {$t("settings.tools.hintA")}<code>.claude</code>{$t("settings.tools.hintSep")}<code>.zcode</code>{$t("settings.tools.hintSep")}<code>.minimax</code>{$t("settings.tools.hintB")}<strong>{$t("settings.tools.hintExtraRoot")}</strong>{$t("settings.tools.hintC")}<strong>{$t("settings.tools.hintRecent")}</strong>{$t("settings.tools.hintD")}
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
                    aria-label={$t("settings.tools.removeRoot")}
                    onclick={() => toolDataRoots = toolDataRoots.filter((_, j) => j !== i)}
                  >×</button>
                </div>
              {/each}
            </div>
            <div class="account__actions">
              <button
                class="btn btn--ghost"
                onclick={() => toolDataRoots = [...toolDataRoots, ""]}
              >{$t("settings.tools.addRoot")}</button>
            </div>

            <h3 class="section__title" style="margin-top: 14px;">{$t("settings.tools.perTool")}</h3>
            <p class="hint">{$t("settings.tools.perToolHint")}</p>
            {#each TOOL_DATA_DIR_KEYS as key (key)}
              <label class="field">
                <span class="field__label">{$t(TOOL_DATA_LABELS[key])}</span>
                <input
                  class="field__input"
                  type="text"
                  placeholder={$t("settings.tools.perToolPlaceholder")}
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
                <span class="line-badge line-badge--bad"><i class="rt-dot rt-dot--err"></i>{$t("settings.router.portError")}</span>
              {:else if routerCfg.enabled && routerStatus?.health.listening}
                <span class="line-badge line-badge--ok"><i class="rt-dot rt-dot--live"></i>{$t("settings.router.serving")}</span>
              {:else if routerCfg.enabled}
                <span class="line-badge"><i class="rt-dot"></i>{$t("settings.common.enabled")}</span>
              {:else}
                <span class="line-badge"><i class="rt-dot"></i>{$t("settings.common.disabled")}</span>
              {/if}
              <span class="spacer"></span>
              <label class="rt-switch" title={$t("settings.router.masterSwitchTitle")}>
                <input type="checkbox" bind:checked={routerCfg.enabled} />
                <span class="rt-switch__track"><span class="rt-switch__thumb"></span></span>
              </label>
            </div>
            <p class="rt-desc">{$t("settings.router.desc")}</p>
            <div class="rt-addr-row">
              <code class="rt-addr">http://127.0.0.1:{routerCfg.port}</code>
              <button type="button" class="btn btn--ghost" onclick={copyRouterUrl}>
                {routerUrlCopied ? $t("settings.router.copied") : $t("settings.router.copyAddress")}
              </button>
              <span class="rt-hint">{$t("settings.router.addrHint")}</span>
              <span class="spacer"></span>
              <button type="button" class="rt-link" onclick={() => (svcOpen = !svcOpen)}>
                {svcOpen ? $t("settings.router.svcCollapse") : $t("settings.router.svcExpand")}
              </button>
            </div>

            {#if svcOpen}
              <div class="rt-sect rt-sect--in">
                <h3 class="rt-sect__title">{$t("settings.router.svcTitle")} <span class="rt-sect__sub">{$t("settings.router.svcSub")}</span></h3>
                <div class="rt-grid">
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.proactiveLabel")}</span>
                    <input class="rt-input" type="number" min="0" max="95" bind:value={routerCfg.proactive_threshold_percent} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.cooldownLabel")}</span>
                    <input class="rt-input" type="number" min="5" max="86400" bind:value={routerCfg.error_cooldown_secs} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.breakerLabel")}</span>
                    <input class="rt-input" type="number" min="1" max="20" bind:value={routerCfg.conn_breaker_count} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.probeStartLabel")}</span>
                    <input class="rt-input" type="number" min="10" max="3600" bind:value={routerCfg.probe_start_secs} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.probeMaxLabel")}</span>
                    <input class="rt-input" type="number" min="1" max="20" bind:value={routerCfg.probe_max_attempts} />
                  </label>
                  <label class="rt-field">
                    <span class="rt-label">{$t("settings.router.portLabel")}</span>
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
                    <span>{$t("settings.router.gaugeSwitchAt", { n: routerCfg.proactive_threshold_percent })}</span>
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
                    aria-label={$t("settings.router.proactiveAria")}
                  />
                </div>
                <div class="rt-gauge__scale"><span>0%</span><span>25%</span><span>50%</span><span>75%</span><span>100%</span></div>
                <div class="rt-gauge__state" class:rt-gauge__state--warn={gaugeRemaining !== null && gaugeRemaining <= routerCfg.proactive_threshold_percent}>
                  {#if gaugeRemaining === null}
                    {$t("settings.router.gaugeNoDataA")}<strong>{$t("settings.router.gaugeNoDataStrong")}</strong>{$t("settings.router.gaugeNoDataB")}
                  {:else if gaugeRemaining <= routerCfg.proactive_threshold_percent}
                    {$t("settings.router.gaugeRemainA")} <strong>{Math.round(gaugeRemaining)}%</strong>{$t("settings.router.gaugeBelowB")} <strong>{routerCfg.proactive_threshold_percent}%</strong>{$t("settings.router.gaugeBelowC")}
                  {:else}
                    {$t("settings.router.gaugeRemainA")} <strong>{Math.round(gaugeRemaining)}%</strong>{$t("settings.router.gaugeOkB")} <strong>{Math.round(gaugeRemaining - routerCfg.proactive_threshold_percent)}%</strong>{$t("settings.router.gaugeOkC")}
                  {/if}
                </div>

                <!-- 切回探视时间线 -->
                {#if routerStatus?.route?.probe}
                  {@const pr = routerStatus.route.probe}
                  <div class="rt-probe rt-sect--in">
                    <div class="rt-probe__t">
                      {$t("settings.router.probing", { n: pr.attempts, total: routerCfg.probe_max_attempts })}
                      <span class="line-badge line-badge--ok"><i class="rt-dot rt-dot--live"></i>{$t("settings.router.probeQualified")}</span>
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
                    <p class="rt-hint">{$t("settings.router.nextProbeA")}{probeInText(pr.next_probe_at)}{$t("settings.router.nextProbeB")}</p>
                  </div>
                {/if}

                <div class="rt-howto">
                  <strong>{$t("settings.router.fourLayersTitle")}</strong>{$t("settings.router.fourLayers")}
                </div>
              </div>
            {/if}
          </section>

          <!-- ═══════ 路由链卡 ═══════ -->
          {#if !route}
            <section class="rt-card rt-empty">
              <h2 class="rt-card__title">{$t("settings.router.emptyTitle")}</h2>
              <p class="rt-desc">{$t("settings.router.emptyDesc")}</p>
              <button type="button" class="btn btn--primary" onclick={ensureRoute}>{$t("settings.router.createRoute")}</button>
            </section>
          {:else}
            {@const st = routerStatus?.route}
            <section class="rt-card" class:rt-card--off={!route.on}>
              <div class="rt-card__hd">
                <input class="rt-card__name" type="text" bind:value={route.name} placeholder={$t("settings.router.routeName")} />
                <span class="line-badge">{route.protocol === "openai" ? $t("settings.router.openaiCompat") : "Anthropic"}</span>
                <span class="spacer"></span>
                <label class="rt-switch" title={$t("settings.router.chainSwitchTitle")}>
                  <input type="checkbox" bind:checked={route.on} />
                  <span class="rt-switch__track"><span class="rt-switch__thumb"></span></span>
                </label>
                <button type="button" class="rt-link" onclick={() => (chainOpen = !chainOpen)}>
                  {$t("settings.router.currentlyOn")} {route.candidates[st?.active_index ?? 0]?.model || "—"} {chainOpen ? "▼" : "▲"}
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
                    {cand.model || $t("settings.router.noModel")}
                  </button>
                  {#if i < route.candidates.length - 1}<span class="rt-arrow">→</span>{/if}
                {/each}
                <span class="spacer"></span>
                <code class="rt-key" title={route.token}>{route.token ? `${route.token.slice(0, 7)}…${route.token.slice(-6)}` : $t("settings.router.pendingToken")}</code>
                <button type="button" class="btn btn--ghost btn--sm" onclick={() => copyRouteToken(route.token)} disabled={!route.token}>
                  {tokenCopied ? $t("settings.router.copied") : $t("settings.router.copy")}
                </button>
              </div>

              <div class="rt-collapse" class:rt-collapse--open={chainOpen}>
                <div>
                  <div class="rt-sect">
                    <div class="rt-grid">
                      <label class="rt-field">
                        <span class="rt-label">{$t("settings.router.routeName")}</span>
                        <input class="rt-input" type="text" bind:value={route.name} />
                      </label>
                      <label class="rt-field">
                        <span class="rt-label" title={$t("settings.router.protocolTitle")}>{$t("settings.router.protocolLabel")}</span>
                        <select class="rt-input" bind:value={route.protocol}>
                          <option value="openai">{$t("settings.router.openaiCompat")}</option>
                          <option value="anthropic">Anthropic</option>
                        </select>
                      </label>
                    </div>
                    <label class="rt-field" style="margin-top:12px">
                      <span class="rt-label">{$t("settings.router.apiKeyLabel")}</span>
                      <span class="rt-url-row">
                        <input class="rt-input rt-mono" type="text" value={route.token || $t("settings.router.tokenAuto")} readonly />
                        <button type="button" class="btn btn--ghost" onclick={() => copyRouteToken(route.token)} disabled={!route.token}>{$t("settings.router.copy")}</button>
                      </span>
                    </label>
                  </div>

                  <div class="rt-sect">
                    <h3 class="rt-sect__title">
                      {$t("settings.router.chainTitle")} <span class="rt-sect__sub">{$t("settings.router.chainSub")}</span>
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
                            <span class="rt-line__ttl">{i === 0 ? $t("settings.router.primary") : $t("settings.router.backupN", { n: i })}</span>
                            {#if cs}
                              <span class="line-badge {lineBadge(cs.state)}" title={$t("settings.router.remainingTitle")}>
                                <i class="rt-dot" style="background:{remainingColor(cs.remaining_percent)}"></i>
                                {LINE_STATE_LABELS[cs.state] ? $t(LINE_STATE_LABELS[cs.state]) : cs.state}{cs.remaining_percent !== undefined ? $t("settings.router.remainPct", { n: Math.round(cs.remaining_percent) }) : ""}
                              </span>
                            {/if}
                            <span class="spacer"></span>
                            <span class="ops">
                              <button type="button" class="rt-op" disabled={i === 0} onclick={() => moveCandidate(i, -1)} title={$t("settings.router.moveUp")}>↑</button>
                              <button type="button" class="rt-op" disabled={i === route.candidates.length - 1} onclick={() => moveCandidate(i, 1)} title={$t("settings.router.moveDown")}>↓</button>
                              <button type="button" class="rt-op rt-op--del" onclick={() => removeCandidate(i)} title={$t("settings.router.removeLine")}>✕</button>
                            </span>
                          </div>
                          <div class="rt-line__row">
                            <label class="rt-field">
                              <span class="rt-label" title={$t("settings.router.accountTitle")}>{$t("settings.router.accountLabel")}</span>
                              <select
                                class="rt-input"
                                value={cand.account}
                                onchange={(e) => onCandidateAccountChanged(cand, (e.currentTarget as HTMLSelectElement).value)}
                              >
                                <option value="">{$t("settings.router.pickAccount")}</option>
                                {#each routableAccounts as acct (acct.instance_id)}
                                  <option value={acct.instance_id}>
                                    {acct.label}{authKindFor(acct.provider_kind) === "access_key_secret" ? $t("settings.router.needsRouteKey") : ""}
                                  </option>
                                {/each}
                              </select>
                            </label>
                            <label class="rt-field">
                              <span class="rt-label" title={$t("settings.router.modelTitle")}>{$t("settings.router.modelLabel")}</span>
                              <span class="rt-model-wrap">
                                <input class="rt-input rt-mono" type="text" bind:value={cand.model} placeholder={$t("settings.router.modelPlaceholder")} />
                                <button
                                  type="button"
                                  class="rt-op"
                                  title={$t("settings.router.fetchModelsTitle")}
                                  disabled={modelFetching[modelKey(cand.account, cand.base_url)] || !cand.account || !cand.base_url}
                                  onclick={() => handleFetchModels(cand, route.protocol)}
                                >{modelFetching[modelKey(cand.account, cand.base_url)] ? "…" : "▼"}</button>
                                {#if modelPickerFor === modelKey(cand.account, cand.base_url) && (modelOptions[modelKey(cand.account, cand.base_url)]?.length ?? 0) > 0}
                                  <span class="rt-models">
                                    <b>{$t("settings.router.fetchedModels", { n: modelOptions[modelKey(cand.account, cand.base_url)].length })}</b>
                                    {#each modelOptions[modelKey(cand.account, cand.base_url)] ?? [] as m (m)}
                                      <button type="button" class:rt-models--sel={m === cand.model} onclick={() => pickModel(cand, m)}>{m}</button>
                                    {/each}
                                  </span>
                                {/if}
                              </span>
                            </label>
                          </div>
                          <label class="rt-field">
                            <span class="rt-label" title={$t("settings.router.baseUrlTitle")}>{$t("settings.router.baseUrlLabel")}</span>
                            <span class="rt-url-row">
                              <input class="rt-input rt-mono" type="text" bind:value={cand.base_url} placeholder={$t("settings.router.baseUrlPlaceholder")} />
                              {#if cand.base_url === (ROUTER_BASE_PRESETS[accountKindOf(cand.account)] ?? "")}
                                <span class="rt-tag">{$t("settings.router.autoFilled")}</span>
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
                            <p class="rt-hint">{$t("settings.router.cooldown", { until: cooldownUntilText(cs.cooldown_until), reason: cs.cooldown_reason === "quota" ? $t("settings.router.reasonQuota") : cs.cooldown_reason === "auth" ? $t("settings.router.reasonAuth") : $t("settings.router.reasonConn") })}</p>
                          {/if}
                          <details class="rt-cap">
                            <summary>
                              {$t("settings.router.capTitle")}
                              {#if cand.plan_limit_tokens_daily || cand.monthly_cost_limit}
                                {$t("settings.router.daily")} <b>{cand.plan_limit_tokens_daily || $t("settings.router.unset")}</b>{cand.plan_limit_tokens_daily ? " tokens" : ""} ·
                                {$t("settings.router.monthly")} <b>{cand.monthly_cost_limit || $t("settings.router.unset")}</b>{cand.monthly_cost_limit ? $t("settings.router.amountSuffix") : ""}
                              {:else}
                                {$t("settings.router.capUnset")}
                              {/if}
                            </summary>
                            <div class="rt-cap__body">
                              <label class="rt-field">
                                <span class="rt-label" title={$t("settings.router.dailyLimitTitle")}>{$t("settings.router.dailyLimitLabel")}</span>
                                <input class="rt-input" type="number" min="0" bind:value={cand.plan_limit_tokens_daily} placeholder={$t("settings.router.unset")} />
                              </label>
                              <label class="rt-field">
                                <span class="rt-label" title={$t("settings.router.monthlyLimitTitle")}>{$t("settings.router.monthlyLimitLabel")}</span>
                                <input class="rt-input" type="number" min="0" bind:value={cand.monthly_cost_limit} placeholder={$t("settings.router.monthlyLimitPlaceholder")} />
                              </label>
                            </div>
                          </details>
                        </div>
                      {/each}
                    </div>
                    <div class="rt-foot">
                      <button type="button" class="btn btn--ghost" onclick={addCandidate}>{$t("settings.router.addBackup")}</button>
                      <span class="spacer"></span>
                      <span class="rt-hint">{$t("settings.router.footHint")}</span>
                    </div>
                  </div>
                </div>
              </div>
            </section>
          {/if}
        </div>

      {:else}
        <div class="pane">
          <h2 class="pane__title">{$t("settings.nav.about")}</h2>

          <div class="section">
            <h3 class="section__title">
              TokenUsageMonitor {#if appVersion}<span class="about-ver">v{appVersion}</span>{/if}
            </h3>
            <p class="hint">
              {$t("settings.about.desc")}
            </p>
            <ul class="about-list">
              <li>{$t("settings.about.feat1")}</li>
              <li>{$t("settings.about.feat2")}</li>
              <li>{$t("settings.about.feat3")}</li>
              <li>{$t("settings.about.feat4")}</li>
              <li>{$t("settings.about.feat5")}</li>
            </ul>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.about.projectTitle")}</h3>
            <p class="hint">
              {#if repoUrl}
                <button type="button" class="link-btn" onclick={handleOpenRepo}>{repoUrl}</button>
              {:else}
                {$t("settings.about.repoFallback")}
              {/if}
              {$t("settings.about.repoTail")}
            </p>
            <div class="account__actions">
              <button class="btn btn--ghost" disabled={updateChecking} onclick={handleUpdateCheck}>
                {updateChecking ? $t("settings.about.checking") : $t("settings.about.checkUpdates")}
              </button>
            </div>
            {#if updateError}
              <p class="hint about-update__error">{$t("settings.about.checkFailed", { err: updateError })}</p>
            {:else if updateInfo}
              {#if updateInfo.has_update}
                <p class="hint">
                  {$t("settings.about.updateFound")} <b>{updateInfo.latest}</b>{$t("settings.about.updateFoundCur", { cur: updateInfo.current })}
                  {#if updateInfo.url}
                    <button type="button" class="link-btn" onclick={() => updateInfo?.url && handleOpenUpdateUrl(updateInfo.url)}>
                      {$t("settings.about.gotoReleases")}
                    </button>
                  {/if}
                </p>
              {:else if updateInfo.latest}
                <p class="hint">{$t("settings.about.upToDate", { cur: updateInfo.current })}</p>
              {:else}
                <p class="hint">{$t("settings.about.noReleases")}</p>
              {/if}
            {/if}
            <label class="toggle">
              <input
                type="checkbox"
                checked={checkUpdatesOnStart}
                onchange={(e) => (checkUpdatesOnStart = (e.currentTarget as HTMLInputElement).checked)}
              />
              <span class="toggle__track"><span class="toggle__thumb"></span></span>
            </label>
            <span class="behavior-hint">{$t("settings.about.autoCheckHint")}</span>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.about.creditsTitle")}</h3>
            <p class="hint">{$t("settings.about.creditsHint")}</p>
            <ul class="about-list">
              <li><b>Tauri</b> {$t("settings.about.credTauri")}</li>
              <li><b>Svelte</b> {$t("settings.about.credSvelte")}</li>
              <li><b>tokio</b> {$t("settings.about.credTokio")}</li>
              <li><b>serde</b> {$t("settings.about.credSerde")}</li>
              <li><b>chrono</b> {$t("settings.about.credChrono")}</li>
              <li><b>reqwest / rustls</b> {$t("settings.about.credReqwest")}</li>
              <li><b>rusqlite</b> {$t("settings.about.credRusqlite")}</li>
              <li><b>keyring</b> {$t("settings.about.credKeyring")}</li>
            </ul>
            <p class="hint">{$t("settings.about.creditsTokscale")}</p>
          </div>

          <div class="section">
            <h3 class="section__title">{$t("settings.about.diagnostics")}</h3>
            <p class="hint">
              {$t("settings.about.diagnosticsLine", { total: forms.length, enabled: forms.filter((f) => f.meta.enabled).length || 0 })}
            </p>
            <div class="account__actions">
              <button class="btn btn--ghost" onclick={() => forceRefresh()}>
                {$t("settings.about.refreshAll")}
              </button>
            </div>
          </div>
        </div>
      {/if}
    </section>
  </div>

  <footer class="settings__footer">
    <button class="btn btn--ghost" onclick={handleClose}>{$t("settings.footer.saveClose")}</button>
    <button class="btn btn--primary" disabled={!canSave || saving} onclick={handleSaveAll}>
      {saving ? $t("settings.footer.saving") : savedFlash ? $t("settings.common.savedCheck") : $t("settings.footer.saveSettings")}
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

  /* AK/SK 账户的可选推理 Key 分组：弱化边框 + 说明文字，与上方必填凭据
   * 明确区隔，避免用户以为填了就能路由。 */
  .creds-optional {
    margin-top: 4px;
    padding: 11px 13px;
    border: 1px dashed var(--tum-border, rgba(255, 255, 255, 0.14));
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.02);
  }
  .creds-optional__title {
    font-size: 12.5px;
    font-weight: 650;
    color: var(--tum-text-secondary, #c9d1d9);
    margin-bottom: 5px;
  }
  .creds-optional__desc {
    margin: 0 0 10px;
    font-size: 12px;
    line-height: 1.65;
    color: var(--tum-text-muted, #8b949e);
  }
  .creds-optional__desc strong { color: var(--tum-text-primary, #e6edf3); font-weight: 650; }

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

  /* 关于页：仓库直达 / Releases 链接按钮（视觉是链接，行为走 open_url 白名单）。 */
  .link-btn {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--tum-accent, #4cc2ff);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .about-update__error {
    color: #ff6b6b;
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
  /* 字段标签固定单行高度：否则「模型（…）」这类长标签折行会把下方输入框
   * 顶低，与相邻字段错位。完整文案由 title 承载。 */
  .rt-label {
    font-size: 11.5px; font-weight: 600; color: var(--tum-text-muted, #8b949e);
    height: 16px; line-height: 16px; white-space: nowrap;
    overflow: hidden; text-overflow: ellipsis;
  }
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
    /* 圆点中心 = 卡片左边距 17 + 圆点偏移 9 + 半径 16 = 42px；2px 宽 → left 41px。
     * 之前写成 24px（漏算卡片边距），竖线整体左移 17px 不与圆点重合。 */
    left: 41px;
    top: 29px;
    bottom: 29px;
    width: 2px;
    background: var(--tum-border, rgba(255, 255, 255, 0.14));
    opacity: 0.85;
    pointer-events: none;
    z-index: 0;
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
    /* 不透明底：导轨从圆点正后方穿过，半透明底会看到线影 */
    background: var(--tum-surface, #161b22);
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