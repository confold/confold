// Manual copy actions from the compare view: warn about (and optionally skip) files the running
// comparison already shows as identical, so a folder re-copy doesn't blindly rewrite equal bytes.
// When the comparison doesn't cover a selection, we copy directly — never wait, never guess.
import type { DiffEntry, SyncAction, SyncOp } from "./types";
import { isComparing } from "./utils/compare-helpers";

const isCopyOp = (op: SyncOp) => op === "copy_left_to_right" || op === "copy_right_to_left";

export interface IdenticalStats {
  /** Files the comparison already shows as byte-identical on both sides. */
  count: number;
  /** true when part of the selection has no comparison (pending folders) — those copy whole. */
  hasUnknown: boolean;
}

/**
 * How many identical files a copy batch would re-write. Unknown parts (pending folders in Lazy
 * mode, comparing rows) don't block the guard: their identicals just can't be counted or skipped,
 * and they stay recursive actions.
 *
 * `dirKnown` says whether a folder's children are in the tree. It matters: in lazy mode a folder's
 * raw status stays "skipped" after its level loads (the UI resolves it via `resolveDir`), so raw
 * status alone would mark preloaded folders as unknown forever.
 */
export function identicalStats(
  op: SyncOp,
  entries: DiffEntry[],
  dirKnown: (e: DiffEntry) => boolean = (e) => e.detail !== "not descended",
): IdenticalStats {
  if (!isCopyOp(op)) return { count: 0, hasUnknown: false };
  let count = 0;
  let hasUnknown = false;
  const walk = (entry: DiffEntry): number | null => {
    if (entry.is_dir) {
      if (!dirKnown(entry)) return null;
      let total = 0;
      for (const child of entry.children) {
        const n = walk(child);
        if (n === null) hasUnknown = true;
        else total += n;
      }
      return total;
    }
    if (entry.status === "skipped") return null; // comparing or filter-excluded — unknown
    return entry.status === "identical" ? 1 : 0;
  };
  for (const entry of entries) {
    const n = walk(entry);
    if (n === null) hasUnknown = true;
    else count += n;
  }
  return { count, hasUnknown };
}

/**
 * Rebuild a copy batch without the identical files. Directory copies cannot express "skip a child"
 * (the backend's dir action recurses), so each compared folder expands into per-file actions —
 * every backend's `write_file` creates missing parents — plus plain dir actions only where they
 * create genuinely empty folders. Folders without a comparison stay recursive: without data there
 * is nothing to filter, and per the manual action's meaning they copy whole.
 */
export function expandSkippingIdentical(
  op: SyncOp,
  entries: DiffEntry[],
  dirKnown: (e: DiffEntry) => boolean = (e) => e.detail !== "not descended",
): SyncAction[] {
  const out: SyncAction[] = [];
  const seen = new Set<string>();
  const push = (action: SyncAction) => {
    const k = `${action.op}:${action.rel_path.join("/")}`;
    if (seen.has(k)) return;
    seen.add(k);
    out.push(action);
  };
  for (const entry of entries) {
    // Only copy ops expand; deletes (even of identical files) always pass through untouched.
    if (isCopyOp(op) && entry.is_dir) {
      collectSkipping(op, entry, dirKnown, push);
      continue;
    }
    if (isCopyOp(op) && !entry.is_dir && entry.status === "identical") continue;
    push({ rel_path: entry.rel_path, op, is_dir: entry.is_dir });
  }
  return out;
}

/** Emits the file copies of `dir` minus identicals; returns true if any file copy was emitted. */
function collectSkipping(
  op: SyncOp,
  dir: DiffEntry,
  dirKnown: (e: DiffEntry) => boolean,
  push: (a: SyncAction) => void,
): boolean {
  // Uncompared subtree: a recursive copy is the only honest option (its files can't be filtered).
  if (!dirKnown(dir) || dir.status === "error" || isComparing(dir)) {
    push({ rel_path: dir.rel_path, op, is_dir: true });
    return true;
  }
  let needsFiles = false;
  for (const child of dir.children) {
    if (child.is_dir) {
      needsFiles = collectSkipping(op, child, dirKnown, push) || needsFiles;
    } else {
      needsFiles = true;
      if (child.status !== "identical") {
        push({ rel_path: child.rel_path, op, is_dir: false });
      }
    }
  }
  if (!needsFiles) push({ rel_path: dir.rel_path, op, is_dir: true }); // empty folder: just create it
  return needsFiles;
}
