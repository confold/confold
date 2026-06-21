// Reactive selection of diff entries, shared across the (recursive) tree and the page toolbar.
import { SvelteMap } from "svelte/reactivity";
import type { DiffEntry } from "./types";

/** relative-path key → selected entry. */
export const selection = new SvelteMap<string, DiffEntry>();

export const keyOf = (e: DiffEntry): string => e.rel_path.join("/");

export function toggle(e: DiffEntry): void {
  const k = keyOf(e);
  if (selection.has(k)) selection.delete(k);
  else selection.set(k, e);
}

export function clearSelection(): void {
  selection.clear();
}
