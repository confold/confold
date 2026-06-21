// Setup for the `component` vitest project (jsdom).
//
// Node 26 exposes an experimental `localStorage` global that is `undefined` unless `--localstorage-file`
// is given, and it *shadows* jsdom's `window.localStorage`. App code uses the bare `localStorage`
// identifier (e.g. the page's recents in onMount), so we pin a single working store onto both
// `globalThis` and `window` — jsdom's if present, else a tiny in-memory polyfill.

function makeStorage(): Storage {
  const m = new Map<string, string>();
  return {
    getItem: (k) => (m.has(k) ? (m.get(k) as string) : null),
    setItem: (k, v) => void m.set(k, String(v)),
    removeItem: (k) => void m.delete(k),
    clear: () => m.clear(),
    key: (i) => [...m.keys()][i] ?? null,
    get length() {
      return m.size;
    },
  } as Storage;
}

const store: Storage =
  (typeof window !== "undefined" && window.localStorage) || makeStorage();

for (const target of [globalThis, typeof window !== "undefined" ? window : undefined]) {
  if (target) {
    Object.defineProperty(target, "localStorage", {
      value: store,
      configurable: true,
      writable: true,
    });
  }
}

// jsdom doesn't implement ResizeObserver (used for the virtualized list/tree sizing). A no-op is fine —
// tests don't assert on measured layout (the virtualizer's overscan renders the small fixtures anyway).
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}
