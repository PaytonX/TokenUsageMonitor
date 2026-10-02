import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // 只覆盖 lib 下的纯逻辑；*.svelte 组件测试不在本计划范围
    include: ["src/lib/**/*.test.ts"],
    environment: "node",
  },
});
