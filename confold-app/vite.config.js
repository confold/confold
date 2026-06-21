import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
// @ts-expect-error node builtin (no @types/node in this config's typecheck scope)
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// @ts-expect-error process is a nodejs global
const e2e = !!process.env.VITE_E2E;

// In E2E mode (Playwright visual smokes) the app runs in a plain browser with no Tauri runtime, so
// alias the Tauri API modules to a browser-side shim (canned data + no-op events). Off by default.
/** @type {string} */
const shim = fileURLToPath(new URL("./e2e/tauri-shim.ts", import.meta.url));
/** @type {Record<string, string>} */
const e2eAlias = e2e
  ? {
      "@tauri-apps/api/core": shim,
      "@tauri-apps/api/event": shim,
      "@tauri-apps/api/window": shim,
      "@tauri-apps/plugin-dialog": shim,
    }
  : {};

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit()],
  resolve: { alias: e2eAlias },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    // E2E runs on its own port so it never clashes with a running `tauri dev` / `pnpm dev` on 1420.
    port: e2e ? 1430 : 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
