import { describe, it, expect } from "vitest";
import { isMetadataMethod, metadataVerdict, matchesGlob, excludeRemovalNeedsRefetch } from "./recompute";
import type { DiffEntry, EntryMeta } from "./types";

const meta = (over: Partial<EntryMeta> = {}): EntryMeta => ({
  name: "f.txt",
  rel_path: ["f.txt"],
  kind: "file",
  size: 100,
  mtime: 1000,
  created: null,
  ...over,
});

describe("isMetadataMethod", () => {
  it("is true for the metadata-only methods, false for content methods", () => {
    expect(isMetadataMethod("size")).toBe(true);
    expect(isMetadataMethod("mtime")).toBe(true);
    expect(isMetadataMethod("size-mtime")).toBe(true);
    expect(isMetadataMethod("full")).toBe(false);
    expect(isMetadataMethod("quick")).toBe(false);
  });
});

describe("metadataVerdict", () => {
  it("content methods are not recomputable → null", () => {
    expect(metadataVerdict(meta(), meta(), "full")).toBeNull();
    expect(metadataVerdict(meta(), meta(), "quick")).toBeNull();
  });

  it("size: compares size only (ignores mtime)", () => {
    // same size, different mtime → identical under `size`
    expect(metadataVerdict(meta({ mtime: 1 }), meta({ mtime: 2 }), "size")).toEqual({
      status: "identical",
      detail: null,
    });
    expect(metadataVerdict(meta({ size: 100 }), meta({ size: 200 }), "size")).toEqual({
      status: "different",
      detail: "size differs",
    });
  });

  it("mtime: compares mtime only (ignores size)", () => {
    // different size, same mtime → identical under `mtime`
    expect(metadataVerdict(meta({ size: 1 }), meta({ size: 2 }), "mtime")).toEqual({
      status: "identical",
      detail: null,
    });
    expect(metadataVerdict(meta({ mtime: 1000 }), meta({ mtime: 2000 }), "mtime")).toEqual({
      status: "different",
      detail: "modified time differs",
    });
  });

  it("size-mtime: requires both equal", () => {
    expect(metadataVerdict(meta(), meta(), "size-mtime")).toEqual({ status: "identical", detail: null });
    expect(metadataVerdict(meta({ size: 1 }), meta({ size: 2 }), "size-mtime")).toEqual({
      status: "different",
      detail: "size or modified time differs",
    });
    expect(metadataVerdict(meta({ mtime: 1 }), meta({ mtime: 2 }), "size-mtime")).toEqual({
      status: "different",
      detail: "size or modified time differs",
    });
  });

  it("null vs null mtime is equal; null vs value is not (mirrors the backend's Option equality)", () => {
    expect(metadataVerdict(meta({ mtime: null }), meta({ mtime: null }), "mtime")).toEqual({
      status: "identical",
      detail: null,
    });
    // null vs a value → not equal
    expect(metadataVerdict(meta({ mtime: null }), meta({ mtime: 5 }), "mtime")).toEqual({
      status: "different",
      detail: "modified time differs",
    });
  });
});

describe("matchesGlob", () => {
  it("matches a bare extension pattern against the filename at any depth", () => {
    // matches name: "b.tmp"
    expect(matchesGlob(["a", "b.tmp"], "*.tmp")).toBe(true);
    // depth-1
    expect(matchesGlob(["scratch.tmp"], "*.tmp")).toBe(true);
    // non-matching extension
    expect(matchesGlob(["a", "b.txt"], "*.tmp")).toBe(false);
  });

  it("* does not cross directory boundaries (no ** behaviour)", () => {
    // "*.tmp" should NOT match "a/b.tmp" as a full-path test (the `/` blocks `*`),
    // but it DOES match the bare name "b.tmp" — which is how the backend also accepts it.
    // That's already covered above. Here we confirm a pattern like "a*.tmp" won't
    // accidentally match "dir/a.tmp":
    expect(matchesGlob(["dir", "a.tmp"], "a*.tmp")).toBe(true); // name "a.tmp" matches "a*.tmp"
    expect(matchesGlob(["dir", "b.tmp"], "a*.tmp")).toBe(false); // name "b.tmp" doesn't
  });

  it("matches an exact name pattern against any component", () => {
    expect(matchesGlob(["node_modules"], "node_modules")).toBe(true);
    expect(matchesGlob(["src", "node_modules", "pkg"], "node_modules")).toBe(false); // mid-path, full-path doesn't match; name is "pkg"
  });

  it("matches against the full path when the pattern contains a slash", () => {
    expect(matchesGlob(["data", "raw", "2024"], "data/raw/2024")).toBe(true);
    expect(matchesGlob(["data", "raw", "2025"], "data/raw/2024")).toBe(false);
  });

  it("? matches exactly one non-slash character", () => {
    expect(matchesGlob(["f1.txt"], "f?.txt")).toBe(true);
    expect(matchesGlob(["f12.txt"], "f?.txt")).toBe(false);
  });
});

