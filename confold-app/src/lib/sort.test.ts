import { describe, it, expect, beforeEach } from "vitest";
import { sortState, sortEntries } from "./sort.svelte";
import { entry } from "./test-fixtures";
import type { EntryMeta } from "./types";

const meta = (size: number, mtime: number | null = 0): EntryMeta => ({
  name: "",
  rel_path: [],
  kind: "file",
  size,
  mtime,
  created: null,
});

beforeEach(() => {
  sortState.key = "name";
});

describe("sortEntries", () => {
  it("puts directories first, then files by name", () => {
    const out = sortEntries([
      entry({ name: "b" }),
      entry({ name: "a" }),
      entry({ name: "zdir", is_dir: true }),
    ]);
    expect(out.map((e) => e.name)).toEqual(["zdir", "a", "b"]);
  });

  it("sorts by status with divergences first", () => {
    sortState.key = "status";
    const out = sortEntries([
      entry({ name: "a", status: "identical" }),
      entry({ name: "b", status: "different" }),
      entry({ name: "c", status: "left_only" }),
    ]);
    expect(out.map((e) => e.name)).toEqual(["b", "c", "a"]);
  });

  it("status sort uses the effective status (resolved lazy dir, not raw skipped)", () => {
    sortState.key = "status";
    // A lazy dir resolving to "different" still has raw status "skipped"; a resolver surfaces its
    // effective status so it sorts above an identical file.
    const dir = entry({ name: "d", is_dir: true, status: "skipped", detail: "not descended" });
    const file = entry({ name: "f", status: "identical" });
    const statusOf = (e: typeof dir) =>
      e.status === "skipped" && e.detail === "not descended" ? "different" : e.status;
    // Without the resolver, raw "skipped" (weight 4) sorts the dir after "identical"? No — dirs always
    // come first, so check it's the effective ordering among same-kind via a second dir.
    const dir2 = entry({ name: "e", is_dir: true, status: "identical" });
    const out = sortEntries([dir2, dir], statusOf);
    // dir (effective "different") sorts before dir2 ("identical").
    expect(out.map((e) => e.name)).toEqual(["d", "e"]);
    expect(file.name).toBe("f"); // (file kept to document the dirs-first rule)
  });

  it("sorts by size descending (largest of the two sides)", () => {
    sortState.key = "size";
    const out = sortEntries([
      entry({ name: "small", left: meta(10) }),
      entry({ name: "big", right: meta(100) }),
    ]);
    expect(out.map((e) => e.name)).toEqual(["big", "small"]);
  });

  it("does not mutate the input array", () => {
    const input = [entry({ name: "b" }), entry({ name: "a" })];
    sortEntries(input);
    expect(input.map((e) => e.name)).toEqual(["b", "a"]);
  });
});
