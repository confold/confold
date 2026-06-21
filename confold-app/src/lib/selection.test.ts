import { describe, it, expect, beforeEach } from "vitest";
import { selection, toggle, keyOf, clearSelection } from "./selection.svelte";
import { entry } from "./test-fixtures";

beforeEach(() => clearSelection());

describe("selection store", () => {
  it("keyOf joins the relative path", () => {
    expect(keyOf(entry({ name: "x", rel_path: ["a", "b", "x"] }))).toBe("a/b/x");
  });

  it("toggle adds then removes an entry", () => {
    const e = entry({ name: "f", rel_path: ["f"] });
    toggle(e);
    expect(selection.has("f")).toBe(true);
    expect(selection.size).toBe(1);
    toggle(e);
    expect(selection.has("f")).toBe(false);
    expect(selection.size).toBe(0);
  });

  it("clearSelection empties the map", () => {
    toggle(entry({ name: "a", rel_path: ["a"] }));
    toggle(entry({ name: "b", rel_path: ["b"] }));
    expect(selection.size).toBe(2);
    clearSelection();
    expect(selection.size).toBe(0);
  });
});
