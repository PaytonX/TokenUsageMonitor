import { installDevMock } from "./lib/dev-mock";
installDevMock();

import { initLocaleWithBackend } from "./lib/i18n/store";

initLocaleWithBackend();

import { mount } from "svelte";
import "./styles/tokens.css";
import TrendWindow from "./lib/components/TrendWindow.svelte";

const app = mount(TrendWindow, {
  target: document.getElementById("app")!,
});

export default app;