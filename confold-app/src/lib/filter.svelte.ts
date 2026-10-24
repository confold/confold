// Reactive show/hide-by-status filter, shared across the (recursive) tree and the page filter bar.
import { SvelteSet } from "svelte/reactivity";
import type { DiffEntry, DiffStatus } from "./types";

/** True when this is a directory present on both sides whose subtree hasn't been descended into yet. */
export function isPending(e: DiffEntry): boolean {
  return e.is_dir && e.status === "skipped" && e.detail === "not descended";
}

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
 *
 * An unresolved pending directory (lazy mode, subtree not yet known) returns `null` from `statusOf`
 * and must always stay visible: it represents an uninspected folder that may contain differences,
 * not a skipped/filtered file.
 */
export function nodeVisible(
  e: DiffEntry,
  statusOf: (e: DiffEntry) => DiffStatus | null = (x) => (isPending(x) ? null : x.status),
): boolean {
  const s = statusOf(e);
  if (s === null) return true;
  if (isPending(e) && s === "skipped" && e.children.length === 0) return true;
  if (!hidden.has(s)) return true;
  return e.is_dir && e.children.some((c) => nodeVisible(c, statusOf));
}
