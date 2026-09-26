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
    type ProviderCatalog,
    type Preset,
    type PresetSubMode,
    type AccountMeta,
    type Settings,
    type Credentials,
    type TestResult,
    type ProxyTestResult,
    type RatesSnapshot,
    emitTabsChanged,
    readHiddenTabs,
    setTabEnabled,
    type PageTab,
  } from "./lib";
  import { CURRENCIES, USD_RATES, applyRatesSnapshot, type Currency } from "./lib/currency";
  import ProviderLogo from "./lib/components/ProviderLogo.svelte";

  type Tab = "general" | "accounts" | "interaction" | "network" | "about";

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
  let pollInterval = $state<number>(0);
  let ringWindow = $state<string>("auto");
  let edgeSnap = $state(true);
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
  let hubMode = $state("off");
  let hubPort = $state(43210);
  let hubBase = $state("");
  let reportOn = $state(false);
  let hubToken = $state("");
  // 网络代理（spec §3.4）：空字符串 = 不设置代理（跟随系统环境变量）。
  let proxyEnabled = $state(false);
  let proxyUrl = $state("");
  let proxyTesting = $state(false);
  let proxyResult = $state<ProxyTestResult | null>(null);
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
      forms = buildForms();
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
      await upsertAccount({
        instance_id: "",
        provider_kind: preset.kind,
        label: preset.display_name,
        accent_color: preset.default_accent,
        enabled: true,
      });
      await refreshAll();
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
        rate_overrides: buildRateOverrides(),
        proxy_url: proxyEnabled && proxyUrl.trim() ? proxyUrl.trim() : null,
      };
      await saveSettings(next);
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
  });
async function handleMinimize() {
    await getCurrentWindow().minimize();
  }
  async function handleToggleMaximize() {
    await getCurrentWindow().toggleMaximize();
  }
