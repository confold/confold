// Browser-side stand-in for the Tauri runtime, used ONLY when VITE_E2E=1 (wired via vite.config.js
// resolve.alias). It lets the real frontend run in a plain Chromium for Playwright visual smokes:
// `invoke` returns canned data, and events/dialog/window are no-ops. This is NOT a backend — just
// enough to render and navigate the UI. Keep responses minimal; jsdom component tests cover logic.
import type { DiffReport } from "../src/lib/types";

const meta = (name: string, rel: string[], size = 10) => ({
  name,
  rel_path: rel,
  kind: "file" as const,
  size,
  mtime: 1_700_000_000_000,
  created: null,
});

// A small mixed tree: one of each status so the tree, legend, and filters all have something to show.
const REPORT: DiffReport = {
  summary: { identical: 1, different: 1, left_only: 1, right_only: 1, skipped: 0, errored: 0 },
  root: {
    rel_path: [],
    name: "",
    is_dir: true,
    status: "different",
    left: null,
    right: null,
    detail: null,
    children: [
      { rel_path: ["readme.md"], name: "readme.md", is_dir: false, status: "different",
        left: meta("readme.md", ["readme.md"]), right: meta("readme.md", ["readme.md"], 20),
        detail: "size differs", children: [] },
      { rel_path: ["same.txt"], name: "same.txt", is_dir: false, status: "identical",
        left: meta("same.txt", ["same.txt"]), right: meta("same.txt", ["same.txt"]), detail: null, children: [] },
      { rel_path: ["only_left.txt"], name: "only_left.txt", is_dir: false, status: "left_only",
        left: meta("only_left.txt", ["only_left.txt"]), right: null, detail: null, children: [] },
      { rel_path: ["only_right.txt"], name: "only_right.txt", is_dir: false, status: "right_only",
        left: null, right: meta("only_right.txt", ["only_right.txt"]), detail: null, children: [] },
      { rel_path: ["blob.bin"], name: "blob.bin", is_dir: false, status: "different",
        left: meta("blob.bin", ["blob.bin"], 32), right: meta("blob.bin", ["blob.bin"], 32), detail: "binary differs", children: [] },
      { rel_path: ["big.log"], name: "big.log", is_dir: false, status: "different",
        left: meta("big.log", ["big.log"], 3_400_000), right: meta("big.log", ["big.log"], 3_500_000), detail: "size differs", children: [] },
    ],
  },
};

// A large-text-file hunks fixture (for the diff_file_large path).
const drow = (n: number, kind: string, l: string, r: string) => ({
  left_no: n, right_no: n, kind, left: l, right: r,
  left_words: [], right_words: [], left_words_w: [], right_words_w: [],
});
// 100 hunks with realistic word ranges on the replace rows — mirrors noisy.log's shape (the volume the
// real "black window" report hit), to check the frontend renders it rather than blanking.
const rep = (n: number, l: string, r: string) => ({
  left_no: n, right_no: n, kind: "replace", left: l, right: r,
  left_words: [{ start: 0, end: 3 }], right_words: [{ start: 0, end: 3 }],
  left_words_w: [{ start: 0, end: 3 }], right_words_w: [{ start: 0, end: 3 }],
});
const HUNKS = {
  hunks: Array.from({ length: 100 }, (_, k) => {
    const base = 1 + k * 500;
    return {
      left_start: base,
      right_start: base,
      rows: [
        drow(base, "equal", `ctx ${base}`, `ctx ${base}`),
        rep(base + 1, `old ${base}`, `new ${base}`),
        drow(base + 2, "equal", `ctx ${base + 2}`, `ctx ${base + 2}`),
      ],
    };
  }),
  summary: { equal: 200, inserted: 0, deleted: 0, replaced: 100 },
  is_complete: false,
  total_hunks: 0,
  next_hunk_index: 100,
};

// 48 bytes = 3 rows of 16. One isolated differing byte in EACH row (offsets 2, 18, 34) → every row
// differs, but the bytes are non-contiguous → 3 distinct difference regions. (Guards the regression
// where per-row grouping collapsed an all-rows-differ file to a single region.)
const LEFT_BYTES = Array.from({ length: 48 }, (_, i) => i % 256);
const RIGHT_BYTES = LEFT_BYTES.map((b, i) => (i === 2 || i === 18 || i === 34 ? b ^ 0xff : b));

const meta2 = { fp: "", eol: "\n", final_nl: false, mtime: null, created: null };

const responses: Record<string, (args?: Record<string, unknown>) => unknown> = {
  source_types: () => [],
  compare: () => REPORT,
  compare_level: () => REPORT,
  diff_file: (args) => {
    const rel = String((args?.left as { rel?: string })?.rel ?? "");
    if (rel.endsWith(".bin")) return { result: { kind: "binary", identical: false }, left: meta2, right: meta2 };
    if (rel.endsWith(".log")) return { result: { kind: "too_large", left_size: 3_400_000, right_size: 3_500_000 }, left: meta2, right: meta2 };
    if (rel.endsWith(".md")) {
      const diff = {
        rows: [
          drow(1, "equal", "# Title", "# Title"),
          rep(2, "left version", "right version, longer"),
          drow(3, "equal", "shared line", "shared line"),
        ],
        summary: { equal: 2, inserted: 0, deleted: 0, replaced: 1 },
      };
      return { result: { kind: "text", diff }, left: meta2, right: meta2 };
    }
    return new Promise(() => {}); // other: stay in the loading skeleton (visual smoke only)
  },
  diff_file_large: () => ({ kind: "text_hunks", hunks: HUNKS, left: meta2, right: meta2, left_size: 3_400_000, right_size: 3_500_000 }),
  hex_compare: () => ({
    left: LEFT_BYTES,
    right: RIGHT_BYTES,
    left_len: LEFT_BYTES.length,
    right_len: RIGHT_BYTES.length,
    truncated: false,
  }),
};

export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const fn = responses[cmd];
  return Promise.resolve((fn ? fn(args) : undefined) as T);
}

export function listen(): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function getCurrentWindow() {
  return { close() {} };
}

export function open(): Promise<string | null> {
  return Promise.resolve(null);
}

export type UnlistenFn = () => void;
