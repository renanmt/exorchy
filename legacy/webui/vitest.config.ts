import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  // Without this, `solid-js` resolves to its SERVER build under vitest: signals
  // still read, but createEffect never runs, so any test that asserts on
  // reactivity silently tests nothing.
  resolve: { conditions: ["development", "browser"] },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["src/test/setup.ts"],
    exclude: ["src-tauri/**", "node_modules/**"],
  },
});
