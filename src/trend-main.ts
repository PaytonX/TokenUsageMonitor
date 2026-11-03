import { installDevMock } from "./lib/dev-mock";
installDevMock();

import { initLocaleWithBackend } from "./lib/i18n/store";

initLocaleWithBackend();

// 平台标识进 <html data-platform>：浅色令牌与 macOS 标题栏避让的作用域
// （src/lib/platform.ts）。必须在 mount 前执行，保证首帧前生效。
import { applyPlatformAttributes } from "./lib/platform";

applyPlatformAttributes();

import { mount } from "svelte";
import "./styles/tokens.css";
import TrendWindow from "./lib/components/TrendWindow.svelte";

const app = mount(TrendWindow, {
  target: document.getElementById("app")!,
});

export default app;