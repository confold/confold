// Incremental re-compute (Phase-5 §7): reuse the data we already hold when a control changes, instead
// of re-fetching the whole tree. This module holds the PURE verdict/filter logic (tested in
// recompute.test.ts); the UI wiring (walking the loaded tree, deciding fetch-vs-recompute) lives in
// +page.svelte.
//
// Bucket 1a — metadata-only methods (`size` / `mtime` / `size-mtime`): the verdict is a pure function of
// the size/mtime we already have for every loaded entry. Mirrors the backend (confold-core).
//
// Bucket 1b — adding an exclude: entries now matching the new glob are marked filtered in the client,
// no backend round-trip. Removing a glob requires a backend re-scan (those entries were never compared,
// so we have no verdict for them — Bucket 2).
import type { DiffEntry, DiffStatus, EntryMeta } from "./types";

export type Method = "quick" | "full" | "size" | "mtime" | "size-mtime";
export type Verdict = { status: DiffStatus; detail: string | null };

/** Methods whose verdict is derivable from metadata alone (no content read) → recomputable client-side. */
export function isMetadataMethod(method: Method): method is "size" | "mtime" | "size-mtime" {
  return method === "size" || method === "mtime" || method === "size-mtime";
}

/**
 * Verdict for a both-sides file pair under a metadata-only method, from the size/mtime already held.
 * Mirrors the backend's `binary_verdict` (same detail strings) so client recompute == a fresh scan.
 * Returns `null` for content methods (`full` / `quick`) — those need a byte read, not recomputable here.
 */
export function metadataVerdict(left: EntryMeta, right: EntryMeta, method: Method): Verdict | null {
  const sizeEq = left.size === right.size;
  const mtimeEq = left.mtime === right.mtime; // null === null mirrors the backend's Option<i64> equality
  switch (method) {
    case "size":
      return sizeEq
        ? { status: "identical", detail: null }
        : { status: "different", detail: "size differs" };
    case "mtime":
      return mtimeEq
        ? { status: "identical", detail: null }
        : { status: "different", detail: "modified time differs" };
    case "size-mtime":
      return sizeEq && mtimeEq
        ? { status: "identical", detail: null }
        : { status: "different", detail: "size or modified time differs" };
    default:
      return null; // full / quick → needs content
  }
}

// ---- Bucket 1b: client-side glob matching (mirrors the backend's FilterSet) ----

/**
 * Convert a glob pattern to a RegExp, mirroring globset's default semantics:
 * `*` matches any run of chars that are NOT `/` (single-directory wildcard only — no `**` here);
 * `?` matches one non-`/` char; everything else is literal. Case-sensitive (same as Rust globset default).
 */
function globToRegex(pattern: string): RegExp {
  let re = "^";
  for (const ch of pattern) {
    if (ch === "*") re += "[^/]*";
    else if (ch === "?") re += "[^/]";
    else re += ch.replace(/[.+^${}()|[\]\\]/g, "\\$&"); // escape regex meta chars
  }
  re += "$";
  return new RegExp(re);
}

/**
 * Does an entry at `relPath` match `pattern`? Mirrors the backend's `FilterSet::is_excluded` matching:
 * tries the pattern against the **full path** (`a/b/c.tmp`) and the **bare name** (`c.tmp`).
 */
export function matchesGlob(relPath: string[], pattern: string): boolean {
  const name = relPath[relPath.length - 1] ?? "";
  const fullPath = relPath.join("/");
  const re = globToRegex(pattern);
  return re.test(fullPath) || re.test(name);
}

/**
 * For a removal of exclude patterns, decide whether a backend re-fetch is needed.
 *
 * Returns `true` when any entry that would be un-filtered (matches a removed pattern and doesn't match
 * any remaining active pattern) is NOT in `savedVerdicts` — meaning the backend never compared it and
 * we have no verdict to restore. Returns `false` when all such entries have saved verdicts (they were
 * added client-side AFTER the comparison, so we already hold their real verdicts).
 *
 * This makes `backendExcludes` tracking unnecessary: the `savedVerdicts` map is the authoritative
 * source of truth for "do we have a verdict for this entry?".
 */
export function excludeRemovalNeedsRefetch(
  root: DiffEntry,
  removedPatterns: string[],
  activePatterns: string[],
  savedVerdicts: ReadonlyMap<string, unknown>,
): boolean {
  const walk = (entries: DiffEntry[]): boolean => {
    for (const e of entries) {
      const key = e.rel_path.join("/");
      if (
        e.status === "skipped" &&
        e.detail === "filtered" &&
        !savedVerdicts.has(key) &&
        removedPatterns.some((p) => matchesGlob(e.rel_path, p)) &&
        !activePatterns.some((p) => matchesGlob(e.rel_path, p))
      ) {
        return true; // found an entry we'd need to show but have no verdict for
      }
      if (e.is_dir && e.children.length > 0 && walk(e.children)) return true;
    }
    return false;
  };
  return walk(root.children);
}
