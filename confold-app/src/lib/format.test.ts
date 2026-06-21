import { describe, it, expect } from "vitest";
import { fmtDate } from "./format";

describe("fmtDate", () => {
  it("returns empty for null/undefined", () => {
    expect(fmtDate(null)).toBe("");
    expect(fmtDate(undefined)).toBe("");
  });

  it("formats epoch ms as YYYY-MM-DD HH:mm (TZ-independent shape)", () => {
    expect(fmtDate(0)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    expect(fmtDate(1718200000000)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
  });
});
