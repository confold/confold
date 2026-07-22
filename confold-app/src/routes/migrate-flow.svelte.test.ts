// @vitest-environment jsdom
//
// End-to-end-ish safety net for the Migrate wizard, driven through the real `+page.svelte` with Tauri
// mocked. It asserts behaviour that MUST survive the planned god-component split (C1-full): config-panel
// gating, the per-item plan checkboxes → only-checked-applied, the empty plan, and the streamed progress
// (skipped pre-population + live outcomes + done summary). Written against rendered behaviour (not
// internals) so extracting <MigrateConfigPanel>/<MigratePlanModal>/<MigrateProgressModal> can't silently
// regress it.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";

const { invokeMock, listenMock, PLAN, ctx } = vi.hoisted(() => {
  const PLAN = [
    { rel_path: ["new.txt"], op: "copy_left_to_right", is_dir: false, reason: "new", item_count: 1 },
    { rel_path: ["diff.txt"], op: "copy_left_to_right", is_dir: false, reason: "different", item_count: 1 },
    { rel_path: ["extra.txt"], op: "delete_right", is_dir: false, reason: "extra", item_count: 1 },
  ];
  // `ctx.plan` lets a test swap what the mocked `migrate_actions` returns; `ctx.listeners` captures the
  // Tauri event callbacks so a test can simulate `migrate-progress` / `migrate-done`.
  const ctx: {
    plan: typeof PLAN;
    listeners: Map<string, (e: { payload: unknown }) => void>;
    diffFile: unknown; // when set, `diff_file` resolves to this (else stays pending → loading skeleton)
  } = { plan: PLAN, listeners: new Map(), diffFile: undefined };
  const invokeMock = vi.fn((cmd: string, _args?: Record<string, unknown>) => {
    switch (cmd) {
      case "source_types":
        return Promise.resolve([]);
      case "load_recents":
        return Promise.resolve({ origins: [], destinations: [] });
      case "migrate_actions": {
        // The backend no longer returns the plan; it streams it via a `plan-ready` event tagged with the
        // token it was given. Echo the captured plan back through that listener (matching the token).
        const token = (_args as { token?: number })?.token ?? 0;
        queueMicrotask(() =>
          ctx.listeners
            .get("plan-ready")
            ?.({ payload: { token, flow: "migrate", actions: ctx.plan, error: null } }),
        );
        return Promise.resolve();
      }
      case "migrate_apply":
        return Promise.resolve();
      case "diff_file":
        return ctx.diffFile !== undefined ? Promise.resolve(ctx.diffFile) : new Promise(() => {});
      case "diff_file_large": {
        const m = { fp: "", eol: "\n", final_nl: false, mtime: null, created: null };
        const drow = { left_no: 1, right_no: 1, kind: "replace", left: "old", right: "new", left_words: [], right_words: [], left_words_w: [], right_words_w: [] };
        return Promise.resolve({
          kind: "text_hunks",
          hunks: { hunks: [{ left_start: 1, right_start: 1, rows: [drow] }], summary: { equal: 0, inserted: 0, deleted: 0, replaced: 1 }, is_complete: true, total_hunks: 1, next_hunk_index: null },
          left: m, right: m, left_size: 3_400_000, right_size: 3_500_000,
        });
      }
      default:
        return Promise.resolve(undefined);
    }
  });
  const listenMock = vi.fn((event: string, cb: (e: { payload: unknown }) => void) => {
    ctx.listeners.set(event, cb);
    return Promise.resolve(() => {});
  });
  return { invokeMock, listenMock, PLAN, ctx };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ close: vi.fn() }) }));

import Page from "./+page.svelte";

const ORIGIN = "/data/origin";
const DEST = "/data/dest";

function seedRecents() {
  localStorage.setItem(
    "confold-origin-recents",
    JSON.stringify([{ spec: { kind: "fs", fields: { root: ORIGIN } }, isDir: true }]),
  );
  localStorage.setItem(
    "confold-dest-recents",
    JSON.stringify([{ spec: { kind: "fs", fields: { root: DEST } }, isDir: true }]),
  );
}

