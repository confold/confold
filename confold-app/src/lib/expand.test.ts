import { describe, it, expect, beforeEach } from "vitest";
import { expanded, isOpen, toggleOpen } from "./expand.svelte";

beforeEach(() => expanded.clear());

describe("expand store", () => {
  it("directories are collapsed by default", () => {
    expect(isOpen("anything")).toBe(false);
  });

  it("toggleOpen expands then re-collapses a key", () => {
    toggleOpen("d");
    expect(isOpen("d")).toBe(true);
    expect(expanded.has("d")).toBe(true);
    toggleOpen("d");
    expect(isOpen("d")).toBe(false);
    expect(expanded.has("d")).toBe(false);
  });
});
