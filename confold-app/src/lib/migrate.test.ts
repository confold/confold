import { describe, it, expect } from "vitest";
import {
  actionKey,
  parseExclude,
  planItems,
  uncheckedCount,
  checkedItemTotal,
  splitChecked,
  appliedByReason,
  appliedCount,
  skippedCount,
  type MigrateAction,
  type MigrateOutcome,
} from "./migrate";

const action = (
  path: string[],
  reason: MigrateAction["reason"],
  item_count = 1,
  is_dir = false,
): MigrateAction => ({ rel_path: path, op: "copy_left_to_right", is_dir, reason, item_count });

const outcome = (
  path: string,
  reason: MigrateOutcome["reason"],
  opts: Partial<MigrateOutcome> = {},
): MigrateOutcome => ({ path, reason, ok: true, error: null, ...opts });

describe("actionKey", () => {
  it("joins the rel path with '/'", () => {
    expect(actionKey(action(["a", "b", "c.txt"], "new"))).toBe("a/b/c.txt");
    expect(actionKey(action(["top.txt"], "new"))).toBe("top.txt");
  });
});

describe("parseExclude", () => {
  it("trims, drops blanks, and splits on commas", () => {
    expect(parseExclude("*.tmp, node_modules ,  ,.git")).toEqual(["*.tmp", "node_modules", ".git"]);
  });
  it("returns an empty list for an empty/blank string", () => {
    expect(parseExclude("")).toEqual([]);
    expect(parseExclude("  ,  ")).toEqual([]);
  });
});

describe("planItems", () => {
  const actions = [
    action(["new.txt"], "new"),
    action(["newdir"], "new", 45, true), // a dir counts its whole subtree
    action(["diff.txt"], "different"),
    action(["extra.txt"], "extra"),
  ];
  it("sums item_count per reason (dirs count their subtree)", () => {
    expect(planItems(actions, "new")).toBe(46); // 1 + 45
    expect(planItems(actions, "different")).toBe(1);
    expect(planItems(actions, "extra")).toBe(1);
  });
  it("is zero for a reason with no actions", () => {
    expect(planItems([action(["x"], "new")], "extra")).toBe(0);
  });
});

describe("checkbox accounting", () => {
  const actions = [
    action(["a.txt"], "new"),
    action(["dir"], "new", 10, true),
    action(["c.txt"], "different"),
  ];

  it("uncheckedCount counts actions whose key is not in the set", () => {
    expect(uncheckedCount(actions, new Set(["a.txt", "dir", "c.txt"]))).toBe(0);
    expect(uncheckedCount(actions, new Set(["a.txt"]))).toBe(2);
    expect(uncheckedCount(actions, new Set())).toBe(3);
  });

  it("checkedItemTotal sums item_count over checked actions only", () => {
    // all checked → 1 + 10 + 1
    expect(checkedItemTotal(actions, new Set(["a.txt", "dir", "c.txt"]))).toBe(12);
    // only the dir checked → 10
    expect(checkedItemTotal(actions, new Set(["dir"]))).toBe(10);
    expect(checkedItemTotal(actions, new Set())).toBe(0);
  });
});

describe("splitChecked", () => {
  const actions = [
    action(["keep.txt"], "new"),
    action(["skip.txt"], "different"),
    action(["gone.txt"], "extra"),
  ];

  it("routes checked actions to apply and builds skipped outcomes for the rest", () => {
    const { toApply, skipped } = splitChecked(actions, new Set(["keep.txt", "gone.txt"]));

    expect(toApply.map((a) => actionKey(a))).toEqual(["keep.txt", "gone.txt"]);

    expect(skipped).toEqual([
      { path: "skip.txt", reason: "different", ok: true, error: null, skipped: true, op: "copy_left_to_right" },
    ]);
  });

  it("everything checked → nothing skipped", () => {
    const { toApply, skipped } = splitChecked(actions, new Set(["keep.txt", "skip.txt", "gone.txt"]));
    expect(toApply).toHaveLength(3);
    expect(skipped).toHaveLength(0);
  });

  it("nothing checked → all skipped, none applied", () => {
    const { toApply, skipped } = splitChecked(actions, new Set());
    expect(toApply).toHaveLength(0);
    expect(skipped).toHaveLength(3);
    expect(skipped.every((o) => o.skipped && o.ok)).toBe(true);
  });
});

describe("progress counters", () => {
  const outcomes = [
    outcome("a.txt", "new"),
    outcome("b.txt", "new", { skipped: true }), // pre-populated skip — excluded from applied counts
    outcome("c.txt", "different"),
    outcome("d.txt", "extra", { ok: false, error: "boom" }),
    outcome("e.txt", "extra", { skipped: true }),
  ];

  it("appliedByReason excludes skipped outcomes", () => {
    expect(appliedByReason(outcomes, "new")).toBe(1); // a.txt only (b.txt is skipped)
    expect(appliedByReason(outcomes, "different")).toBe(1);
    expect(appliedByReason(outcomes, "extra")).toBe(1); // d.txt (failed still counts as applied/attempted)
  });

  it("appliedCount and skippedCount partition the outcomes", () => {
    expect(appliedCount(outcomes)).toBe(3); // a, c, d
    expect(skippedCount(outcomes)).toBe(2); // b, e
    expect(appliedCount(outcomes) + skippedCount(outcomes)).toBe(outcomes.length);
  });
});
