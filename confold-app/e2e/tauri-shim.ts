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
      { rel_path: ["image.png"], name: "image.png", is_dir: false, status: "different",
        left: meta("image.png", ["image.png"], 68), right: meta("image.png", ["image.png"], 68), detail: "image differs", children: [] },
      { rel_path: ["pending.dat"], name: "pending.dat", is_dir: false, status: "different",
        left: meta("pending.dat", ["pending.dat"], 32), right: meta("pending.dat", ["pending.dat"], 32), detail: "binary differs", children: [] },
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

let providerProfile = {
  kind: "ollama",
  baseUrl: "http://127.0.0.1:11434/v1",
  model: "qwen3:4b-instruct",
  useApiKey: false,
};
let verifiedModel: string | null = "qwen3:4b-instruct";
let semanticLanguage = "en";

function delayed<T>(value: T, delayMs = 1_000): Promise<T> {
  return new Promise((resolve) => {
    setTimeout(() => resolve(value), delayMs);
  });
}

const responses: Record<string, (args?: Record<string, unknown>) => unknown> = {
  source_types: () => [
    {
      id: "nuxeo",
      name: "Nuxeo",
      icon: "nuxeo",
      capabilities: { list: true, read: true, fingerprint: true, write: false },
      fields: [
        { key: "url", label: "Server URL (incl. /nuxeo context path)", kind: "text", required: true, secret: false, default: null, options: [], show_when: null },
        { key: "username", label: "Username", kind: "text", required: true, secret: false, default: null, options: [], show_when: null },
        { key: "password", label: "Password", kind: "password", required: true, secret: true, default: null, options: [], show_when: null },
        { key: "root", label: "Document path", kind: "path", required: false, secret: false, default: "/", options: [], show_when: null },
      ],
    },
  ],
  test_source: () => ({ ok: true, is_dir: true, message: "Connected" }),
  nuxeo_browse_routes: (args) => {
    const path = String(args?.path ?? "/");
    const pageIndex = Number(args?.pageIndex ?? 0);
    const entries = path === "/"
      ? [
          { name: "default-domain", title: "Default domain", path: "/default-domain" },
          { name: "system", title: "System", path: "/system" },
        ]
      : path === "/default-domain"
        ? [{ name: "workspaces", title: "Workspaces", path: "/default-domain/workspaces" }]
        : pageIndex === 1
          ? [{ name: "archive", title: "Archive", path: `${path}/archive` }]
          : [{ name: "contracts", title: "Contracts", path: `${path}/contracts` }];
    return { entries, has_more: path === "/default-domain/workspaces" && pageIndex === 0 };
  },
  nuxeo_search_routes: (args) => ({
    entries: [{ name: "signed-contracts", title: "Signed contracts", path: "/default-domain/workspaces/signed-contracts" }],
    has_more: false,
  }),
  load_recents: () => ({ origins: [], destinations: [] }),
  save_recents: () => undefined,
  shell_integration_status: () => ({ supported: true, installed: false }),
  load_provider_settings: () => ({
    profile: providerProfile,
    hasApiKey: false,
    hasCustomHeaders: false,
    verifiedModel,
  }),
  load_semantic_preferences: () => ({ language: semanticLanguage }),
  save_semantic_preferences: (args) => {
    const preferences = args?.preferences;
    semanticLanguage =
      typeof preferences === "object" &&
      preferences !== null &&
      "language" in preferences &&
      typeof preferences.language === "string"
        ? preferences.language
        : "en";
    return { language: semanticLanguage };
  },
  save_provider_settings: (args) => {
    providerProfile = {
      ...providerProfile,
      ...((args?.request as { profile?: typeof providerProfile })?.profile ?? {}),
    };
    verifiedModel = null;
    return {
      profile: providerProfile,
      hasApiKey: false,
      hasCustomHeaders: false,
      verifiedModel,
    };
  },
  list_provider_models: () => [
    "qwen3:4b-instruct",
    "qwen2.5-coder:7b",
    "deepseek-coder:6.7b",
    "llava:7b",
    "llama3.2:3b",
  ],
  test_provider_capability: () => {
    if (providerProfile.model === "reject-structured") {
      return new Promise((_, reject) => {
        setTimeout(
          () => reject(new Error("Provider did not honor the required JSON schema.")),
          1_000,
        );
      });
    }
    verifiedModel = providerProfile.model || "qwen3:8b";
    return delayed({ model: verifiedModel });
  },
  semantic_review: () => delayed({
    kind: "review",
    model: verifiedModel || "qwen3:8b",
    configuration: {
      provider: providerProfile.kind,
      endpoint: providerProfile.baseUrl,
      requestedModel: providerProfile.model,
      responseModel: verifiedModel || "qwen3:8b",
      language: semanticLanguage,
    },
    origin_path: "/tmp/cdemo/left/readme.md",
    destination_path: "/tmp/cdemo/right/readme.md",
    proposal: {
      schema_version: 1,
      operation_id: "visual-operation",
      verdict: "merged",
      summary: "Both documents contribute useful, non-conflicting details.",
      contributions: [
        { source: "left", intent: "Preserve the concise introduction.", disposition: "preserved" },
        { source: "right", intent: "Preserve the expanded example.", disposition: "preserved" },
      ],
      warnings: [],
      result: "# Title\n\nConcise introduction.\n\nExpanded example.\n",
    },
    report: {
      schema_version: 1,
      operation_id: "visual-operation",
      verdict: "merged",
      applicable: true,
      summary: "Both documents contribute useful, non-conflicting details.",
      contributions: [
        { source: "left", intent: "Preserve the concise introduction.", disposition: "preserved" },
        { source: "right", intent: "Preserve the expanded example.", disposition: "preserved" },
      ],
      warnings: [],
      result_sha256: "fixture",
      diffs: [
        { source: "left", unified_diff: "--- left\n+++ proposal\n@@\n+Expanded example.\n" },
        { source: "right", unified_diff: "--- right\n+++ proposal\n@@\n+Concise introduction.\n" },
      ],
    },
  }, 3_000),
  semantic_apply_to: (args) => ({
    schema_version: 1,
    operation_id: "visual-operation",
    output:
      args?.target === "origin"
        ? "/tmp/cdemo/left/readme.md"
        : "/tmp/cdemo/right/readme.md",
    sha256: "saved",
    byte_len: 51,
  }),
  semantic_apply_new: (args) => ({
    schema_version: 1,
    operation_id: "visual-operation",
    output: String(args?.output ?? "/tmp/cdemo/left/readme-merged.md"),
    sha256: "saved",
    byte_len: 51,
  }),
  compare: () => REPORT,
  compare_level: () => REPORT,
  diff_file: (args) => {
    const rel = String((args?.left as { rel?: string })?.rel ?? "");
    if (rel.endsWith("pending.dat")) return new Promise(() => {});
    if (rel.endsWith(".bin")) return { result: { kind: "binary", identical: false }, left: meta2, right: meta2 };
    if (rel.endsWith(".png")) return { result: { kind: "binary", identical: false }, left: meta2, right: meta2 };
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

export function save(): Promise<string | null> {
  return Promise.resolve("/tmp/cdemo/left/readme-merged.md");
}

export type UnlistenFn = () => void;
