import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "path";

// Tauri expects a fixed port and exposes env vars for the frontend
const port = process.env.TAURI_DEV_PORT
  ? Number(process.env.TAURI_DEV_PORT)
  : 5173;

// Multi-page build: dashboard (index.html) + settings window (settings.html)
// + standalone full-window trend view (trend.html) + local-tools window
// (toolwindow.html) + the compact pill's edge handle (peek.html). All share
// src/lib/ modules; only the entry module differs.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
    rollupOptions: {
      input: {
        dashboard: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
        trend: resolve(__dirname, "trend.html"),
        tools: resolve(__dirname, "toolwindow.html"),
        peek: resolve(__dirname, "peek.html"),
      },
    },
  },
});
