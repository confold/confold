// Pure helpers for the binary hex compare (no DOM; tested in hex.test.ts via `pnpm test`).

/** One byte cell in a hex row. `present` = within this side's length; `differ` = positional mismatch. */
export type HexByte = { hex: string; ch: string; present: boolean; differ: boolean };

/** A 16-byte row aligned positionally across the two files. */
export type HexRow = { offset: number; left: HexByte[]; right: HexByte[] };

const BYTES_PER_ROW = 16;

function cell(present: boolean, byte: number, differ: boolean): HexByte {
  if (!present) return { hex: "  ", ch: " ", present: false, differ: false };
  return {
    hex: byte.toString(16).padStart(2, "0"),
    ch: byte >= 32 && byte < 127 ? String.fromCharCode(byte) : ".",
    present: true,
    differ,
  };
}

/** Build positionally-aligned hex rows for two byte arrays, flagging per-byte differences. */
export function hexRows(left: number[], right: number[]): HexRow[] {
  const len = Math.max(left.length, right.length);
  const rows: HexRow[] = [];
  for (let off = 0; off < len; off += BYTES_PER_ROW) {
    const l: HexByte[] = [];
    const r: HexByte[] = [];
    for (let i = 0; i < BYTES_PER_ROW; i++) {
      const idx = off + i;
      const lp = idx < left.length;
      const rp = idx < right.length;
      const differ = lp !== rp || (lp && rp && left[idx] !== right[idx]);
      l.push(cell(lp, left[idx], lp && differ));
      r.push(cell(rp, right[idx], rp && differ));
    }
    rows.push({ offset: off, left: l, right: r });
  }
  return rows;
}

/** Format a byte offset as a fixed-width hex address. */
export const formatOffset = (n: number): string => n.toString(16).padStart(8, "0");
