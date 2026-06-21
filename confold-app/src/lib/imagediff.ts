// Pure pixel-diff helpers for the image comparator (no DOM; tested in imagediff.test.ts via `pnpm test`).
// Operate on plain RGBA arrays so they run in node tests as well as on canvas ImageData in the browser.

export type Box = { x: number; y: number; w: number; h: number };

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "avif"]);

/** True if the path looks like a raster/vector image the comparator can render (by extension). */
export function isImagePath(path: string): boolean {
  const ext = path.toLowerCase().split(".").pop() ?? "";
  return IMAGE_EXTS.has(ext);
}

/** Per-pixel difference mask of two equally-sized RGBA buffers. A pixel differs when any channel's
 *  absolute delta exceeds `tolerance` (0..255). Returns a 0/1 mask (length w·h) and the differ count. */
export function pixelDiff(
  a: Uint8ClampedArray | number[],
  b: Uint8ClampedArray | number[],
  w: number,
  h: number,
  tolerance: number,
): { mask: Uint8Array; count: number } {
  const mask = new Uint8Array(w * h);
  let count = 0;
  for (let p = 0; p < w * h; p++) {
    const i = p * 4;
    const d = Math.max(
      Math.abs(a[i] - b[i]),
      Math.abs(a[i + 1] - b[i + 1]),
      Math.abs(a[i + 2] - b[i + 2]),
      Math.abs(a[i + 3] - b[i + 3]),
    );
    if (d > tolerance) {
      mask[p] = 1;
      count++;
    }
  }
  return { mask, count };
}

/** Cluster the differing pixels of a mask into bounding boxes (4-connected on a coarse cell grid), so
 *  the UI can step between change regions. Boxes are in pixel coords, ordered top→bottom, left→right. */
export function clusterBoxes(mask: Uint8Array, w: number, h: number, cell = 16): Box[] {
  if (w === 0 || h === 0) return [];
  const cols = Math.ceil(w / cell);
  const rows = Math.ceil(h / cell);
  const hot = new Uint8Array(cols * rows); // cell has ≥1 differing pixel
  for (let p = 0; p < mask.length; p++) {
    if (!mask[p]) continue;
    const px = p % w;
    const py = (p - px) / w;
    hot[Math.floor(py / cell) * cols + Math.floor(px / cell)] = 1;
  }
  const seen = new Uint8Array(cols * rows);
  const boxes: Box[] = [];
  const stack: number[] = [];
  for (let c = 0; c < cols * rows; c++) {
    if (!hot[c] || seen[c]) continue;
    let mincx = cols;
    let mincy = rows;
    let maxcx = 0;
    let maxcy = 0;
    stack.push(c);
    seen[c] = 1;
    while (stack.length) {
      const k = stack.pop() as number;
      const kx = k % cols;
      const ky = (k - kx) / cols;
      mincx = Math.min(mincx, kx);
      mincy = Math.min(mincy, ky);
      maxcx = Math.max(maxcx, kx);
      maxcy = Math.max(maxcy, ky);
      const nb = [
        kx > 0 ? k - 1 : -1,
        kx < cols - 1 ? k + 1 : -1,
        ky > 0 ? k - cols : -1,
        ky < rows - 1 ? k + cols : -1,
      ];
      for (const n of nb) {
        if (n >= 0 && hot[n] && !seen[n]) {
          seen[n] = 1;
          stack.push(n);
        }
      }
    }
    const x = mincx * cell;
    const y = mincy * cell;
    boxes.push({
      x,
      y,
      w: Math.min((maxcx + 1) * cell, w) - x,
      h: Math.min((maxcy + 1) * cell, h) - y,
    });
  }
  boxes.sort((p, q) => p.y - q.y || p.x - q.x);
  return boxes;
}
