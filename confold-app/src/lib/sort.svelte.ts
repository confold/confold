// Reactive sort key for the tree, shared across the (recursive) tree and the page control.
import type { DiffEntry } from "./types";

export type SortKey = "name" | "status" | "size" | "mtime";

/** Reactive holder (Svelte 5 module-scope rune). Bind `sortState.key` from the UI. */
export const sortState = $state<{ key: SortKey }>({ key: "name" });

// Status sort weight: surface divergences first, identical/skipped last.
const STATUS_ORDER: Record<string, number> = {
  different: 0,
  left_only: 1,
  right_only: 2,
  error: 3,
  skipped: 4,
  identical: 5,
};

const maxSize = (e: DiffEntry) => Math.max(e.left?.size ?? 0, e.right?.size ?? 0);
const maxMtime = (e: DiffEntry) => Math.max(e.left?.mtime ?? 0, e.right?.mtime ?? 0);

/**
 * Return a sorted copy of `entries`: directories first, then by the active key. `statusOf` supplies
 * the *effective* status for the status sort — a lazy directory's resolved status (its raw `status`
 * stays `skipped` until its subtree is known), so resolved divergences sort to the top as verdicts
 * stream in. Defaults to the raw status.
 */
export function sortEntries(
  entries: DiffEntry[],
  statusOf: (e: DiffEntry) => string = (e) => e.status,
): DiffEntry[] {
  const k = sortState.key;
  const out = [...entries];
  out.sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1; // dirs first
    switch (k) {
      case "status":
        return (STATUS_ORDER[statusOf(a)] - STATUS_ORDER[statusOf(b)]) || a.name.localeCompare(b.name);
      case "size":
        return maxSize(b) - maxSize(a) || a.name.localeCompare(b.name);
      case "mtime":
        return maxMtime(b) - maxMtime(a) || a.name.localeCompare(b.name);
      default:
        return a.name.localeCompare(b.name);
    }
  });
  return out;
}
