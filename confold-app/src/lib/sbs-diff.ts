// Intra-line diff rendering for the side-by-side view: pure helpers over diff rows + the backend's
// char/word change ranges. No DOM (tested in sbs-diff.test.ts via `pnpm test`).
import type { DiffRow, WordRange } from "./types";

/** A run of a line tagged as changed or not — for highlighting in the main panes. */
export type Seg = { text: string; changed: boolean };

/** Split a line into segments by the changed ranges (char offsets), marking each changed/unchanged. */
export function segments(text: string | null, ranges: WordRange[]): Seg[] {
  if (!text) return [{ text: "", changed: false }];
  if (ranges.length === 0) return [{ text, changed: false }];
  const chars = [...text];
  const out: Seg[] = [];
  let pos = 0;
  for (const r of ranges) {
    if (r.start > pos) out.push({ text: chars.slice(pos, r.start).join(""), changed: false });
    out.push({ text: chars.slice(r.start, r.end).join(""), changed: true });
    pos = r.end;
  }
  if (pos < chars.length) out.push({ text: chars.slice(pos).join(""), changed: false });
  return out;
}

/** Changed ranges for a row honouring the word/char toggle (backend provides both; only `replace`). */
export function rangesFor(row: DiffRow, wordMode: boolean): { left: WordRange[]; right: WordRange[] } {
  if (wordMode && row.kind === "replace") return { left: row.left_words_w, right: row.right_words_w };
  return { left: row.left_words, right: row.right_words };
}

/** Char-level alignment of a replaced line pair, reconstructed from the per-side changed ranges. */
type Cell = { l: string; r: string; kind: "eq" | "del" | "ins" | "rep" };
function alignPair(lt: string, lr: WordRange[], rt: string, rr: WordRange[]): Cell[] {
  const L = [...lt];
  const R = [...rt];
  const cells: Cell[] = [];
  let li = 0;
  let ri = 0;
  let ld = 0;
  let rd = 0;
  while (li < L.length || ri < R.length) {
    const inDel = ld < lr.length && li >= lr[ld].start && li < lr[ld].end;
    const inIns = rd < rr.length && ri >= rr[rd].start && ri < rr[rd].end;
    if (inDel && inIns) {
      cells.push({ l: L.slice(li, lr[ld].end).join(""), r: R.slice(ri, rr[rd].end).join(""), kind: "rep" });
      li = lr[ld].end;
      ri = rr[rd].end;
      ld++;
      rd++;
    } else if (inDel) {
      cells.push({ l: L.slice(li, lr[ld].end).join(""), r: "", kind: "del" });
      li = lr[ld].end;
      ld++;
    } else if (inIns) {
      cells.push({ l: "", r: R.slice(ri, rr[rd].end).join(""), kind: "ins" });
      ri = rr[rd].end;
      rd++;
    } else {
      const lnext = ld < lr.length ? lr[ld].start : L.length;
      const rnext = rd < rr.length ? rr[rd].start : R.length;
      const n = Math.min(lnext - li, rnext - ri);
      if (n <= 0) break; // safety against a malformed range set
      cells.push({ l: L.slice(li, li + n).join(""), r: R.slice(ri, ri + n).join(""), kind: "eq" });
      li += n;
      ri += n;
    }
  }
  return cells;
}

// cls drives the colour: eq = modified-line context (light orange), rep = changed word (dark orange),
// del = removed (red), ins = added (green), gap = blank padding (no shading, so it reads as a hole).
export type DCellKind = "eq" | "del" | "ins" | "rep" | "gap";
export type DCell = { text: string; cls: DCellKind; stop: number };
export type DLine = { sign: "-" | "+"; cells: DCell[] };

/**
 * Build the detail block as GROUPS: a replaced row → a group of two gap-aligned `-`/`+` lines; a whole
 * add/remove row → a one-line group. `stopCount` is the number of edits the stepper walks.
 */
export function buildDetailModel(
  rows: DiffRow[],
  curRange: { start: number; end: number } | null,
  wordMode: boolean,
): { groups: DLine[][]; stopCount: number } {
  const groups: DLine[][] = [];
  let stop = 0;
  if (!curRange) return { groups, stopCount: 0 };
  // Push the real text (coloured), then — if this side is shorter — a SEPARATE neutral gap of the
  // padding, so alignment spaces never look like a changed/added space.
  const emit = (arr: DCell[], text: string, cls: DCellKind, s: number, w: number) => {
    arr.push({ text, cls, stop: s });
    const pad = w - [...text].length;
    if (pad > 0) arr.push({ text: " ".repeat(pad), cls: "gap", stop: -1 });
  };
  const gap = (arr: DCell[], w: number) => arr.push({ text: " ".repeat(w), cls: "gap", stop: -1 });
  for (let r = curRange.start; r < curRange.end; r++) {
    const row = rows[r];
    if (row.kind === "delete") {
      const t = row.left ?? "";
      groups.push([{ sign: "-", cells: [{ text: t, cls: "del", stop: t.length ? stop++ : -1 }] }]);
    } else if (row.kind === "insert") {
      const t = row.right ?? "";
      groups.push([{ sign: "+", cells: [{ text: t, cls: "ins", stop: t.length ? stop++ : -1 }] }]);
    } else if (row.kind === "replace") {
      const ew = rangesFor(row, wordMode);
      const cells = alignPair(row.left ?? "", ew.left, row.right ?? "", ew.right);
      const minus: DCell[] = [];
      const plus: DCell[] = [];
      for (const c of cells) {
        const w = Math.max([...c.l].length, [...c.r].length);
        if (c.kind === "eq") {
          emit(minus, c.l, "eq", -1, w);
          emit(plus, c.r, "eq", -1, w);
        } else if (c.kind === "rep") {
          const s = stop++;
          emit(minus, c.l, "rep", s, w);
          emit(plus, c.r, "rep", s, w);
        } else if (c.kind === "del") {
          const s = stop++;
          emit(minus, c.l, "del", s, w);
          gap(plus, w);
        } else {
          const s = stop++;
          gap(minus, w);
          emit(plus, c.r, "ins", s, w);
        }
      }
      groups.push([
        { sign: "-", cells: minus },
        { sign: "+", cells: plus },
      ]);
    }
  }
  return { groups, stopCount: stop };
}
