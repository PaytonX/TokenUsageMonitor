<script lang="ts">
  import { onMount } from "svelte";
  import {
    getProviders,
    getSettings,
    saveSettings,
    saveCredentials,
    deleteCredentials,
    testProvider,
    closeSettings,
    forceRefresh,
    type ProviderInfo,
    type Settings,
    type Credentials,
    type TestResult,
  } from "./lib";

  type ProviderFormState = {
    info: ProviderInfo;
    /** BearerKey fields */
    apiKey: string;
    /** AccessKeySecret fields */
    accessKey: string;
    secretKey: string;
    /** UI-only flags */
    dirty: boolean;
    testing: boolean;
    testResult: TestResult | null;
    saved: boolean;
    error: string | null;
  };

  let providers = $state<ProviderFormState[]>([]);
  let settings = $state<Settings | null>(null);
  let pollInterval = $state<number>(0);
  let ringWindow = $state<string>("auto");
  let edgeSnap = $state(true);
  let notifyEnabled = $state(true);
  let saving = $state(false);
  let savedFlash = $state(false);

  onMount(async () => {
    providers = (await getProviders()).map((info) => ({
      info,
      apiKey: "",
      accessKey: "",
      secretKey: "",
      dirty: false,
      testing: false,
      testResult: null,
      saved: false,
      error: null,
    }));
    settings = await getSettings();
    pollInterval = settings.poll_interval_seconds ?? 0;
    ringWindow = settings.ring_window ?? "auto";
    edgeSnap = settings.edge_snap ?? true;
    notifyEnabled = settings.notify_enabled ?? true;
  });

  function isAccessKey(info: ProviderInfo): boolean {
    return info.auth_kind === "access_key_secret";
  }

  function buildCredentials(p: ProviderFormState): Credentials {
    if (isAccessKey(p.info)) {
      return {
        kind: "access_key_secret",
        access_key: p.accessKey.trim(),
        secret_key: p.secretKey.trim(),
      };
    }
    return {
      kind: "bearer_key",
      api_key: p.apiKey.trim(),
    };
  }

  function hasCredentialInput(p: ProviderFormState): boolean {
    if (isAccessKey(p.info)) {
      return p.accessKey.trim().length > 0 && p.secretKey.trim().length > 0;
    }
    return p.apiKey.trim().length > 0;
  }

  async function toggleProvider(p: ProviderFormState, enabled: boolean) {
    if (!settings) return;
    if (enabled) {
      if (!settings.enabled_providers.includes(p.info.id)) {
        settings.enabled_providers = [...settings.enabled_providers, p.info.id];
      }
    } else {
      settings.enabled_providers = settings.enabled_providers.filter(
        (id) => id !== p.info.id,
      );
    }
    p.dirty = true;
  }

  async function testConnection(p: ProviderFormState) {
    if (!hasCredentialInput(p)) {
      p.error = "请先填写凭证";
      p.testResult = null;
      return;
    }
    p.testing = true;
    p.error = null;
    p.testResult = null;
    const result = await testProvider(p.info.id, buildCredentials(p));
    p.testing = false;
    p.testResult = result;
    if (result.ok) {
      p.dirty = true;
    }
  }

  async function saveProvider(p: ProviderFormState) {
    if (!hasCredentialInput(p)) {
      p.error = "请先填写凭证";
      return;
    }
    p.error = null;
    try {
      await saveCredentials(p.info.id, buildCredentials(p));
      p.dirty = false;
      p.saved = true;
      setTimeout(() => (p.saved = false), 2000);
    } catch (e) {
      p.error = String(e);
    }
  }

  async function clearProvider(p: ProviderFormState) {
    p.apiKey = "";
    p.accessKey = "";
    p.secretKey = "";
    p.testResult = null;
    p.error = null;
    try {
      await deleteCredentials(p.info.id);
      p.dirty = false;
    } catch (e) {
      p.error = String(e);
    }
  }

  async function handleSaveAll() {
    if (!settings) return;
    saving = true;
    try {
      const next: Settings = {
        ...settings,
        poll_interval_seconds: pollInterval,
        ring_window: ringWindow,
        edge_snap: edgeSnap,
        notify_enabled: notifyEnabled,
      };
      await saveSettings(next);
      settings = next;
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 2000);
      // Trigger a refresh so the dashboard picks up any newly-enabled providers.
      await forceRefresh();
    } finally {
      saving = false;
    }
  }

  async function handleClose() {
    await handleSaveAll();
    await closeSettings();
  }

  function formatTestResult(r: TestResult | null): string {
    if (!r) return "";
    if (r.ok) {
      return "✓ 连接成功";
    }
    return `✗ ${r.message}`;
  }

  function testResultClass(r: TestResult | null): string {
    if (!r) return "";
    return r.ok ? "test-ok" : "test-fail";
  }

  let canSave = $derived(settings !== null);
</script>

