import { describe, it, expect } from "vitest";
import { segments, rangesFor, buildDetailModel } from "./sbs-diff";
import { row } from "./test-fixtures";

describe("segments", () => {
  it("splits a line by changed ranges", () => {
    expect(segments("hello", [{ start: 1, end: 3 }])).toEqual([
      { text: "h", changed: false },
      { text: "el", changed: true },
      { text: "lo", changed: false },
    ]);
  });

  it("handles null text and no ranges", () => {
    expect(segments(null, [])).toEqual([{ text: "", changed: false }]);
    expect(segments("x", [])).toEqual([{ text: "x", changed: false }]);
  });
});

describe("rangesFor", () => {
  const r = row({
    kind: "replace",
    left: "a",
    right: "b",
    left_words: [{ start: 0, end: 1 }],
    right_words: [{ start: 0, end: 1 }],
    left_words_w: [{ start: 0, end: 9 }],
    right_words_w: [{ start: 0, end: 9 }],
  });

  it("uses character ranges in char mode", () => {
    expect(rangesFor(r, false)).toEqual({ left: [{ start: 0, end: 1 }], right: [{ start: 0, end: 1 }] });
  });

  it("uses word ranges in word mode", () => {
    expect(rangesFor(r, true)).toEqual({ left: [{ start: 0, end: 9 }], right: [{ start: 0, end: 9 }] });
  });

  it("ignores word mode for non-replace rows", () => {
    expect(rangesFor(row({ kind: "delete", left: "x" }), true)).toEqual({ left: [], right: [] });
  });
});

describe("buildDetailModel", () => {
  it("is empty when no change is selected", () => {
    expect(buildDetailModel([], null, false)).toEqual({ groups: [], stopCount: 0 });
  });

  const repRow = () => [
    row({
      kind: "replace",
      left: "foo123bar",
      right: "foo456bar",
      left_words: [{ start: 3, end: 6 }],
      right_words: [{ start: 3, end: 6 }],
      left_words_w: [{ start: 0, end: 9 }],
      right_words_w: [{ start: 0, end: 9 }],
    }),
  ];

  it("builds an aligned − / + group for a replaced row (char mode)", () => {
    const m = buildDetailModel(repRow(), { start: 0, end: 1 }, false);
    expect(m.stopCount).toBe(1);
    expect(m.groups).toHaveLength(1);
    const [minus, plus] = m.groups[0];
    expect(minus.sign).toBe("-");
    expect(plus.sign).toBe("+");
    expect(minus.cells.map((c) => c.text).join("")).toBe("foo123bar");
    expect(plus.cells.map((c) => c.text).join("")).toBe("foo456bar");
    expect(minus.cells.find((c) => c.cls === "rep")).toMatchObject({ text: "123", stop: 0 });
    expect(plus.cells.find((c) => c.cls === "rep")).toMatchObject({ text: "456", stop: 0 });
  });

  it("highlights the whole token as one rep cell in word mode", () => {
    const m = buildDetailModel(repRow(), { start: 0, end: 1 }, true);
    const [minus, plus] = m.groups[0];
    expect(minus.cells).toEqual([{ text: "foo123bar", cls: "rep", stop: 0 }]);
    expect(plus.cells).toEqual([{ text: "foo456bar", cls: "rep", stop: 0 }]);
  });

  it("pads the shorter side with a NEUTRAL gap cell (not coloured)", () => {
    // left "ab", right "abc": nothing removed on the left, "c" inserted on the right.
    const rows = [
      row({
        kind: "replace",
        left: "ab",
        right: "abc",
        left_words: [],
        right_words: [{ start: 2, end: 3 }],
      }),
    ];
    const [minus, plus] = buildDetailModel(rows, { start: 0, end: 1 }, false).groups[0];
    expect(minus.cells).toEqual([
      { text: "ab", cls: "eq", stop: -1 },
      { text: " ", cls: "gap", stop: -1 },
    ]);
    expect(plus.cells).toEqual([
      { text: "ab", cls: "eq", stop: -1 },
      { text: "c", cls: "ins", stop: 0 },
    ]);
  });

  it("renders whole add/remove rows as single-line groups", () => {
    const rows = [
      row({ kind: "delete", left_no: 1, left: "gone" }),
      row({ kind: "insert", right_no: 1, right: "added" }),
    ];
    const m = buildDetailModel(rows, { start: 0, end: 2 }, false);
    expect(m.stopCount).toBe(2);
    expect(m.groups).toEqual([
      [{ sign: "-", cells: [{ text: "gone", cls: "del", stop: 0 }] }],
      [{ sign: "+", cells: [{ text: "added", cls: "ins", stop: 1 }] }],
    ]);
  });
});
