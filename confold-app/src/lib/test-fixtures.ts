// Tiny builders for unit tests (not a test file itself; imported by *.test.ts). Run: `pnpm test`.
import type { DiffEntry, DiffRow, FileDiff, RowKind } from "./types";

export function entry(p: Partial<DiffEntry> & { name: string }): DiffEntry {
  return {
    rel_path: p.rel_path ?? [p.name],
    name: p.name,
    is_dir: p.is_dir ?? false,
    status: p.status ?? "identical",
    left: p.left ?? null,
    right: p.right ?? null,
    detail: p.detail ?? null,
    children: p.children ?? [],
  };
}

export function row(p: Partial<DiffRow> & { kind: RowKind }): DiffRow {
  return {
    left_no: p.left_no ?? null,
    right_no: p.right_no ?? null,
    kind: p.kind,
    left: p.left ?? null,
    right: p.right ?? null,
    left_words: p.left_words ?? [],
    right_words: p.right_words ?? [],
    left_words_w: p.left_words_w ?? [],
    right_words_w: p.right_words_w ?? [],
  };
}

export const fileDiff = (rows: DiffRow[]): FileDiff => ({
  rows,
  summary: { equal: 0, inserted: 0, deleted: 0, replaced: 0 },
});
