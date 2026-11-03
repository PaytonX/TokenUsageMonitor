import { installDevMock } from "./lib/dev-mock";

// 仅开发期 + 仅当页面里没有 Tauri 运行时时生效（见 dev-mock.ts 顶部说明）。
// 生产构建被 tree-shake 掉；`tauri dev` 的原生窗口下自动跳过。
installDevMock();

import { initLocaleWithBackend } from "./lib/i18n/store";

initLocaleWithBackend();

// 平台标识进 <html data-platform>：浅色令牌与 macOS 标题栏避让的作用域
// （src/lib/platform.ts）。必须在 mount 前执行，保证首帧前生效。
import { applyPlatformAttributes } from "./lib/platform";

applyPlatformAttributes();

import { mount } from "svelte";
import "./styles/tokens.css";
import App from "./App.svelte";

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
