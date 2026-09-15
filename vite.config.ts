// `vitest/config` re-exports Vite's own `defineConfig` and adds the `test` block, so the app
// build and its tests are configured from one file and cannot drift apart.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri injects TAURI_ENV_* at build time. Windows ships a Chromium-based WebView2 while
// macOS ships WebKit, so the two platforms need different transpile targets.
const isWindows = process.env["TAURI_ENV_PLATFORM"] === "windows";
const isDebugBuild = Boolean(process.env["TAURI_ENV_DEBUG"]);

export default defineConfig({
  plugins: [react()],
  // Tauri prints its own compile output below Vite's; clearing would hide Rust errors.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Rust sources are compiled by cargo, not bundled by Vite; watching them causes
    // a reload storm during `tauri dev`.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: isWindows ? "chrome105" : "safari13",
    minify: isDebugBuild ? false : "esbuild",
    sourcemap: isDebugBuild,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // Vitest globals stay off: every test imports what it uses, so the app's own type-check
    // covers the test files without a separate ambient types entry.
    globals: false,
    restoreMocks: true,
    // Class names must be the ones written in the module, otherwise a test asserting on a
    // rendered class would be asserting on a hash.
    css: { modules: { classNameStrategy: "non-scoped" } },
  },
});
