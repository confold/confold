// Typed bindings for every Tauri command — the single source of truth for the frontend↔backend
// contract. Each function maps 1:1 to a `#[tauri::command]` in `src-tauri/src/lib.rs`. Argument keys
// are camelCase here; Tauri converts them to the Rust snake_case parameter names. Calling these instead
// of raw `invoke("name", { ... })` makes a wrong argument name or type a COMPILE error rather than a
// runtime "command rejected" (which is how the `delete_origin`/`start_hunk` casing bugs slipped through).
//
// When you add or rename a command — or any of its arguments — update THIS file and the Rust signature
// together. Tests mock `@tauri-apps/api/core`'s `invoke`, so they still intercept these calls.
import { invoke } from "@tauri-apps/api/core";
import type {
  CompareOpts,
  DiffReport,
  SourceSpec,
  FileRef,
  DiffFileResult,
  LargeDiffResult,
  ActionOutcome,
  SyncAction,
  FileDiff,
  SaveResult,
  HexCompare,
  SourceTypeInfo,
} from "$lib/types";
import type { MigrateAction } from "$lib/migrate";

/** Which difference categories a Migrate run acts on (the config-panel toggles). */
export type MigrateFlags = {
  copy_new: boolean;
  overwrite_different: boolean;
  delete_extra: boolean;
  delete_origin: boolean;
};

/** Which side(s) a Sync run trusts, plus deletion + conflict handling. */
export type SyncFlags = {
  trust_left: boolean;
  trust_right: boolean;
  delete_diffs: boolean;
  conflict_rule: "newer" | "larger" | "manual";
};

/** Result of probing a source (the picker's ✓/✗ + dir-vs-file inference). */
export type TestResult = { ok: boolean; is_dir: boolean; message: string };

export const commands = {
  /** Full/level comparison of two sources (streams verdicts via `entry-resolved` events). */
  compare: (left: SourceSpec, right: SourceSpec, opts: CompareOpts, token: number) =>
    invoke<DiffReport>("compare", { left, right, opts, token }),

  /** Compare a single directory level (lazy expand). */
  compareLevel: (left: SourceSpec, right: SourceSpec, opts: CompareOpts, rel: string, token: number) =>
    invoke<DiffReport>("compare_level", { left, right, opts, rel, token }),

  /** Start a Migrate (one-directional) plan computation on a background thread. Returns immediately;
   *  progress arrives via `plan-progress` and the action list via `plan-ready` (tagged with `token`). */
  migrateActions: (left: SourceSpec, right: SourceSpec, opts: CompareOpts, flags: MigrateFlags, token: number) =>
    invoke<void>("migrate_actions", { left, right, opts, flags, token }),

  /** Start a Sync (bidirectional) plan computation on a background thread (see `migrateActions`). */
  syncActions: (left: SourceSpec, right: SourceSpec, opts: CompareOpts, flags: SyncFlags, token: number) =>
    invoke<void>("sync_actions", { left, right, opts, flags, token }),

  /** Apply a Migrate/Sync plan on a background thread (progress via `migrate-progress`/`migrate-phase`,
   *  completion via `migrate-done`). `deleteOrigin` enables the M2 move (Migrate only). */
  migrateApply: (args: {
    left: SourceSpec;
    right: SourceSpec;
    actions: MigrateAction[];
    generation: number;
    deleteOrigin: boolean;
    opts: CompareOpts;
  }) => invoke<void>("migrate_apply", args),

  /** Cancel the migrate/sync apply in progress. */
  migrateCancel: () => invoke<void>("migrate_cancel"),

  /** Dry-run a batch of manual sync actions (Compare-mode copy/delete). */
  planActions: (left: SourceSpec, right: SourceSpec, actions: SyncAction[]) =>
    invoke<ActionOutcome[]>("plan_actions", { left, right, actions }),

  /** Apply a batch of manual sync actions. */
  applyActions: (left: SourceSpec, right: SourceSpec, actions: SyncAction[]) =>
    invoke<ActionOutcome[]>("apply_actions", { left, right, actions }),

  /** Diff a single file pair (text/binary/too-large). */
  diffFile: (left: FileRef, right: FileRef) => invoke<DiffFileResult>("diff_file", { left, right }),

  /** Large-file diff: hunks-only (paginated via `startHunk`) or hex, bounded by the optional caps. */
  diffFileLarge: (
    left: FileRef,
    right: FileRef,
    opts?: { maxBytes?: number; maxHunks?: number; contextLines?: number; startHunk?: number },
  ) => invoke<LargeDiffResult>("diff_file_large", { left, right, ...opts }),

  /** Diff two in-memory strings (used by the side-by-side merge preview). */
  diffStrings: (left: string, right: string) => invoke<FileDiff>("diff_strings", { left, right }),

  /** Save a file with on-disk conflict detection (`expect` = the fingerprint the UI last read). */
  saveFile: (file: FileRef, contents: string, expect: string | null | undefined, force: boolean) =>
    invoke<SaveResult>("save_file", { file, contents, expect, force }),

  /** Positional hex byte-compare of two (possibly binary) files. */
  hexCompare: (left: FileRef, right: FileRef) => invoke<HexCompare>("hex_compare", { left, right }),

  /** Read a file's bytes (for the image preview). */
  readBytes: (file: FileRef) => invoke<ArrayBuffer>("read_bytes", { file }),

  /** The catalog of source types + their config-form field specs. */
  sourceTypes: () => invoke<SourceTypeInfo[]>("source_types"),

  /** Probe a source config: reachable? directory or single file? */
  testSource: (spec: SourceSpec) => invoke<TestResult>("test_source", { spec }),

  installShellIntegration: () => invoke<void>("install_shell_integration"),
  uninstallShellIntegration: () => invoke<void>("uninstall_shell_integration"),
  shellIntegrationStatus: () => invoke<{ installed: boolean }>("shell_integration_status"),

  loadRecents: () => invoke<{
    origins: { spec: SourceSpec; isDir: boolean; stale: boolean }[];
    destinations: { spec: SourceSpec; isDir: boolean; stale: boolean }[];
  }>("load_recents"),
  saveRecents: (
    origins: { spec: SourceSpec; isDir: boolean }[],
    destinations: { spec: SourceSpec; isDir: boolean }[],
  ) => invoke<void>("save_recents", { origins, destinations }),
  pathExists: (path: string) => invoke<boolean>("path_exists", { path }),
};
