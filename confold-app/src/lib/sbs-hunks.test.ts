import { describe, it, expect } from "vitest";
import { linesOf, computeHunks } from "./sbs-hunks";
import { row, fileDiff } from "./test-fixtures";

const sample = () =>
  fileDiff([
    row({ kind: "equal", left_no: 1, right_no: 1, left: "a", right: "a" }),
    row({ kind: "replace", left_no: 2, right_no: 2, left: "b", right: "B" }),
    row({ kind: "insert", right_no: 3, right: "C" }),
    row({ kind: "equal", left_no: 3, right_no: 4, left: "d", right: "d" }),
  ]);

describe("linesOf", () => {
  it("reconstructs each side's line array from the aligned rows", () => {
    expect(linesOf(sample())).toEqual({ left: ["a", "b", "d"], right: ["a", "B", "C", "d"] });
  });
});

describe("computeHunks", () => {
  it("groups consecutive non-equal rows with each side's offset and lines", () => {
    const h = computeHunks(sample());
    expect(h).toHaveLength(1);
    expect(h[0]).toMatchObject({
      startRow: 1,
      endRow: 3,
      leftBefore: 1,
      rightBefore: 1,
      leftLines: ["b"],
      rightLines: ["B", "C"],
    });
  });

  it("returns no hunks for an all-equal diff", () => {
    const d = fileDiff([row({ kind: "equal", left_no: 1, right_no: 1, left: "a", right: "a" })]);
    expect(computeHunks(d)).toEqual([]);
  });

  it("separates two changes by an equal gap into two hunks", () => {
    const d = fileDiff([
      row({ kind: "replace", left_no: 1, right_no: 1, left: "a", right: "A" }),
      row({ kind: "equal", left_no: 2, right_no: 2, left: "x", right: "x" }),
      row({ kind: "replace", left_no: 3, right_no: 3, left: "b", right: "B" }),
    ]);
    const h = computeHunks(d);
    expect(h.map((x) => x.startRow)).toEqual([0, 2]);
  });
});
