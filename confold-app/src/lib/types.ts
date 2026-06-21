// Mirrors the `DiffReport` JSON returned by the Rust `compare` command (confold-core).

export type DiffStatus =
  | "identical"
  | "different"
  | "left_only"
  | "right_only"
  | "skipped"
  | "error";

export interface EntryMeta {
  name: string;
  rel_path: string[];
  kind: "file" | "dir" | "symlink";
  size: number;
  mtime: number | null; // Unix epoch milliseconds
  created: number | null; // Unix epoch ms; null when the OS/backend doesn't expose it
}

export interface DiffEntry {
  rel_path: string[];
  name: string;
  is_dir: boolean;
  status: DiffStatus;
  left: EntryMeta | null;
  right: EntryMeta | null;
  detail: string | null;
  children: DiffEntry[];
}

export interface Summary {
  identical: number;
  different: number;
  left_only: number;
  right_only: number;
  skipped: number;
  errored: number;
}

export interface DiffReport {
  root: DiffEntry;
  summary: Summary;
}

export interface CompareOpts {
  method: "quick" | "full" | "size" | "mtime" | "size-mtime";
  include: string[];
  exclude: string[];
}

export type SyncOp =
  | "copy_left_to_right"
  | "copy_right_to_left"
  | "delete_left"
  | "delete_right";

export interface SyncAction {
  rel_path: string[];
  op: SyncOp;
  is_dir: boolean;
}

export interface ActionOutcome {
  rel_path: string[];
  op: SyncOp;
  ok: boolean;
  error: string | null;
}

// ---- Side-by-side file diff (Phase 4) ----
export type RowKind = "equal" | "insert" | "delete" | "replace";

export interface WordRange {
  start: number;
  end: number;
}

export interface DiffRow {
  left_no: number | null;
  right_no: number | null;
  kind: RowKind;
  left: string | null;
  right: string | null;
  /** Changed ranges at character granularity (for `replace`). */
  left_words: WordRange[];
  right_words: WordRange[];
  /** Changed ranges at word granularity — whole changed words (for `replace`). */
  left_words_w: WordRange[];
  right_words_w: WordRange[];
}

export interface DiffSummary {
  equal: number;
  inserted: number;
  deleted: number;
  replaced: number;
}

export interface FileDiff {
  rows: DiffRow[];
  summary: DiffSummary;
}

export type DiffResult =
  | { kind: "text"; diff: FileDiff }
  | { kind: "binary"; identical: boolean }
  | { kind: "too_large"; left_size: number; right_size: number }; // → open via the large-file flow

/** One hunk: context + changed rows, with the 1-based start line on each side. */
export interface DiffHunk {
  rows: DiffRow[];
  left_start: number;
  right_start: number;
}

/** Paginated hunks-only view for large text files. */
export interface FileDiffHunks {
  hunks: DiffHunk[];
  summary: DiffSummary;
  /** True when we reached EOF within the caps — all changes are represented. */
  is_complete: boolean;
  /** Total hunk count (only valid when is_complete=true; 0 otherwise). */
  total_hunks: number;
  /** Hunk index to pass as start_hunk in the next Load-more call; null when all hunks are loaded. */
  next_hunk_index: number | null;
}

/** Response from the `diff_file_large` command: a text-hunks view or a binary verdict. */
export type LargeDiffResult =
  | { kind: "text_hunks"; hunks: FileDiffHunks; left: FileMeta; right: FileMeta; left_size: number; right_size: number }
  | { kind: "binary"; identical: boolean; left: FileMeta; right: FileMeta; left_size: number; right_size: number };

/** Per-side metadata: content fingerprint (conflict detection) + EOL/final-newline (faithful saves). */
export interface FileMeta {
  fp: string;
  eol: string;
  final_nl: boolean;
  mtime: number | null; // epoch ms; for the side-by-side date header
  created: number | null;
}

/** Capabilities a source declares (mirrors `confold_vfs::Capabilities`); gates which actions the UI offers. */
export interface Capabilities {
  list: boolean;
  read: boolean;
  fingerprint: boolean;
  write: boolean;
}

/** A data source chosen in the UI: a registered kind id + flat config values. Generic — never changes as
 *  backends are added (each backend's form/identity comes from `source_types()`). Secrets live in
 *  `fields` and are never persisted. */
export type SourceSpec = { kind: string; fields: Record<string, string> };

/** One configurable field of a source type (from `source_types()`); the picker renders a form from these. */
export interface FieldSpec {
  /** Config key; dotted for nested fields (e.g. `auth.password`). */
  key: string;
  label: string;
  kind: "text" | "number" | "password" | "path" | "textarea" | "select";
  required: boolean;
  /** Secret material — the UI masks it and never persists it. */
  secret: boolean;
  default: string | null;
  options: string[];
  /** Conditional display `"key=value"` — show only when another field has this value. */
  show_when: string | null;
}

/** A source type the picker can offer (from the `source_types()` command). */
export interface SourceTypeInfo {
  id: string;
  name: string;
  /** Icon for the picker (emoji today) — sourced from the backend so new kinds need no frontend switch. */
  icon: string;
  capabilities: Capabilities;
  fields: FieldSpec[];
}

/** A single file on a source: the source + the relative path within it (the unit the file commands take). */
export interface FileRef {
  source: SourceSpec;
  rel: string;
}

/** `diff_file` payload: the diff + per-side metadata. */
export interface DiffFileResult {
  result: DiffResult;
  left: FileMeta;
  right: FileMeta;
}

/** Outcome of `save_file`: written (new fingerprint) or refused because the file changed on disk. */
export type SaveResult = { kind: "saved"; fp: string } | { kind: "conflict" };

/** `hex_compare` payload: each side's bytes (capped) + the true lengths + whether it was truncated. */
export interface HexCompare {
  left: number[];
  right: number[];
  left_len: number;
  right_len: number;
  truncated: boolean;
}