beforeEach(() => {
  invokeMock.mockClear();
  ctx.plan = PLAN;
  ctx.diffFile = undefined;
  ctx.listeners.clear();
  localStorage.clear();
  seedRecents();
});

// Simulate a backend Tauri event reaching the page's listener.
function emit(event: string, payload: unknown) {
  ctx.listeners.get(event)?.({ payload });
}

const boxes = () =>
  document.querySelectorAll<HTMLInputElement>(".plan-check-col input[type=checkbox]");

// Render → pick both folder sources → switch to Migrate → open the config panel.
async function gotoConfig() {
  render(Page);
  await fireEvent.click(await screen.findByTitle(ORIGIN));
  await fireEvent.click(await screen.findByTitle(DEST));
  await fireEvent.click(screen.getByLabelText("Select mode"));
  const menuItem = screen
    .getAllByRole("button")
    .find((b) => b.classList.contains("mode-item") && b.textContent?.includes("Migrate"))!;
  await fireEvent.click(menuItem);
  await fireEvent.click(document.querySelector(".split-main")!);
  await screen.findByText("Preview migration");
}

// Config panel → ack the backup → preview → wait for the plan modal.
async function openPlan() {
  await gotoConfig();
  await fireEvent.click(screen.getByLabelText(/I have a backup/));
  await fireEvent.click(screen.getByText("Preview migration"));
  await screen.findByText("Migration plan");
}

// Like openPlan, but turns the M2 "Move — delete origin" toggle on first.
async function openPlanWithMove() {
  await gotoConfig();
  await fireEvent.click(screen.getByLabelText(/delete the origin after copying/i));
  await fireEvent.click(screen.getByLabelText(/I have a backup/));
  await fireEvent.click(screen.getByText("Preview migration"));
  await screen.findByText("Migration plan");
}

const applyArgs = () =>
  invokeMock.mock.calls.find((c) => c[0] === "migrate_apply")?.[1] as
    | { deleteOrigin: boolean; opts: unknown }
    | undefined;

describe("Migrate config panel", () => {
  it("gates Preview behind the backup acknowledgement when a destructive flag is on", async () => {
    await gotoConfig();
    const preview = screen.getByText("Preview migration") as HTMLButtonElement;
    // Default flags include "overwrite different" (destructive) → disabled until the box is ticked.
    expect(preview.disabled).toBe(true);
    await fireEvent.click(screen.getByLabelText(/I have a backup/));
    expect(preview.disabled).toBe(false);
  });
});

describe("Migrate plan wizard", () => {
  it("renders one pre-checked checkbox per planned action", async () => {
    await openPlan();
    expect(boxes()).toHaveLength(PLAN.length);
    expect([...boxes()].every((b) => b.checked)).toBe(true);
  });

  it("shows per-category item counts", async () => {
    await openPlan();
    const counts = document.querySelector(".plan-counts")!.textContent ?? "";
    expect(counts).toContain("+1 new");
    expect(counts).toContain("1 override");
    expect(counts).toContain("1 delete");
  });

  it("an empty plan shows 'already matches' and offers no apply button", async () => {
    ctx.plan = [];
    await openPlan();
    expect(screen.getByText(/already matches/i)).toBeTruthy();
    expect(screen.queryByText("Migrate →")).toBeNull();
  });

  it("applies only the checked actions — unchecking one drops it from the backend call", async () => {
    await openPlan();
    await fireEvent.click(boxes()[0]); // uncheck new.txt
    await fireEvent.click(screen.getByText("Migrate →"));

    const applyCall = invokeMock.mock.calls.find((c) => c[0] === "migrate_apply");
    expect(applyCall).toBeTruthy();
    const sentPaths = (applyCall![1] as { actions: { rel_path: string[] }[] }).actions.map((a) =>
      a.rel_path.join("/"),
    );
    expect(sentPaths).toEqual(["diff.txt", "extra.txt"]);
    expect(sentPaths).not.toContain("new.txt");
  });
});