<main class="settings" oncontextmenu={(e) => e.preventDefault()}>
  <header class="settings__header">
    <div class="settings__brand">
      <span class="settings__dot"></span>
      <span class="settings__title">TokenUsageMonitor · 设置</span>
    </div>
  </header>

  <section class="settings__section">
    <h2 class="settings__section-title">轮询间隔</h2>
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
  </section>

  <section class="settings__section">
    <h2 class="settings__section-title">环形用量窗口</h2>
    <p class="settings__hint">
      主面板与迷你胶囊的百分比圆环基于哪个窗口计算。
    </p>
    <label class="interval">
      <select class="interval-select" bind:value={ringWindow}>
        <option value="auto">自动（各来源最紧张窗口）</option>
        <option value="five_hour">5 小时窗口</option>
        <option value="daily">当日窗口</option>
        <option value="weekly">周用量窗口</option>
        <option value="monthly">月度窗口</option>
      </select>
    </label>
  </section>

  <section class="settings__section">
    <h2 class="settings__section-title">行为</h2>
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
        <span class="behavior-label">用量告急通知</span>
        <span class="behavior-hint">用量跨越警告/告急阈值时弹出系统通知。</span>
      </div>
      <label class="toggle">
        <input type="checkbox" bind:checked={notifyEnabled} />
        <span class="toggle__track"><span class="toggle__thumb"></span></span>
      </label>
    </div>
  </section>

  <section class="settings__section">
    <h2 class="settings__section-title">Provider 凭证</h2>
    <p class="settings__hint">
      凭证保存在 Windows 凭据管理器中（DPAPI 加密），不会写入磁盘明文。
    </p>

    {#each providers as p (p.info.id)}
      <article class="provider" data-enabled={settings?.enabled_providers.includes(p.info.id)}>
        <header class="provider__header">
          <div class="provider__info">
            <span class="provider__name">{p.info.display_name}</span>
            <span class="provider__auth">
              {p.info.auth_kind === "access_key_secret" ? "Access Key + Secret" : "Bearer API Key"}
            </span>
          </div>
          <label class="toggle">
            <input
              type="checkbox"
              checked={settings?.enabled_providers.includes(p.info.id) ?? false}
              onchange={(e) => toggleProvider(p, (e.target as HTMLInputElement).checked)}
            />
            <span class="toggle__track"><span class="toggle__thumb"></span></span>
          </label>
        </header>

        <div class="provider__body">
          {#if p.info.has_credentials && !p.dirty}
            <div class="provider__stored">
              <span class="provider__badge">已保存</span>
              <button class="btn btn--ghost" onclick={() => (p.dirty = true)}>修改</button>
              <button class="btn btn--ghost" onclick={() => clearProvider(p)}>清除</button>
            </div>
          {:else}
            {#if isAccessKey(p.info)}
              <label class="field">
                <span class="field__label">Access Key</span>
                <input
                  class="field__input"
                  type="password"
                  placeholder="AK..."
                  bind:value={p.accessKey}
                  oninput={() => (p.dirty = true)}
                />
              </label>
              <label class="field">
                <span class="field__label">Secret Key</span>
                <input
                  class="field__input"
                  type="password"
                  placeholder="SK..."
                  bind:value={p.secretKey}
                  oninput={() => (p.dirty = true)}
                />
              </label>
            {:else}
              <label class="field">
                <span class="field__label">API Key</span>
                <input
                  class="field__input"
                  type="password"
                  placeholder="sk-..."
                  bind:value={p.apiKey}
                  oninput={() => (p.dirty = true)}
                />
              </label>
            {/if}

            <div class="provider__actions">
              <button
                class="btn btn--ghost"
                disabled={p.testing || !hasCredentialInput(p)}
                onclick={() => testConnection(p)}
              >
                {p.testing ? "测试中…" : "测试连接"}
              </button>
              <button
                class="btn btn--primary"
                disabled={!hasCredentialInput(p)}
                onclick={() => saveProvider(p)}
              >
                {p.saved ? "已保存 ✓" : "保存凭证"}
              </button>
              {#if p.info.has_credentials}
                <button class="btn btn--ghost" onclick={() => clearProvider(p)}>清除</button>
              {/if}
            </div>

            {#if p.testResult}
              <div class="provider__test {testResultClass(p.testResult)}">
                {formatTestResult(p.testResult)}
              </div>
            {/if}
            {#if p.error}
              <div class="provider__error">{p.error}</div>
            {/if}
          {/if}
        </div>
      </article>
    {/each}
  </section>

  <footer class="settings__footer">
    <button class="btn btn--ghost" onclick={handleClose}>
      保存并关闭
    </button>
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
    background: var(--tum-bg-solid);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    overflow: hidden;
  }

  .settings__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--tum-space-4) var(--tum-space-5);
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
    font-size: var(--tum-font-size-lg);
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    font-family: var(--tum-font-mono);
  }

  .settings__section {
    padding: var(--tum-space-4) var(--tum-space-5);
    border-bottom: 1px solid var(--tum-border);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-3);
    /* Allow content to shrink within narrow windows. */
    min-width: 0;
  }

  .settings__section-title {
    font-size: var(--tum-font-size-sm);
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    font-weight: 600;
  }

  .settings__hint {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-secondary);
    line-height: 1.5;
  }

  .interval {
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
    /* Wrap on narrow windows so the input + hint don't get clipped. */
    flex-wrap: wrap;
  }

  .interval input {
    /* Fluid width so the field grows/shrinks with the window instead of
       overflowing when the settings window is not maximized. */
    width: 100%;
    max-width: 160px;
    min-width: 80px;
    box-sizing: border-box;
    padding: 6px 10px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-accent);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-base);
    font-variant-numeric: tabular-nums;
    -webkit-app-region: no-drag;
  }

  .interval input:focus {
    outline: none;
    border-color: var(--tum-accent-stroke);
    box-shadow: 0 0 0 2px var(--tum-accent-fill);
  }

  .interval-select {
    width: 100%;
    max-width: 260px;
    min-width: 160px;
    padding: 6px 10px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-text-primary);
    font-family: var(--tum-font);
    font-size: var(--tum-font-size-base);
    -webkit-app-region: no-drag;
    cursor: pointer;
  }

  .interval-select:focus {
    outline: none;
    border-color: var(--tum-accent-stroke);
    box-shadow: 0 0 0 2px var(--tum-accent-fill);
  }

  .interval__hint {
    font-size: var(--tum-font-size-sm);
    color: var(--tum-text-muted);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }

  /* Provider list scrolls independently of header/footer */
  .settings__section:nth-of-type(4) {
    flex: 1;
    /* 关键：flex 子元素默认 min-height:auto，会导致内容撑开父级而非滚动；
       设 0 才能让滚动区收缩到剩余高度并真正 overflow 滚动 */
    min-height: 0;
    overflow-y: auto;
    padding-bottom: var(--tum-space-4);
    scrollbar-width: thin;
    scrollbar-color: var(--tum-border) transparent;
  }

  .behavior-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--tum-space-3);
    -webkit-app-region: no-drag;
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

  .provider {
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-md);
    background: var(--tum-surface);
    overflow: hidden;
    transition: border-color 0.15s ease;
  }

  .provider + .provider {
    margin-top: var(--tum-space-3);
  }

  .provider[data-enabled="true"] {
    border-color: var(--tum-accent-stroke);
    border-left: 2px solid var(--tum-accent);
  }

  .provider__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--tum-space-3) var(--tum-space-4);
    background: rgba(15, 23, 42, 0.4);
  }

  .provider__info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .provider__name {
    font-size: var(--tum-font-size-base);
    font-weight: 600;
    color: var(--tum-text-primary);
    letter-spacing: 0.3px;
  }

  .provider__auth {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.8px;
    font-family: var(--tum-font-mono);
  }

  .provider__body {
    padding: var(--tum-space-3) var(--tum-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--tum-space-3);
  }

  .provider__stored {
    display: flex;
    align-items: center;
    gap: var(--tum-space-3);
  }

  .provider__badge {
    padding: 2px 8px;
    background: var(--tum-success-fill);
    color: var(--tum-success);
    border-radius: var(--tum-radius-xs);
    font-size: var(--tum-font-size-xs);
    font-weight: 600;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    /* Ensure the field can shrink within narrow parent and the input never gets
       clipped by sibling columns. */
    min-width: 0;
  }

  .field__label {
    font-size: var(--tum-font-size-xs);
    color: var(--tum-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.8px;
    font-family: var(--tum-font-mono);
  }

  .field__input {
    /* Fill the available width so long API keys stay visible and don't get
       squeezed by the provider card's right-side controls. */
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
    /* Larger vertical hit area; previously 8px which on 100% DPI Windows
       scaling truncated the typed text below the baseline. */
    padding: 10px 12px;
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid var(--tum-border);
    border-radius: var(--tum-radius-xs);
    color: var(--tum-text-primary);
    font-family: var(--tum-font-mono);
    font-size: var(--tum-font-size-base);
    line-height: 1.4;
    /* Let the box grow with content on very narrow viewports rather than
       cropping. */
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

  .provider__actions {
    display: flex;
    /* Wrap on narrow windows so buttons never get clipped — they flow to a
       second line instead. */
    flex-wrap: wrap;
    gap: var(--tum-space-2);
  }

  .btn {
    padding: 6px 12px;
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

  .provider__test {
    padding: 6px 10px;
    font-size: var(--tum-font-size-sm);
    border-radius: var(--tum-radius-sm);
    border-left: 3px solid var(--tum-border);
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
    word-break: break-all;
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }

  .provider__error {
    padding: 6px 10px;
    font-size: var(--tum-font-size-sm);
    color: var(--tum-danger);
    background: var(--tum-danger-fill);
    border-radius: var(--tum-radius-xs);
    font-family: var(--tum-font-mono);
    letter-spacing: 0.3px;
  }

  /* Toggle switch */
  .toggle {
    position: relative;
    display: inline-flex;
    cursor: pointer;
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
