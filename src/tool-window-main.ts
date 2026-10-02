import { installDevMock } from "./lib/dev-mock";
installDevMock();

import { mount } from "svelte";
import "./styles/tokens.css";
import ToolWindow from "./lib/components/ToolWindow.svelte";

const app = mount(ToolWindow, {
  target: document.getElementById("app")!,
});

export default app;