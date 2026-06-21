<script lang="ts">
  // Side-by-side hex compare for binary files: positional byte diff, virtualized like the text view.
  import { onMount } from "svelte";
  import { commands } from "$lib/commands";
  import type { HexCompare, FileRef } from "$lib/types";
  import { hexRows, formatOffset } from "$lib/hex";
  import FileViewHeader from "./FileViewHeader.svelte";

  let {
    left,
    right,
    identical,
    name = "",
    onback,
    readOnly = false,
    onkeep,
    onskip,
  }: {
    left: FileRef;
    right: FileRef;
    identical: boolean;
    name?: string;
    onback?: () => void;
    readOnly?: boolean;
    onkeep?: () => void;
    onskip?: () => void;
  } = $props();

  let data = $state<HexCompare | null>(null);
  let err = $state("");

  onMount(async () => {
    try {
      data = await commands.hexCompare(left, right);
    } catch (e) {
      err = String(e);
    }
  });

  const rows = $derived(data ? hexRows(data.left, data.right) : []);

  // Fixed-height virtualization (only viewport rows in the DOM).
  const ROW_H = 20;
  const OVERSCAN = 12;
  let scrollTop = $state(0);
  let viewportH = $state(480);
  const vStart = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const vEnd = $derived(Math.min(rows.length, vStart + Math.ceil(viewportH / ROW_H) + OVERSCAN * 2));
  const visible = $derived(rows.slice(vStart, vEnd).map((row, k) => ({ i: vStart + k, row })));

  // Difference navigation: group contiguous runs of differing BYTES (by offset) into regions, so ↑/↓
  // jump between distinct differing spans — not per-row (a file where every row has a differing byte
  // would otherwise collapse to a single region). Each region carries its byte offset span.
  const BYTES_PER_ROW = 16; // matches confold-textdiff/hex.ts
  const regions = $derived.by(() => {
    const out: { start: number; end: number }[] = []; // byte-offset spans
    for (const row of rows) {
      const n = Math.max(row.left.length, row.right.length);
      for (let k = 0; k < n; k++) {
        if (!(row.left[k]?.differ || row.right[k]?.differ)) continue;
        const off = row.offset + k;
        const last = out[out.length - 1];
        if (last && off === last.end + 1) last.end = off;
        else out.push({ start: off, end: off });
      }
    }
    return out;
  });
  let cur = $state(0);
  let hexEl = $state<HTMLElement | undefined>();
  // Keep `cur` valid if the region set changes (new file loaded).
  $effect(() => {
    if (cur > regions.length - 1) cur = Math.max(0, regions.length - 1);
  });
  // A row index is in the current region if the region's offset span touches that row.
  const inCurRegion = (i: number) => {
    const r = regions[cur];
    if (!r) return false;
    return i >= Math.floor(r.start / BYTES_PER_ROW) && i <= Math.floor(r.end / BYTES_PER_ROW);
  };
  function go(delta: number) {
    if (regions.length === 0) return;
    cur = Math.max(0, Math.min(regions.length - 1, cur + delta));
    const row = Math.floor(regions[cur].start / BYTES_PER_ROW);
    if (hexEl) hexEl.scrollTop = Math.max(0, row * ROW_H - viewportH / 2 + ROW_H);
  }
</script>

<div class="hexview">
  <FileViewHeader {name} {onback} {readOnly} {onkeep} {onskip} />
  <div class="hexbar">
    <span class="verdict" class:same={identical}>{identical ? "Binary — identical" : "Binary — they differ"}</span>
    {#if regions.length > 0}
      <span class="hexnav">
        <button class="ghost nav" onclick={() => go(-1)} disabled={cur === 0} aria-label="previous difference">↑</button>
        <button class="ghost nav" onclick={() => go(1)} disabled={cur >= regions.length - 1} aria-label="next difference">↓</button>
        <span class="navcount">{cur + 1} / {regions.length} diff{regions.length === 1 ? "" : "s"}</span>
      </span>
    {/if}
    {#if data}
      <span class="meta">
        left {data.left_len.toLocaleString()} B · right {data.right_len.toLocaleString()} B{data.truncated
          ? " · showing first 256 KB"
          : ""}
      </span>
    {/if}
    {#if err}<span class="err">{err}</span>{/if}
  </div>

  {#if rows.length === 0}
    <p class="empty">{err ? "" : "Both files are empty."}</p>
  {:else}
    <div
      class="hex"
      bind:this={hexEl}
      onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)}
      bind:clientHeight={viewportH}
    >
      <div class="hex-spacer" style="height: {rows.length * ROW_H}px">
        {#each visible as v (v.i)}
          <div class="hexrow" class:cur={inCurRegion(v.i)} style="top: {v.i * ROW_H}px">
            <span class="off">{formatOffset(v.row.offset)}</span>
            <span class="side">
              <span class="bytes">{#each v.row.left as b}<span class="b" class:hd={b.differ}>{b.hex}</span>{/each}</span>
              <span class="ascii">{#each v.row.left as b}<span class:hd={b.differ}>{b.ch}</span>{/each}</span>
            </span>
            <span class="side">
              <span class="bytes">{#each v.row.right as b}<span class="b" class:hd={b.differ}>{b.hex}</span>{/each}</span>
              <span class="ascii">{#each v.row.right as b}<span class:hd={b.differ}>{b.ch}</span>{/each}</span>
            </span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  /* Fill the flex slot given by the parent so the hex area uses all available vertical space. */
  .hexview {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .hexbar {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
    padding: 0.4rem 0;
    font-size: 0.85rem;
    flex: none;
  }
  .verdict {
    font-weight: 600;
    color: #e5484d;
  }
  .verdict.same {
    color: #2bb3a3;
  }
  .hexnav {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }
  .hexnav .nav {
    border: 1px solid #ccc;
    background: transparent;
    color: inherit;
    border-radius: 6px;
    padding: 0.05em 0.45em;
    font-size: 0.8rem;
    cursor: pointer;
    line-height: 1.2;
  }
  .hexnav .nav:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .navcount {
    font-size: 0.78rem;
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
  }
  .meta {
    opacity: 0.65;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }
  .err {
    color: #e5484d;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }
  .hex {
    position: relative;
    border: 1px solid #ddd;
    border-radius: 8px;
    overflow: auto;
    background: #fff;
    flex: 1 1 auto;
    min-height: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.8rem;
  }
  .hex-spacer {
    position: relative;
    width: 100%;
  }
  .hexrow {
    position: absolute;
    left: 0;
    height: 20px;
    line-height: 20px;
    display: flex;
    gap: 1.4rem;
    padding: 0 0.6rem;
    white-space: pre;
    box-sizing: border-box;
  }
  /* The currently-navigated difference region. */
  .hexrow.cur {
    background: rgba(229, 72, 77, 0.1);
    border-left: 2px solid #e5484d;
    padding-left: calc(0.6rem - 2px);
  }
  .off {
    color: #999;
    user-select: none;
  }
  .side {
    display: flex;
    gap: 0.9rem;
  }
  .b {
    padding-right: 0.4ch;
  }
  .ascii {
    opacity: 0.85;
  }
  .hd {
    background: rgba(229, 72, 77, 0.45);
    border-radius: 2px;
  }
  .empty {
    padding: 1rem;
    opacity: 0.6;
  }
  @media (prefers-color-scheme: dark) {
    .hex {
      background: #232323;
      border-color: #3a3a3a;
    }
  }
</style>
