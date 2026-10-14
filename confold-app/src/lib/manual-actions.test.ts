import { describe, it, expect } from "vitest";
import { identicalStats, expandSkippingIdentical } from "./manual-actions";
import type { DiffEntry, SyncOp } from "./types";

const COPY: SyncOp = "copy_left_to_right";

function entry(partial: Partial<DiffEntry> & { rel_path: string[] }): DiffEntry {
  return {
    name: partial.rel_path[partial.rel_path.length - 1] ?? "",
    is_dir: false,
    status: "identical",
    left: null,
    right: null,
    detail: null,
    children: [],
    ...partial,
  } as DiffEntry;
}

const dir = (rel: string[], status: string, children: DiffEntry[]) =>
  entry({ rel_path: rel, is_dir: true, status: status as DiffEntry["status"], children });
const file = (rel: string[], status: string) =>
  entry({ rel_path: rel, status: status as DiffEntry["status"] });

describe("identicalStats", () => {
  it("counts identical files under a fully-loaded folder", () => {
    const sel = [
      dir(["2014"], "different", [
        dir(["2014", "sub"], "different", [file(["2014", "sub", "a.pdf"], "identical")]),
        file(["2014", "b.docx"], "different"),
        file(["2014", "c.txt"], "left_only"),
      ]),
    ];
    expect(identicalStats(COPY, sel)).toEqual({ count: 1, hasUnknown: false });
  });

  it("still counts known identical FILES in a Lazy batch with pending folders", () => {
    // The owner's case: Lazy mode leaves folders undescribed; top-level identical files must
    // still trigger the guard instead of silently copying everything.
    const pending = dir(["2014 - Presupuestos"], "skipped", []);
    Object.assign(pending, { detail: "not descended" });
    const sel = [pending, file(["informe.odt"], "identical"), file(["nuevo.odt"], "different")];
    expect(identicalStats(COPY, sel)).toEqual({ count: 1, hasUnknown: true });
  });

  it("trusts a preloaded folder even though its raw status still says skipped", () => {
    // Lazy folders keep status "skipped"/"not descended" after their level loads (the UI resolves
    // via resolveDir); with dirKnown=true the walk must use the loaded children.
    const preloaded = dir(["2009 - Presupuestos"], "skipped", [
      file(["2009 - Presupuestos", "a.odt"], "identical"),
      file(["2009 - Presupuestos", "b.odt"], "identical"),
    ]);
    Object.assign(preloaded, { detail: "not descended" });
    const dirKnown = (e: DiffEntry) => e.rel_path.length === 0 || e.rel_path[0].startsWith("2009");
    expect(identicalStats(COPY, [preloaded], dirKnown)).toEqual({
      count: 2,
      hasUnknown: false,
    });
    const actions = expandSkippingIdentical(COPY, [preloaded], dirKnown);
    expect(actions).toEqual([]); // everything identical inside -> nothing to copy
  });

  it("is unknown while any file verdict is still streaming", () => {
    const sel = [dir(["d"], "different", [file(["d", "x"], "skipped")])];
    Object.assign(sel[0].children[0], { detail: "comparing" });
    expect(identicalStats(COPY, sel)).toEqual({ count: 0, hasUnknown: true });
  });

  it("ignores deleted ops — deletes pass through without any guard", () => {
    const sel = [file(["a"], "identical")];
    expect(identicalStats("delete_right", sel)).toEqual({ count: 0, hasUnknown: false });
  });
});

describe("expandSkippingIdentical", () => {
  it("keeps non-identical files, drops identical ones, and creates only empty folders", () => {
    const sel = [
      dir(["2014"], "different", [
        dir(["2014", "sub"], "identical", [file(["2014", "sub", "a.pdf"], "identical")]),
        file(["2014", "b.docx"], "different"),
        file(["2014", "c.txt"], "left_only"),
        dir(["2014", "empty"], "identical", []),
      ]),
    ];
    const actions = expandSkippingIdentical(COPY, sel);
    const keys = actions.map((a) => a.rel_path.join("/"));
    expect(keys).toEqual(["2014/b.docx", "2014/c.txt", "2014/empty"]);
    expect(actions.find((a) => a.is_dir)?.rel_path.join("/")).toBe("2014/empty");
  });

  it("keeps a pending folder recursive while filtering known identical siblings", () => {
    const pending = dir(["2014 - Presupuestos"], "skipped", []);
    Object.assign(pending, { detail: "not descended" });
    const sel = [pending, file(["informe.odt"], "identical"), file(["nuevo.odt"], "different")];
    const actions = expandSkippingIdentical(COPY, sel);
    expect(actions).toEqual([
      { rel_path: ["2014 - Presupuestos"], op: COPY, is_dir: true },
      { rel_path: ["nuevo.odt"], op: COPY, is_dir: false },
    ]);
  });

  it("never filters delete ops, even for identical files", () => {
    const sel = [file(["a"], "identical"), dir(["d"], "identical", [file(["d", "x"], "identical")])];
    const actions = expandSkippingIdentical("delete_right", sel);
    expect(actions).toHaveLength(2);
    expect(actions.every((a) => a.op === "delete_right")).toBe(true);
    expect(actions.find((a) => a.is_dir)?.rel_path.join("/")).toBe("d");
  });

  it("deduplicates a file selected both directly and through its folder", () => {
    const sel = [
      dir(["d"], "different", [file(["d", "b"], "different")]),
      file(["d", "b"], "different"),
    ];
    const actions = expandSkippingIdentical(COPY, sel);
    expect(actions).toHaveLength(1);
  });
});
