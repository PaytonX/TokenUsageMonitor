import { installDevMock } from "./lib/dev-mock";

// 仅开发期 + 仅当页面里没有 Tauri 运行时时生效（见 dev-mock.ts 顶部说明）。
// 生产构建被 tree-shake 掉；`tauri dev` 的原生窗口下自动跳过。
installDevMock();

import { initLocaleWithBackend } from "./lib/i18n/store";

initLocaleWithBackend();

import { mount } from "svelte";
import "./styles/tokens.css";
import App from "./App.svelte";

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
