// @vitest-environment jsdom
//
// End-to-end-ish safety net for the Sync wizard, driven through the real `+page.svelte` with Tauri
// mocked. Mirrors migrate-flow: config-panel gating (trust + backup ack), the bidirectional plan with
// per-row directions, and that applying routes through `migrate_apply` with `delete_origin: false`
// (Sync never moves).
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";

const { invokeMock, listenMock, PLAN, ctx } = vi.hoisted(() => {
  // A bidirectional plan: a left-only copy (→), a right-only copy (←), and a resolved conflict (→).
  const PLAN = [
    { rel_path: ["only_left.txt"], op: "copy_left_to_right", is_dir: false, reason: "new", item_count: 1 },
    { rel_path: ["only_right.txt"], op: "copy_right_to_left", is_dir: false, reason: "new", item_count: 1 },
    { rel_path: ["conflict.txt"], op: "copy_left_to_right", is_dir: false, reason: "different", item_count: 1 },
  ];
  const ctx: { plan: typeof PLAN; listeners: Map<string, (e: { payload: unknown }) => void> } = {
    plan: PLAN,
    listeners: new Map(),
  };
  const invokeMock = vi.fn((cmd: string, _args?: Record<string, unknown>) => {
    switch (cmd) {
      case "source_types":
        return Promise.resolve([]);
      case "sync_actions": {
        // Backend streams the plan via a `plan-ready` event (tagged with the token) instead of returning it.
        const token = (_args as { token?: number })?.token ?? 0;
        queueMicrotask(() =>
          ctx.listeners
            .get("plan-ready")
            ?.({ payload: { token, flow: "sync", actions: ctx.plan, error: null } }),
        );
        return Promise.resolve();
      }
      case "migrate_apply":
        return Promise.resolve();
      case "diff_file":
        return new Promise(() => {}); // never resolves → the file view stays in its loading skeleton
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
  localStorage.setItem("confold-origin-recents", JSON.stringify([{ spec: { kind: "fs", fields: { root: ORIGIN } }, isDir: true }]));
  localStorage.setItem("confold-dest-recents", JSON.stringify([{ spec: { kind: "fs", fields: { root: DEST } }, isDir: true }]));
}

beforeEach(() => {
  invokeMock.mockClear();
  ctx.plan = PLAN;
  ctx.listeners.clear();
  localStorage.clear();
  seedRecents();
});

// Trust/delete checkboxes live in the config panel's `.mig-ops` fieldset, in order.
const trustBoxes = () => document.querySelectorAll<HTMLInputElement>(".mig-ops input[type=checkbox]");
const applyArgs = () =>
  invokeMock.mock.calls.find((c) => c[0] === "migrate_apply")?.[1] as
    | { deleteOrigin: boolean }
    | undefined;

// Render → pick both sources → switch to Sync → open the config panel.
async function gotoSyncConfig() {
  render(Page);
  await fireEvent.click(await screen.findByTitle(ORIGIN));
  await fireEvent.click(await screen.findByTitle(DEST));
  await fireEvent.click(screen.getByLabelText("Select mode"));
  const item = screen
    .getAllByRole("button")
    .find((b) => b.classList.contains("mode-item") && b.textContent?.includes("Sync"))!;
  await fireEvent.click(item);
  await fireEvent.click(document.querySelector(".split-main")!);
  await screen.findByText("Preview sync");
}

// Config (both sides trusted by default) → ack backup → preview → wait for the plan.
async function openSyncPlan() {
  await gotoSyncConfig();
  await fireEvent.click(screen.getByLabelText(/I have a backup/));
  await fireEvent.click(screen.getByText("Preview sync"));
  await screen.findByText("Sync plan");
}

describe("Sync config panel", () => {
  it("gates Preview behind the backup acknowledgement", async () => {
    await gotoSyncConfig();
    const preview = screen.getByText("Preview sync") as HTMLButtonElement;
    expect(preview.disabled).toBe(true); // both trusted by default → destructive → needs the ack
    await fireEvent.click(screen.getByLabelText(/I have a backup/));
    expect(preview.disabled).toBe(false);
  });

  it("requires at least one trusted side", async () => {
    await gotoSyncConfig();
    await fireEvent.click(screen.getByLabelText(/I have a backup/));
    await fireEvent.click(trustBoxes()[0]); // untrust left
    await fireEvent.click(trustBoxes()[1]); // untrust right → none trusted
    expect((screen.getByText("Preview sync") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/at least one trusted side/i)).toBeTruthy();
  });

  it("offers 'Delete differences' only when exactly one side is trusted", async () => {
    await gotoSyncConfig();
    // Both trusted → no delete option.
    expect(screen.queryByText(/Delete differences/i)).toBeNull();
    await fireEvent.click(trustBoxes()[1]); // untrust right → exactly one trusted
    expect(screen.getByText(/Delete differences/i)).toBeTruthy();
  });
});

describe("Sync plan wizard", () => {
  it("puts the action on the written side (two-column) and applies without moving", async () => {
    await openSyncPlan();
    // Two columns per row: [left, right]. The written side shows the action; the source shows "no change".
    const rows = [...document.querySelectorAll(".plan-row")].map((r) => {
      const c = r.querySelectorAll(".plan-cell");
      return {
        path: r.querySelector(".plan-path")?.textContent ?? "",
        left: c[0]?.textContent ?? "",
        right: c[1]?.textContent ?? "",
      };
    });
    const byPath = (p: string) => rows.find((r) => r.path.startsWith(p))!;
    // left-only → written on the RIGHT; source (left) unchanged.
    expect(byPath("only_left.txt").right).toContain("+ copy");
    expect(byPath("only_left.txt").left).toContain("no change");
    // right-only → written on the LEFT (the key fix); source (right) unchanged.
    expect(byPath("only_right.txt").left).toContain("+ copy");
    expect(byPath("only_right.txt").right).toContain("no change");
    // conflict (left wins here) → override lands on the right.
    expect(byPath("conflict.txt").right).toContain("override");

    await fireEvent.click(screen.getByText("Sync →"));
    expect(invokeMock.mock.calls.some((c) => c[0] === "migrate_apply")).toBe(true);
    expect(applyArgs()?.deleteOrigin).toBe(false); // Sync never moves
  });

  it("opens a conflict ON TOP (closing the plan) and returns to it on back", async () => {
    // Regression for the bug where the diff opened *behind* the Sync plan modal.
    await openSyncPlan();
    const conflictRow = [...document.querySelectorAll(".plan-row")].find((r) =>
      r.querySelector(".plan-path")?.textContent?.startsWith("conflict.txt"),
    )!;
    await fireEvent.click(conflictRow.querySelector(".plan-path")!);

    // The Sync plan is gone and the file view (its loading skeleton) is shown in front.
    expect(screen.queryByText("Sync plan")).toBeNull();
    expect(screen.getByLabelText("back")).toBeTruthy();
    expect(screen.getByText("conflict.txt")).toBeTruthy();

    // Back → returns to the Sync plan (not the Migrate plan), via activeFlow.
    await fireEvent.click(screen.getByLabelText("back"));
    expect(await screen.findByText("Sync plan")).toBeTruthy();
  });
});