describe("Migrate progress view", () => {
  it("pre-populates unchecked items as skipped", async () => {
    await openPlan();
    await fireEvent.click(boxes()[0]); // uncheck new.txt
    await fireEvent.click(screen.getByText("Migrate →"));

    const skipped = document.querySelectorAll(".mig-skip");
    expect(skipped).toHaveLength(1);
    expect(skipped[0].textContent).toContain("new.txt");
  });

  it("renders streamed outcomes and the final done summary", async () => {
    await openPlan(); // all 3 checked
    await fireEvent.click(screen.getByText("Migrate →"));

    // First apply → generation 1. Stream one applied outcome, then completion.
    emit("migrate-progress", {
      generation: 1,
      rel_path: ["new.txt"],
      op: "copy_left_to_right",
      reason: "new",
      ok: true,
      error: null,
    });
    await vi.waitFor(() =>
      expect(document.querySelector(".mig-progress-list")?.textContent).toContain("new.txt"),
    );

    emit("migrate-done", {
      generation: 1,
      summary: { total: 3, ok: 3, failed: 0, cancelled: false },
    });
    await tick();
    // Done: the apply is finished, so the "Continue → verify comparison" button is offered.
    await screen.findByText(/Continue/);
  });
});

describe("Migrate move (M2)", () => {
  it("requests the origin-delete when the move is on and the plan is intact", async () => {
    await openPlanWithMove();
    await fireEvent.click(screen.getByText("Migrate →"));
    expect(applyArgs()?.deleteOrigin).toBe(true);
    // The exclusions must travel with it (the backend re-verify must honour them).
    expect(applyArgs()?.opts).toBeTruthy();
  });

  it("voids the move (and warns) when any plan item is unchecked", async () => {
    await openPlanWithMove();
    await fireEvent.click(boxes()[0]); // uncheck one → the plan was modified
    expect(screen.getByText(/Origin-delete skipped/i)).toBeTruthy();
    await fireEvent.click(screen.getByText("Migrate →"));
    expect(applyArgs()?.deleteOrigin).toBe(false);
  });

  it("marks origin rows as 'delete' when Move is on, reverting when an item is unchecked", async () => {
    await openPlanWithMove();
    // ORIG is the first `.plan-cell` per row; count rows whose origin will be deleted.
    const origDeleteCount = () =>
      [...document.querySelectorAll(".plan-row")]
        .map((r) => r.querySelectorAll(".plan-cell")[0]?.textContent ?? "")
        .filter((t) => t.includes("delete")).length;
    // All checked + move on → the two origin-present rows (new.txt, diff.txt) show "✕ delete"
    // (extra.txt isn't in origin → "—"). All-or-nothing, so it's global.
    expect(origDeleteCount()).toBe(2);
    await fireEvent.click(boxes()[0]); // uncheck one → move voided → every origin reverts to "no change"
    expect(origDeleteCount()).toBe(0);
  });
});