</script>

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
            从内置预设创建账户，可添加多个同一来源的账户。凭证保存在 Windows
            凭据管理器中（DPAPI 加密），不会写入磁盘明文。每个账户可自定义标签与强调色，并独立启停。
          </p>

          <div class="section">
            <h3 class="section__title">添加账户</h3>
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
              <div class="empty">还没有账户。请从上方选择一个预设添加。</div>
            {/if}

            {#each forms as f (f.meta.instance_id)}
              <article
                class="account"
                style="--acct-accent: {f.meta.accent_color}"
                data-enabled={f.meta.enabled}
              >
                <header class="account__header">
                  <div class="account__identity">
                    <span
                      class="account__swatch"
                      style="background: {f.meta.accent_color}"
                      aria-hidden="true"
                    ></span>
                    <input
                      class="account__label"
                      type="text"
                      placeholder="账户标签"
                      bind:value={f.meta.label}
                    />
                  </div>
                  <div class="account__badges">
                    <span class="account__kind">{displayNameFor(f.meta.provider_kind)}</span>
                    {#if f.hasCredentials}
                      <span class="account__badge account__badge--ok">凭证已存</span>
                    {/if}
                    <span
                      class="account__badge"
                      class:account__badge--live={f.live}
                      class:account__badge--off={!f.live}
                    >
                      {f.live ? "轮询中" : "已停止"}
                    </span>
                  </div>
                  <label class="toggle">
                    <input
                      type="checkbox"
                      checked={f.meta.enabled}
                      onchange={(e) => (f.meta.enabled = (e.target as HTMLInputElement).checked)}
                    />
                    <span class="toggle__track"><span class="toggle__thumb"></span></span>
                  </label>
                </header>

                <div class="account__body">
                  <div class="account__meta-row">
                    <label class="field field--flex">
                      <span class="field__label">标签</span>
                      <input
                        class="field__input"
                        type="text"
                        placeholder="显示名称"
                        bind:value={f.meta.label}
                      />
                    </label>
                    <label class="field field--swatch">
                      <span class="field__label">强调色</span>
                      <span class="colorpicker">
                        <input
                          type="color"
                          bind:value={f.meta.accent_color}
                          aria-label="强调色"
                        />
                        <span class="colorpicker__hex">{f.meta.accent_color}</span>
                      </span>
                    </label>
                  </div>

                  <label class="field">
                    <span class="field__label">备注（可选）</span>
                    <input
                      class="field__input"
                      type="text"
                      placeholder="例如：主账号 / 备用额度 …"
                      bind:value={f.meta.note}
                    />
                  </label>

                  {#if f.hasCredentials && !f.credsDirty}
                    <div class="account__stored">
                      <span class="account__badge account__badge--ok">已保存</span>
                      <button class="btn btn--ghost" onclick={() => (f.credsDirty = true)}>修改</button>
                      <button class="btn btn--ghost" onclick={() => clearCredentialsFor(f)}>清除</button>
                    </div>
                  {:else}
                    {#if isAccessKey(f.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">Access Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="AK..."
                          bind:value={f.accessKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                      <label class="field">
                        <span class="field__label">Secret Key</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="SK..."
                          bind:value={f.secretKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                    {:else if isLocalToken(f.meta.provider_kind)}
                      <label class="field">
                        <span class="field__label">本地登录凭证</span>
                        <input
                          class="field__input"
                          type="password"
                          placeholder="粘贴 Codex 登录令牌，或点击右侧自动检测"
                          bind:value={f.token}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                      <div class="account__actions">
                        <button class="btn btn--ghost" onclick={() => detectCodexFor(f)}>
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
                          bind:value={f.apiKey}
                          oninput={() => (f.credsDirty = true)}
                        />
                      </label>
                    {/if}

                    <div class="account__actions">
                      <button
                        class="btn btn--ghost"
                        disabled={f.testing || !hasCredentialInput(f)}
                        onclick={() => testConnection(f)}
                      >
                        {f.testing ? "测试中…" : "测试连接"}
                      </button>
                      <button
                        class="btn btn--primary"
                        disabled={!hasCredentialInput(f)}
                        onclick={() => saveCredentialsFor(f)}
                      >
                        {f.saved ? "已保存 ✓" : "保存凭证"}
                      </button>
                      {#if f.hasCredentials}
                        <button class="btn btn--ghost" onclick={() => clearCredentialsFor(f)}>清除</button>
                      {/if}
                    </div>
                  {/if}

                  {#if f.testResult}
                    <div class="account__test {testResultClass(f.testResult)}">
                      {formatTestResult(f.testResult)}
                    </div>
                  {/if}
                  {#if f.error}
                    <div class="account__error">{f.error}</div>
                  {/if}

                  <div class="account__danger">
                    <button
                      class="btn btn--danger"
                      class:btn--confirm={deleteConfirmId === f.meta.instance_id}
                      disabled={busy === f.meta.instance_id}
                      onclick={() => deleteAccount(f)}
                    >
                      {busy === f.meta.instance_id
                        ? "删除中…"
                        : deleteLabel(f)}
                    </button>
                    {#if deleteConfirmId === f.meta.instance_id}
                      <span class="account__danger-hint">再次点击将删除该账户及凭证</span>
                    {/if}
                  </div>
                </div>
              </article>
            {/each}
          </div>
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
          </div>

          <div class="section">
            <h3 class="section__title">多端同步</h3>
            <p class="hint">本机作为 hub 接收其它设备上报；或以 agent 把本机用量上报到指定 hub。改动需重启应用生效。</p>
            <label class="interval">
              <select class="interval-select" bind:value={hubMode}>
                <option value="off">关闭</option>
                <option value="hub">本机作为 hub（接收上报）</option>
                <option value="agent">作为 agent（上报到远端 hub）</option>
              </select>
            </label>
            {#if hubMode === "hub"}
              <label class="interval">
                <input type="number" min="1024" max="65535" bind:value={hubPort} />
                <span class="interval__hint">监听端口（默认 43210）</span>
              </label>
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
              <li>最小化资源占用：常驻内存 ≈ 39 MB</li>
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

  .account + .account {
    margin-top: var(--tum-space-3);
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
</style>