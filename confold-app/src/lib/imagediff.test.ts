import { describe, it, expect } from "vitest";
import { pixelDiff, clusterBoxes, isImagePath } from "./imagediff";

describe("isImagePath", () => {
  it("recognises common image extensions, case-insensitive", () => {
    expect(isImagePath("/a/b/logo.PNG")).toBe(true);
    expect(isImagePath("photo.jpeg")).toBe(true);
    expect(isImagePath("icon.svg")).toBe(true);
    expect(isImagePath("notes.txt")).toBe(false);
    expect(isImagePath("archive")).toBe(false);
  });
});

// Helper: build an RGBA buffer for w*h from a per-pixel [r,g,b,a] list.
const rgba = (px: number[][]): number[] => px.flat();

describe("pixelDiff", () => {
  it("flags pixels whose channel delta exceeds the tolerance", () => {
    const a = rgba([
      [0, 0, 0, 255],
      [10, 10, 10, 255],
    ]);
    const b = rgba([
      [0, 0, 0, 255],
      [200, 10, 10, 255],
    ]);
    const { mask, count } = pixelDiff(a, b, 2, 1, 10);
    expect(count).toBe(1);
    expect(Array.from(mask)).toEqual([0, 1]);
  });

  it("ignores differences within the tolerance", () => {
    const a = rgba([[100, 100, 100, 255]]);
    const b = rgba([[108, 100, 100, 255]]);
    expect(pixelDiff(a, b, 1, 1, 10).count).toBe(0); // delta 8 ≤ 10
  });

  it("counts alpha differences too", () => {
    const a = rgba([[0, 0, 0, 0]]);
    const b = rgba([[0, 0, 0, 255]]);
    expect(pixelDiff(a, b, 1, 1, 0).count).toBe(1);
  });
});

describe("clusterBoxes", () => {
  it("returns no boxes when nothing differs", () => {
    expect(clusterBoxes(new Uint8Array(64), 8, 8, 4)).toEqual([]);
  });

  it("wraps a single differing pixel in one cell-sized box", () => {
    const w = 8;
    const h = 8;
    const mask = new Uint8Array(w * h);
    mask[0] = 1; // top-left pixel
    const boxes = clusterBoxes(mask, w, h, 4);
    expect(boxes).toEqual([{ x: 0, y: 0, w: 4, h: 4 }]);
  });

  it("keeps three well-separated regions distinct (the photo.bmp demo: gaps > the 16px cell)", () => {
    const w = 96;
    const h = 96;
    const mask = new Uint8Array(w * h);
    const fill = (x0: number, x1: number, y0: number, y1: number) => {
      for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) mask[y * w + x] = 1;
    };
    fill(8, 26, 8, 26);
    fill(58, 80, 8, 26);
    fill(32, 54, 60, 82);
    expect(clusterBoxes(mask, w, h, 16)).toHaveLength(3);
  });

  it("separates two distant differing regions into two boxes", () => {
    const w = 16;
    const h = 16;
    const mask = new Uint8Array(w * h);
    mask[0] = 1; // top-left
    mask[15 * w + 15] = 1; // bottom-right
    const boxes = clusterBoxes(mask, w, h, 4);
    expect(boxes).toHaveLength(2);
    expect(boxes[0]).toMatchObject({ x: 0, y: 0 });
    expect(boxes[1].x).toBeGreaterThan(0);
  });
});
