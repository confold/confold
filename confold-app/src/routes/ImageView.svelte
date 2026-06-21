<script lang="ts">
  // Image comparator: side-by-side / overlay / swipe / blink / difference, with a colour tolerance,
  // zoom/fit, metadata + % different, and navigation between change regions. Images are loaded as
  // same-origin blobs (from read_bytes) so the canvas isn't tainted and pixel-diff works.
  import { commands } from "$lib/commands";
  import { onMount, onDestroy } from "svelte";
  import { pixelDiff, clusterBoxes, type Box } from "$lib/imagediff";
  import type { FileRef } from "$lib/types";
  import FileViewHeader from "./FileViewHeader.svelte";

  let {
    left,
    right,
    name = "",
    onback,
    readOnly = false,
    onkeep,
    onskip,
  }: {
    left: FileRef;
    right: FileRef;
    name?: string;
    onback?: () => void;
    readOnly?: boolean;
    onkeep?: () => void;
    onskip?: () => void;
  } = $props();

  type Mode = "side" | "overlay" | "swipe" | "blink" | "diff";
  let mode = $state<Mode>("side");
  let tolerance = $state(16);
  let gridDiv = $state(12); // change regions ≈ this many cells across the image (the "1/x" granularity); user-tunable, min cell clamped to 2px
  let opacity = $state(0.5);
  let swipePos = $state(0.5);
  let zoom = $state(1);
  let err = $state("");

  let leftUrl = $state("");
  let rightUrl = $state("");
  let lw = $state(0);
  let lh = $state(0);
  let rw = $state(0);
  let rh = $state(0);
  let leftSize = $state(0);
  let rightSize = $state(0);
  let leftBmp: ImageBitmap | undefined;
  let rightBmp: ImageBitmap | undefined;

  let diffData: ImageData | null = $state(null);
  let diffW = $state(0);
  let diffH = $state(0);
  let diffPct = $state(0);
  let boxes = $state<Box[]>([]);
  let curBox = $state(-1);

  let stageEl: HTMLDivElement | undefined = $state();
  let diffCanvas: HTMLCanvasElement | undefined = $state();

  const W = $derived(Math.max(lw, rw));
  const H = $derived(Math.max(lh, rh));

  function mime(path: string): string {
    const ext = path.toLowerCase().split(".").pop() ?? "";
    const map: Record<string, string> = {
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      gif: "image/gif",
      webp: "image/webp",
      bmp: "image/bmp",
      svg: "image/svg+xml",
      ico: "image/x-icon",
      avif: "image/avif",
    };
    return map[ext] ?? "application/octet-stream";
  }

  async function load(file: FileRef): Promise<{ url: string; bmp: ImageBitmap; size: number }> {
    const buf = await commands.readBytes(file);
    const blob = new Blob([buf], { type: mime(file.rel) });
    const bmp = await createImageBitmap(blob);
    return { url: URL.createObjectURL(blob), bmp, size: buf.byteLength };
  }

  onMount(async () => {
    try {
      const [l, r] = await Promise.all([load(left), load(right)]);
      leftUrl = l.url;
      rightUrl = r.url;
      leftBmp = l.bmp;
      rightBmp = r.bmp;
      lw = l.bmp.width;
      lh = l.bmp.height;
      rw = r.bmp.width;
      rh = r.bmp.height;
      leftSize = l.size;
      rightSize = r.size;
      computeDiff();
      fit();
    } catch (e) {
      err = String(e);
    }
  });

  onDestroy(() => {
    if (leftUrl) URL.revokeObjectURL(leftUrl);
    if (rightUrl) URL.revokeObjectURL(rightUrl);
  });

  function rgbaOf(bmp: ImageBitmap, dw: number, dh: number, scale: number): Uint8ClampedArray {
    const c = document.createElement("canvas");
    c.width = dw;
    c.height = dh;
    const ctx = c.getContext("2d")!;
    ctx.drawImage(bmp, 0, 0, Math.round(bmp.width * scale), Math.round(bmp.height * scale));
    return ctx.getImageData(0, 0, dw, dh).data;
  }

  // Last diff result kept so the grid-size control can re-cluster without re-running the pixel diff.
  let lastMask: Uint8Array | null = null;
  let dscale = 1;

  // (Re)compute the pixel diff at a capped resolution, then cluster into regions.
  function computeDiff() {
    if (!leftBmp || !rightBmp || W === 0 || H === 0) return;
    const scale = Math.min(1, Math.sqrt(4_000_000 / (W * H)));
    const dw = Math.max(1, Math.round(W * scale));
    const dh = Math.max(1, Math.round(H * scale));
    const a = rgbaOf(leftBmp, dw, dh, scale);
    const b = rgbaOf(rightBmp, dw, dh, scale);
    const { mask, count } = pixelDiff(a, b, dw, dh, tolerance);
    diffPct = (count / (dw * dh)) * 100;
    lastMask = mask;
    dscale = scale;
    diffW = dw;
    diffH = dh;
    recluster();
    const out = new Uint8ClampedArray(dw * dh * 4);
    for (let p = 0; p < dw * dh; p++) {
      const i = p * 4;
      if (mask[p]) {
        out[i] = 255;
        out[i + 1] = 45;
        out[i + 2] = 45;
        out[i + 3] = 255;
      } else {
        out[i] = b[i] * 0.22;
        out[i + 1] = b[i + 1] * 0.22;
        out[i + 2] = b[i + 2] * 0.22;
        out[i + 3] = 255;
      }
    }
    diffData = new ImageData(out, dw, dh);
  }

  // Re-cluster the stored diff mask into regions at the current grid size (cheap; no re-diff).
  function recluster() {
    if (!lastMask) return;
    const cell = Math.max(2, Math.round(Math.max(diffW, diffH) / gridDiv));
    boxes = clusterBoxes(lastMask, diffW, diffH, cell).map((x) => ({
      x: x.x / dscale,
      y: x.y / dscale,
      w: x.w / dscale,
      h: x.h / dscale,
    }));
    curBox = boxes.length ? 0 : -1;
  }

  // Draw the diff image whenever it (or the canvas) is ready and we're in diff mode.
  $effect(() => {
    if (mode === "diff" && diffData && diffCanvas) {
      diffCanvas.width = diffW;
      diffCanvas.height = diffH;
      diffCanvas.getContext("2d")?.putImageData(diffData, 0, 0);
    }
  });

  function fit() {
    if (!stageEl || W === 0) return;
    const z = Math.min(stageEl.clientWidth / (mode === "side" ? W * 2 + 16 : W), stageEl.clientHeight / H);
    zoom = Math.min(1, z > 0 ? z : 1);
  }
  const zoomBy = (f: number) => (zoom = Math.min(8, Math.max(0.05, zoom * f)));

  function gotoBox(i: number) {
    if (!boxes.length || !stageEl) return;
    curBox = ((i % boxes.length) + boxes.length) % boxes.length;
    const b = boxes[curBox];
    stageEl.scrollTo({
      left: (b.x + b.w / 2) * zoom - stageEl.clientWidth / 2,
      top: (b.y + b.h / 2) * zoom - stageEl.clientHeight / 2,
    });
  }

  // swipe divider drag
  let dragging = false;
  function onDown(e: PointerEvent) {
    dragging = true;
    e.preventDefault();
  }
  function onMove(e: PointerEvent) {
    if (!dragging || !stageEl) return;
    const r = stageEl.getBoundingClientRect();
    swipePos = Math.min(1, Math.max(0, (e.clientX - r.left + stageEl.scrollLeft) / (W * zoom)));
  }
  function onUp() {
    dragging = false;
  }

  // auto-blink
  let blinkRight = $state(false);
  let blinkTimer: ReturnType<typeof setInterval> | undefined;
  $effect(() => {
    clearInterval(blinkTimer);
    if (mode === "blink") blinkTimer = setInterval(() => (blinkRight = !blinkRight), 600);
    return () => clearInterval(blinkTimer);
  });

  const fmtBytes = (n: number) =>
    n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;