describe("Migrate plan navigation", () => {
  it("opens a 'different' file ON TOP (closing the plan) and returns to it on back", async () => {
    await openPlan();
    const diffRow = [...document.querySelectorAll(".plan-row")].find((r) =>
      r.querySelector(".plan-path")?.textContent?.startsWith("diff.txt"),
    )!;
    await fireEvent.click(diffRow.querySelector(".plan-path")!);

    expect(screen.queryByText("Migration plan")).toBeNull();
    expect(screen.getByLabelText("back")).toBeTruthy();
    expect(screen.getByText("diff.txt")).toBeTruthy();

    await fireEvent.click(screen.getByLabelText("back"));
    expect(await screen.findByText("Migration plan")).toBeTruthy();
  });

  it("routes a too-large plan item through the large-file flow (regression: blank window)", async () => {
    // Opening a >cap file FROM THE PLAN must show the large-file warning, not leave a blank main
    // (the bug: openFileFromPlan set result.kind="too_large", which matches no view branch → black).
    const meta = { fp: "", eol: "\n", final_nl: false, mtime: null, created: null };
    ctx.plan = [{ rel_path: ["big.log"], op: "copy_left_to_right", is_dir: false, reason: "different", item_count: 1 }];
    ctx.diffFile = { result: { kind: "too_large", left_size: 3_400_000, right_size: 3_500_000 }, left: meta, right: meta };
    await openPlan();
    const row = [...document.querySelectorAll(".plan-row")].find((r) =>
      r.querySelector(".plan-path")?.textContent?.startsWith("big.log"),
    )!;
    await fireEvent.click(row.querySelector(".plan-path")!);

    // The large-file warning appears (not a blank window) …
    expect(await screen.findByText("Large file")).toBeTruthy();
    // … and confirming renders the hunks view with the plan-review OK/Skip still available.
    await fireEvent.click(screen.getByText("Show differences"));
    expect(await screen.findByText("OK")).toBeTruthy();
    expect(screen.getByText("✕ Skip")).toBeTruthy();
  });

  it("offers OK/Skip when reviewing a BINARY plan item, and Skip unchecks it", async () => {
    // A binary diff opens the hex/binary view (not SideBySide); plan-review OK/Skip must still be there.
    const meta = { fp: "", eol: "\n", final_nl: false, mtime: null, created: null };
    ctx.diffFile = { result: { kind: "binary", identical: false }, left: meta, right: meta };
    await openPlan();
    const row = [...document.querySelectorAll(".plan-row")].find((r) =>
      r.querySelector(".plan-path")?.textContent?.startsWith("diff.txt"),
    )!;
    await fireEvent.click(row.querySelector(".plan-path")!);

    // Binary review view is up, with the plan-review controls.
    const skip = await screen.findByText("✕ Skip");
    expect(screen.getByText("OK")).toBeTruthy();
    expect(screen.queryByText("Migration plan")).toBeNull();

    // Skip → back to the plan with diff.txt now unchecked.
    await fireEvent.click(skip);
    await screen.findByText("Migration plan");
    const diffBox = [...document.querySelectorAll(".plan-row")]
      .find((r) => r.querySelector(".plan-path")?.textContent?.startsWith("diff.txt"))
      ?.querySelector<HTMLInputElement>("input[type=checkbox]");
    expect(diffBox?.checked).toBe(false);
  });

  it("reports 'origin moved' when the backend deletes the origin", async () => {
    await openPlanWithMove();
    await fireEvent.click(screen.getByText("Migrate →"));
    emit("migrate-done", {
      generation: 1,
      summary: { total: 3, ok: 3, failed: 0, cancelled: false },
      move_result: {
        attempted: true,
        origin_deleted: true,
        files_deleted: 3,
        dirs_pruned: 1,
        failed: 0,
        cancelled: false,
        blockers: [],
      },
    });
    await tick();
    // Both the header and the result block say "moved"; assert the result detail (file count) too.
    expect(screen.getAllByText(/origin moved/i).length).toBeGreaterThan(0);
    expect(screen.getByText(/Deleted 3 files and pruned 1 empty folder/i)).toBeTruthy();
  });

  it("reports 'origin kept' with blockers when the move is aborted", async () => {
    await openPlanWithMove();
    await fireEvent.click(screen.getByText("Migrate →"));
    emit("migrate-done", {
      generation: 1,
      summary: { total: 3, ok: 2, failed: 1, cancelled: false },
      move_result: {
        attempted: true,
        origin_deleted: false,
        files_deleted: 0,
        dirs_pruned: 0,
        failed: 0,
        cancelled: false,
        blockers: ["orphan.txt — only in origin (not copied)"],
      },
    });
    await tick();
    expect(screen.getByText(/Origin kept/i)).toBeTruthy();
    expect(screen.getByText(/orphan\.txt/)).toBeTruthy();
  });
});
