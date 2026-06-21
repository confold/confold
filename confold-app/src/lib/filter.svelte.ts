// Reactive show/hide-by-status filter, shared across the (recursive) tree and the page filter bar.
import { SvelteSet } from "svelte/reactivity";
import type { DiffEntry, DiffStatus } from "./types";

/** Statuses currently HIDDEN. Empty = show everything. */
export const hidden = new SvelteSet<DiffStatus>();

export function toggleStatus(s: DiffStatus): void {
  if (hidden.has(s)) hidden.delete(s);
  else hidden.add(s);
}

/**
 * A node is visible if its *effective* status is not hidden, OR it is a directory that still
 * contains a visible descendant (so containers of differences survive a "hide identical"-style
 * filter). `statusOf` resolves the effective status: for a lazy/pending directory this is the
 * status computed from its loaded subtree (its raw `status` is just `skipped` until then), so a
 * folder whose contents are all identical disappears when "identical" is hidden.
 */
export function nodeVisible(
  e: DiffEntry,
  statusOf: (e: DiffEntry) => DiffStatus = (x) => x.status,
): boolean {
  if (!hidden.has(statusOf(e))) return true;
  return e.is_dir && e.children.some((c) => nodeVisible(c, statusOf));
}
