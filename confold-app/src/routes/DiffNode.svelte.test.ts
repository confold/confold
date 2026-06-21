// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import DiffNode from "./DiffNode.svelte";
import type { DiffEntry, EntryMeta } from "$lib/types";
import { selection, keyOf, clearSelection } from "$lib/selection.svelte";
import { clearExpanded } from "$lib/expand.svelte";

// Shared module-level stores leak across renders → reset before each test.
beforeEach(() => {
  clearSelection();
  clearExpanded();
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const meta = (over: Partial<EntryMeta> = {}): EntryMeta => ({
  name: "f.txt",
  rel_path: ["f.txt"],
  kind: "file",
  size: 10,
  mtime: 1000,
  created: null,
  ...over,
});

const fileEntry = (over: Partial<DiffEntry> = {}): DiffEntry => ({
  rel_path: ["f.txt"],
  name: "f.txt",
  is_dir: false,
  status: "different",
  left: meta(),
  right: meta(),
  detail: null,
  children: [],
  ...over,
});

const pendingDir = (over: Partial<DiffEntry> = {}): DiffEntry => ({
  rel_path: ["sub"],
  name: "sub",
  is_dir: true,
  status: "skipped",
  left: meta({ name: "sub", rel_path: ["sub"], kind: "dir" }),
  right: meta({ name: "sub", rel_path: ["sub"], kind: "dir" }),
  detail: "not descended",
  children: [],
  ...over,
});

describe("DiffNode rendering", () => {
  it("renders a file name as-is and a directory name with a trailing slash", () => {
    const { container } = render(DiffNode, { entry: fileEntry() });
    expect(screen.getByText("f.txt")).toBeTruthy();
    expect(container.querySelector(".node")?.classList.contains("status-different")).toBe(true);
  });

  it("renders a directory name with '/'", () => {
    render(DiffNode, { entry: pendingDir() });
    expect(screen.getByText("sub/")).toBeTruthy();
  });

  it("uses resolvedStatus over the raw entry status for the row's status class", () => {
    // A pending dir (raw status 'skipped') resolved to 'identical' must paint as identical.
    const { container } = render(DiffNode, {
      entry: pendingDir(),
      resolvedStatus: "identical",
    });
    const node = container.querySelector(".node")!;
    expect(node.classList.contains("status-identical")).toBe(true);
    expect(node.classList.contains("status-skipped")).toBe(false);
  });

  it("shows a spinner while a pending dir is loading, a pending-box otherwise", () => {
    const { container: loadingC } = render(DiffNode, { entry: pendingDir(), loading: true });
    expect(loadingC.querySelector(".spinner")).toBeTruthy();
    expect(loadingC.querySelector(".pend-box")).toBeNull();

    const { container: idleC } = render(DiffNode, { entry: pendingDir(), loading: false });
    expect(idleC.querySelector(".spinner")).toBeNull();
    expect(idleC.querySelector(".pend-box")).toBeTruthy();
  });
});

describe("DiffNode interaction", () => {
  it("single-clicking an openable file calls onopen with the entry", async () => {
    const onopen = vi.fn();
    const entry = fileEntry();
    const { container } = render(DiffNode, { entry, onopen });
    await fireEvent.click(container.querySelector(".node")!, { detail: 1 });
    expect(onopen).toHaveBeenCalledTimes(1);
    expect(onopen).toHaveBeenCalledWith(entry);
  });

  it("double-clicking a file also calls onopen (single-click already opens, dblclick is a no-op)", async () => {
    const onopen = vi.fn();
    const entry = fileEntry();
    const { container } = render(DiffNode, { entry, onopen });
    // Simulate the two clicks a double-click generates: detail=1 (opens), detail=2 (ignored).
    await fireEvent.click(container.querySelector(".node")!, { detail: 1 });
    await fireEvent.click(container.querySelector(".node")!, { detail: 2 });
    await fireEvent.dblClick(container.querySelector(".node")!);
    expect(onopen).toHaveBeenCalledTimes(1); // opened once by the first click only
  });

  it("single-clicking a NON-openable file (one side only) does nothing", async () => {
    const onopen = vi.fn();
    const entry = pendingDir({ right: null, status: "left_only", detail: null });
    const { container } = render(DiffNode, { entry, onopen });
    await fireEvent.click(container.querySelector(".node")!, { detail: 1 });
    expect(onopen).not.toHaveBeenCalled();
  });

  it("expanding a pending directory calls onexpand once and flips the arrow ▶ → ▼", async () => {
    const onexpand = vi.fn();
    const entry = pendingDir();
    render(DiffNode, { entry, onexpand });

    const toggle = screen.getByLabelText("toggle");
    expect(toggle.textContent).toBe("▶");

    await fireEvent.click(toggle);
    expect(onexpand).toHaveBeenCalledTimes(1);
    expect(onexpand).toHaveBeenCalledWith(entry);
    expect(screen.getByLabelText("toggle").textContent).toBe("▼");
  });

  it("single-clicking a folder row expands it after 200ms timer", async () => {
    const onexpand = vi.fn();
    const entry = pendingDir();
    const { container } = render(DiffNode, { entry, onexpand });
    expect(screen.getByLabelText("toggle").textContent).toBe("▶");

    await fireEvent.click(container.querySelector(".node")!, { detail: 1 });
    // Timer not fired yet → no expand
    expect(onexpand).not.toHaveBeenCalled();

    vi.runAllTimers();
    await tick(); // let Svelte process the state update triggered by the timer
    expect(onexpand).toHaveBeenCalledWith(entry);
    expect(screen.getByLabelText("toggle").textContent).toBe("▼");
  });

  it("double-clicking a folder cancels the expand timer and drills in instead", async () => {
    const onopen = vi.fn();
    const onexpand = vi.fn();
    const entry = pendingDir();
    const { container } = render(DiffNode, { entry, onopen, onexpand });

    await fireEvent.click(container.querySelector(".node")!, { detail: 1 });
    await fireEvent.dblClick(container.querySelector(".node")!);
    vi.runAllTimers(); // timer was cancelled — should not fire

    expect(onopen).toHaveBeenCalledWith(entry);    // drilled in
    expect(onexpand).not.toHaveBeenCalled();        // expand never fired
  });

  it("the checkbox toggles the shared selection store", async () => {
    const entry = fileEntry();
    render(DiffNode, { entry });
    expect(selection.has(keyOf(entry))).toBe(false);

    await fireEvent.click(screen.getByLabelText("select f.txt"));
    expect(selection.has(keyOf(entry))).toBe(true);
  });
});
