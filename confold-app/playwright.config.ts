import { defineConfig, devices } from "@playwright/test";

// Playwright visual smokes — a SEPARATE layer from the always-on vitest net (run via `pnpm e2e`, never
// `pnpm test`). The webServer starts the Vite dev server with VITE_E2E=1 so the Tauri API modules are
// aliased to the browser shim (see vite.config.js + e2e/tauri-shim.ts). Run targeted with
// `pnpm e2e <file>` or `pnpm e2e --grep "<title>"`; screenshots/traces land under test-results on failure.
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  fullyParallel: true,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:1430",
    // Headless on purpose: bundled Chromium with no window → it can't hijack link-opening, and Playwright
    // auto-closes it at the end. (The leak we hit is from the Playwright *MCP*, which runs your real
    // Chrome with an isolated profile and leaves it open — use this CLI instead, not the MCP.)
    headless: true,
    screenshot: "only-on-failure",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "pnpm dev",
    url: "http://127.0.0.1:1430",
    env: { VITE_E2E: "1" },
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
