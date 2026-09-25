import { mount } from "svelte";
import "./styles/tokens.css";
import TrendWindow from "./lib/components/TrendWindow.svelte";

const app = mount(TrendWindow, {
  target: document.getElementById("app")!,
});

export default app;