</script>

<svelte:window onpointermove={onMove} onpointerup={onUp} />

<FileViewHeader {name} {onback} {readOnly} {onkeep} {onskip} />

<div class="ibar">
  <span class="modes">
    {#each [["side", "Side"], ["overlay", "Overlay"], ["swipe", "Swipe"], ["blink", "Blink"], ["diff", "Difference"]] as [m, label]}
      <button class="seg" class:on={mode === m} onclick={() => (mode = m as Mode)}>{label}</button>
    {/each}
  </span>

  {#if mode === "overlay"}
    <label class="ctl">opacity <input type="range" min="0" max="1" step="0.02" bind:value={opacity} /></label>
  {/if}
  {#if mode === "diff"}
    <label class="ctl">tolerance {tolerance}
      <input type="range" min="0" max="128" step="1" bind:value={tolerance} onchange={computeDiff} />
    </label>
  {/if}

  <span class="spacer"></span>

  {#if boxes.length}
    <label class="ctl" title="region size: ≈1/{gridDiv} of the image (smaller cells = finer regions)">
      regions 1/{gridDiv}
      <input type="range" min="4" max="40" step="1" bind:value={gridDiv} oninput={recluster} />
    </label>
    <span class="nav">
      <button class="seg" title="previous change" onclick={() => gotoBox(curBox - 1)}>↑</button>
      <span class="navc">{curBox + 1} / {boxes.length}</span>
      <button class="seg" title="next change" onclick={() => gotoBox(curBox + 1)}>↓</button>
    </span>
  {/if}
  <span class="zoom">
    <button class="seg" onclick={() => zoomBy(1 / 1.25)}>−</button>
    <span class="navc">{Math.round(zoom * 100)}%</span>
    <button class="seg" onclick={() => zoomBy(1.25)}>+</button>
    <button class="seg" onclick={fit}>Fit</button>
    <button class="seg" onclick={() => (zoom = 1)}>1:1</button>
  </span>
</div>

<div class="imeta">
  {#if err}
    <span class="err">{err}</span>
  {:else}
    <span>{lw}×{lh} {fmtBytes(leftSize)} · {rw}×{rh} {fmtBytes(rightSize)}</span>
    {#if lw !== rw || lh !== rh}<span class="warn">· different dimensions</span>{/if}
    <span class="pct" class:same={diffPct === 0}>· {diffPct.toFixed(2)}% pixels differ</span>
  {/if}
</div>

<div class="stage" bind:this={stageEl}>
  {#if err}
    <p class="emsg">Couldn't load as images. (You can still inspect the raw bytes via the hex view.)</p>
  {:else if mode === "side"}
    {@const b = curBox >= 0 ? boxes[curBox] : null}
    <div class="sidewrap">
      <div class="sideimg" style="width: {lw * zoom}px; height: {lh * zoom}px">
        <img src={leftUrl} alt="left" style="width: {lw * zoom}px" />
        {#if b}<div class="boxmark" style="left: {b.x * zoom}px; top: {b.y * zoom}px; width: {b.w * zoom}px; height: {b.h * zoom}px"></div>{/if}
      </div>
      <div class="sideimg" style="width: {rw * zoom}px; height: {rh * zoom}px">
        <img src={rightUrl} alt="right" style="width: {rw * zoom}px" />
        {#if b}<div class="boxmark" style="left: {b.x * zoom}px; top: {b.y * zoom}px; width: {b.w * zoom}px; height: {b.h * zoom}px"></div>{/if}
      </div>
    </div>
  {:else}
    <div class="layered" style="width: {W * zoom}px; height: {H * zoom}px">
      {#if mode === "overlay"}
        <img class="lay" src={leftUrl} alt="left" style="width: {lw * zoom}px" />
        <img class="lay" src={rightUrl} alt="right" style="width: {rw * zoom}px; opacity: {opacity}" />
      {:else if mode === "swipe"}
        <img class="lay" src={leftUrl} alt="left" style="width: {lw * zoom}px" />
        <img
          class="lay"
          src={rightUrl}
          alt="right"
          style="width: {rw * zoom}px; clip-path: inset(0 0 0 {swipePos * 100}%)"
        />
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="divider" style="left: {swipePos * W * zoom}px" onpointerdown={onDown}></div>
      {:else if mode === "blink"}
        <img class="lay" src={blinkRight ? rightUrl : leftUrl} alt={blinkRight ? "right" : "left"} style="width: {(blinkRight ? rw : lw) * zoom}px" />
      {:else if mode === "diff"}
        <canvas bind:this={diffCanvas} style="width: {W * zoom}px; height: {H * zoom}px"></canvas>
      {/if}
      {#if curBox >= 0 && mode !== "blink"}
        {@const b = boxes[curBox]}
        <div class="boxmark" style="left: {b.x * zoom}px; top: {b.y * zoom}px; width: {b.w * zoom}px; height: {b.h * zoom}px"></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ibar,
  .imeta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.8rem;
    padding: 0.35rem 0;
  }
  .imeta {
    font-size: 0.8rem;
    opacity: 0.75;
    font-family: ui-monospace, monospace;
  }
  .imeta .warn { color: #c47d1a; }
  .imeta .pct { color: #e5484d; }
  .imeta .pct.same { color: #2bb3a3; }
  .err { color: #e5484d; font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .modes,
  .nav,
  .zoom {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
  }
  .spacer { flex: 1; }
  .seg {
    border: 1px solid rgba(128, 128, 128, 0.4);
    background: transparent;
    color: inherit;
    border-radius: 6px;
    padding: 0.2em 0.55em;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }
  .seg.on {
    background: #396cd8;
    border-color: #396cd8;
    color: #fff;
  }
  .ctl {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
  }
  .navc {
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
    min-width: 2.8em;
    text-align: center;
  }
  .stage {
    border: 1px solid #ddd;
    border-radius: 8px;
    background: #f0f0f0;
    height: 58vh;
    overflow: auto;
  }
  .sidewrap {
    display: flex;
    gap: 1rem;
    padding: 0.5rem;
    align-items: flex-start;
    width: max-content;
  }
  .sideimg {
    position: relative;
    flex: none;
  }
  .sidewrap img {
    display: block;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.15);
    image-rendering: pixelated;
  }
  .layered {
    position: relative;
    margin: 0.5rem;
  }
  .lay {
    position: absolute;
    top: 0;
    left: 0;
    image-rendering: pixelated;
  }
  .layered canvas {
    position: absolute;
    top: 0;
    left: 0;
    image-rendering: pixelated;
  }
  .divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 3px;
    margin-left: -1px;
    background: #396cd8;
    cursor: ew-resize;
    z-index: 3;
  }
  .boxmark {
    position: absolute;
    border: 2px solid #e8a33d;
    box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.04);
    pointer-events: none;
    z-index: 2;
  }
  .emsg {
    padding: 1rem;
    opacity: 0.7;
  }
  @media (prefers-color-scheme: dark) {
    .stage {
      background: #1a1a1a;
      border-color: #3a3a3a;
    }
  }
</style>
