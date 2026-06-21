import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { fileURLToPath } from "node:url";

const alias = { $lib: fileURLToPath(new URL("./src/lib", import.meta.url)) };

// Two projects, cleanly separated so the fast pure-logic tests don't pay for a DOM:
//  - `unit`      → node env, the pure `.test.ts` (lib/ rune stores + domain logic). No DOM.
//  - `component` → jsdom env, `*.svelte.test.ts` rendering real components via @testing-library/svelte.
//    `svelteTesting()` wires the browser resolve-conditions + afterEach cleanup for this project only.
export default defineConfig({
  test: {
    projects: [
      {
        plugins: [svelte()],
        resolve: { alias },
        test: {
          name: "unit",
          include: ["src/**/*.test.ts"],
          exclude: ["src/**/*.svelte.test.ts"],
          environment: "node",
        },
      },
      {
        plugins: [svelte(), svelteTesting()],
        resolve: { alias },
        test: {
          name: "component",
          include: ["src/**/*.svelte.test.ts"],
          environment: "jsdom",
          // A real origin so `window.localStorage` works (the page reads/writes recents in onMount).
          environmentOptions: { jsdom: { url: "http://localhost/" } },
          setupFiles: ["./src/test-setup.component.ts"],
        },
      },
    ],
  },
});