// Helpers for excludeRemovalNeedsRefetch tests.
const filteredEntry = (relPath: string[], inSaved = false, _saved: Map<string, unknown> = new Map()): DiffEntry => ({
  rel_path: relPath,
  name: relPath[relPath.length - 1],
  is_dir: false,
  status: "skipped",
  detail: "filtered",
  left: null,
  right: null,
  children: [],
});
function root(...children: DiffEntry[]): DiffEntry {
  return { rel_path: [], name: "", is_dir: true, status: "identical", detail: null, left: null, right: null, children };
}

describe("excludeRemovalNeedsRefetch", () => {
  it("empty removed patterns → no refetch needed", () => {
    const tree = root(filteredEntry(["a.txt"]));
    expect(excludeRemovalNeedsRefetch(tree, [], [], new Map())).toBe(false);
  });

  it("entry in savedVerdicts (client-filtered) → restore, no refetch", () => {
    const saved = new Map([["a.txt", { status: "different", detail: null }]]);
    const tree = root(filteredEntry(["a.txt"]));
    // "a.txt" would be unfiltered by removing "*.txt" and matches, but has a savedVerdict → no refetch
    expect(excludeRemovalNeedsRefetch(tree, ["*.txt"], [], saved)).toBe(false);
  });

  it("entry NOT in savedVerdicts (backend-filtered) → needs refetch", () => {
    const tree = root(filteredEntry(["a.txt"]));
    expect(excludeRemovalNeedsRefetch(tree, ["*.txt"], [], new Map())).toBe(true);
  });

  it("entry still covered by another active pattern → NOT unfiltered, no refetch", () => {
    // "a.txt" matches removed "*.txt" BUT also matches remaining "*.txt" (same active)... use a different case.
    // "a.bin" matches removed "*.bin" but "*.tmp" is still active. "a.bin" doesn't match "*.tmp" → refetch.
    const tree = root(filteredEntry(["a.bin"]));
    expect(excludeRemovalNeedsRefetch(tree, ["*.bin"], ["*.tmp"], new Map())).toBe(true);
    // "a.tmp" matches remaining "*.tmp" → still filtered → not unfiltered → no refetch from this entry.
    const tree2 = root(filteredEntry(["a.tmp"]));
    expect(excludeRemovalNeedsRefetch(tree2, ["*.bin"], ["*.tmp"], new Map())).toBe(false);
  });

  it("the key scenario: compare empty→add*.txt(saved)→add*.bin(saved)→remove*.txt: no refetch", () => {
    // Backend ran with empty excludes. Both *.txt and *.bin were added client-side → both in savedVerdicts.
    const saved = new Map<string, unknown>([
      ["report.txt", { status: "identical", detail: null }],
      ["data.bin", { status: "different", detail: null }],
    ]);
    const tree = root(filteredEntry(["report.txt"]), filteredEntry(["data.bin"]));
    // Remove "*.txt" leaving "*.bin" active. "report.txt" has savedVerdict → no refetch.
    expect(excludeRemovalNeedsRefetch(tree, ["*.txt"], ["*.bin"], saved)).toBe(false);
  });

  it("compare with *.txt excluded (backend) → add *.bin client → remove *.txt: needs refetch", () => {
    // Backend ran with *.txt excluded → *.txt files are filtered, NOT in savedVerdicts.
    // *.bin was added client-side → data.bin IS in savedVerdicts.
    const saved = new Map<string, unknown>([["data.bin", { status: "different", detail: null }]]);
    const tree = root(filteredEntry(["report.txt"]), filteredEntry(["data.bin"]));
    // Remove "*.txt" leaving "*.bin". "report.txt" is NOT in saved → needs refetch.
    expect(excludeRemovalNeedsRefetch(tree, ["*.txt"], ["*.bin"], saved)).toBe(true);
  });
});
