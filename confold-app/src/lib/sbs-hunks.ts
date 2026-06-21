// Hunk geometry for the side-by-side view: pure helpers over a `FileDiff` (no DOM; tested via `pnpm test`).
import type { FileDiff } from "./types";

/** A change block (consecutive non-equal rows) + where its lines sit in each side's line array. */
export type Hunk = {
  startRow: number;
  endRow: number; // exclusive — rows [startRow, endRow) belong to this hunk
  leftBefore: number;
  rightBefore: number;
  leftLines: string[];
  rightLines: string[];
};

/** Reconstruct the current line arrays of each side from the aligned diff. */
export function linesOf(d: FileDiff): { left: string[]; right: string[] } {
  const left: string[] = [];
  const right: string[] = [];
  for (const r of d.rows) {
    if (r.left_no !== null) left.push(r.left ?? "");
    if (r.right_no !== null) right.push(r.right ?? "");
  }
  return { left, right };
}

/** Group consecutive non-equal rows into hunks, tracking each side's line offset. */
export function computeHunks(d: FileDiff): Hunk[] {
  const out: Hunk[] = [];
  let leftCount = 0;
  let rightCount = 0;
  let i = 0;
  const rows = d.rows;
  while (i < rows.length) {
    if (rows[i].kind === "equal") {
      leftCount++;
      rightCount++;
      i++;
      continue;
    }
    const startRow = i;
    const leftBefore = leftCount;
    const rightBefore = rightCount;
    const leftLines: string[] = [];
    const rightLines: string[] = [];
    while (i < rows.length && rows[i].kind !== "equal") {
      if (rows[i].left_no !== null) {
        leftLines.push(rows[i].left ?? "");
        leftCount++;
      }
      if (rows[i].right_no !== null) {
        rightLines.push(rows[i].right ?? "");
        rightCount++;
      }
      i++;
    }
    out.push({ startRow, endRow: i, leftBefore, rightBefore, leftLines, rightLines });
  }
  return out;
}
