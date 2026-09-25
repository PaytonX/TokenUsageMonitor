import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  // Force runes mode for every component. Without this, Svelte's per-file
  // auto-detection can mis-classify prop-less root components (e.g. the trend
  // window) that happen to name a variable `state`, treating `$state` as a
  // legacy store subscription and failing type-checking.
  compilerOptions: { runes: true },
  preprocess: vitePreprocess(),
};
