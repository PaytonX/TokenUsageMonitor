import { mount } from "svelte";
import "./styles/tokens.css";
import Settings from "./Settings.svelte";

const app = mount(Settings, {
  target: document.getElementById("app")!,
});

export default app;
