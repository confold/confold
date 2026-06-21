// Reactive expand/collapse state for directory rows (default: everything COLLAPSED).
import { SvelteSet } from "svelte/reactivity";

/** Keys (relative paths) of EXPANDED directories. Empty = all collapsed (the default). */
export const expanded = new SvelteSet<string>();

export function isOpen(key: string): boolean {
  return expanded.has(key);
}

export function toggleOpen(key: string): void {
  if (expanded.has(key)) expanded.delete(key);
  else expanded.add(key);
}

export function clearExpanded(): void {
  expanded.clear();
}
