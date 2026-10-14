import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import MigrateProgressModal from "./MigrateProgressModal.svelte";
import type { MigrateOutcome } from "$lib/migrate";

const BASE = {
  outcomes: [] as MigrateOutcome[],
  applying: false,
  doneSummary: { total: 1, ok: 1, failed: 0, cancelled: false },
  moveSummary: null,
  phase: "" as "" | "verifying" | "emptying_origin",
  originSpec: null,
  destSpec: null,
  checkedTotal: 0,
  oncancel: () => {},
  oncontinue: () => {},
};

describe("MigrateProgressModal", () => {
  it("titles the run after the operation that started it (copy, not always Migration)", () => {
    render(MigrateProgressModal, { ...BASE, noun: "Copy" });
    expect(screen.getByText("Copy complete ✓")).toBeTruthy();
  });

  it("keeps the Migration title by default", () => {
    render(MigrateProgressModal, { ...BASE });
    expect(screen.getByText("Migration complete ✓")).toBeTruthy();
  });

  it("exposes the failing operation's error as a tooltip on the whole row", () => {
    const { container } = render(MigrateProgressModal, {
      ...BASE,
      doneSummary: { total: 2, ok: 1, failed: 1, cancelled: false },
      outcomes: [
        {
          path: "2014 - Presupuestos/informe.pdf",
          op: "copy_left_to_right",
          reason: "new",
          ok: true,
        },
        {
          path: "2014 - Presupuestos/azul.pdf",
          op: "copy_left_to_right",
          reason: "new",
          ok: false,
          error: "Nuxeo ranged read: HTTP 404",
        },
      ] as MigrateOutcome[],
    });
    expect(screen.getByText("Migration completed with errors")).toBeTruthy();
    const failedRow = container.querySelector(".mig-fail");
    if (!failedRow) throw new Error("failed row did not render");
    expect(failedRow.getAttribute("title")).toBe("Nuxeo ranged read: HTTP 404");
    const okRow = container.querySelector(".mig-ok");
    expect(okRow?.getAttribute("title")).toBeNull();
  });
});
