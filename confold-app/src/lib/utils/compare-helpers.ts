import type { CompareOpts, DiffEntry, DiffRow, FileDiff, FileDiffHunks, FileRef, SourceSpec } from "$lib/types";
import { parseExclude } from "$lib/migrate";

export function isComparing(e: DiffEntry): boolean {
    return !e.is_dir && e.status === "skipped" && e.detail === "comparing";
}

export function hunksToFileDiff(h: FileDiffHunks): FileDiff {
    const rows: DiffRow[] = [];
    for (let i = 0; i < h.hunks.length; i++) {
        if (i > 0) {
            rows.push({
                left_no: null, right_no: null, kind: "equal",
                left: null, right: null,
                left_words: [], right_words: [], left_words_w: [], right_words_w: [],
            } as DiffRow);
        }
        rows.push(...h.hunks[i].rows);
    }
    return { rows, summary: h.summary };
}

export function splitPath(p: string): [string, string] {
    const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
    return i < 0 ? [".", p] : [p.slice(0, i), p.slice(i + 1)];
}

export function fileRefOf(spec: SourceSpec): FileRef {
    const [dir, name] = splitPath(spec.fields.root ?? "");
    const root = dir || (spec.kind === "fs" ? "" : "/");
    return { source: { kind: spec.kind, fields: { ...spec.fields, root } }, rel: name };
}

export const refKey = (r: FileRef) => `${r.source.fields.root ?? ""}/${r.rel}`;

export function makeOpts(m: CompareOpts["method"], excludeStr: string): CompareOpts {
    return { method: m, include: [], exclude: parseExclude(excludeStr) };
}
