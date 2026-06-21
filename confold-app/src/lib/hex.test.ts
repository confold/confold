import { describe, it, expect } from "vitest";
import { hexRows, formatOffset } from "./hex";

describe("hexRows", () => {
  it("flags the differing byte positionally", () => {
    const rows = hexRows([1, 2, 3], [1, 9, 3]);
    expect(rows).toHaveLength(1);
    expect(rows[0].offset).toBe(0);
    expect(rows[0].left[0].differ).toBe(false);
    expect(rows[0].left[1]).toMatchObject({ hex: "02", differ: true });
    expect(rows[0].right[1]).toMatchObject({ hex: "09", differ: true });
    expect(rows[0].left[2].differ).toBe(false);
  });

  it("marks bytes past one side's end as differing (length mismatch)", () => {
    const rows = hexRows([1], [1, 2]);
    expect(rows[0].left[1]).toMatchObject({ present: false, differ: false });
    expect(rows[0].right[1]).toMatchObject({ present: true, hex: "02", differ: true });
  });

  it("renders printable ascii and dots for control bytes", () => {
    const rows = hexRows([0x41, 0x00], [0x41, 0x00]);
    expect(rows[0].left[0].ch).toBe("A");
    expect(rows[0].left[1].ch).toBe(".");
  });

  it("splits into 16-byte rows", () => {
    const data = Array.from({ length: 20 }, (_, i) => i);
    const rows = hexRows(data, data);
    expect(rows).toHaveLength(2);
    expect(rows[1].offset).toBe(16);
    expect(rows[0].left).toHaveLength(16);
  });

  it("is empty for two empty files", () => {
    expect(hexRows([], [])).toEqual([]);
  });
});

describe("formatOffset", () => {
  it("zero-pads to 8 hex digits", () => {
    expect(formatOffset(0)).toBe("00000000");
    expect(formatOffset(16)).toBe("00000010");
    expect(formatOffset(255)).toBe("000000ff");
  });
});
