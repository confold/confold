import { describe, it, expect, beforeEach } from "vitest";
import { hidden, toggleStatus, nodeVisible } from "./filter.svelte";
import { entry } from "./test-fixtures";

beforeEach(() => hidden.clear());

describe("filter store", () => {
  it("a node is visible when its status is not hidden", () => {
    expect(nodeVisible(entry({ name: "a", status: "identical" }))).toBe(true);
  });

  it("toggleStatus hides and unhides a status", () => {
    toggleStatus("identical");
    expect(hidden.has("identical")).toBe(true);
    expect(nodeVisible(entry({ name: "a", status: "identical" }))).toBe(false);
    toggleStatus("identical");
    expect(hidden.has("identical")).toBe(false);
  });

  it("a hidden directory survives if it contains a visible descendant", () => {
    toggleStatus("identical");
    const dir = entry({
      name: "d",
      is_dir: true,
      status: "identical",
      children: [entry({ name: "c", status: "different" })],
    });
    expect(nodeVisible(dir)).toBe(true);
  });

  it("a hidden directory with only hidden descendants is hidden", () => {
    toggleStatus("identical");
    const dir = entry({
      name: "d",
      is_dir: true,
      status: "identical",
      children: [entry({ name: "c", status: "identical" })],
    });
    expect(nodeVisible(dir)).toBe(false);
  });

  it("filters by effective status: a lazy dir (raw skipped) resolving to identical is hidden", () => {
    // A lazy/pending dir keeps raw status `skipped` until loaded; its real status comes from a
    // resolver. Hiding "identical" must hide a dir whose resolved status is identical.
    toggleStatus("identical");
    const dir = entry({
      name: "d",
      is_dir: true,
      status: "skipped",
      detail: "not descended",
      children: [entry({ name: "c", status: "identical" })],
    });
    // Without a resolver, raw `skipped` is not hidden → visible (the old, buggy behaviour).
    expect(nodeVisible(dir)).toBe(true);
    // With a resolver reporting the effective status, the all-identical dir is hidden.
    const statusOf = (e: typeof dir) =>
      e.status === "skipped" && e.detail === "not descended" ? "identical" : e.status;
    expect(nodeVisible(dir, statusOf)).toBe(false);
  });
});
