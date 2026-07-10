<script lang="ts">
  import { commands } from "$lib/commands";
  import { tick } from "svelte";
  import type { FileDiff, SaveResult, FileMeta, FileRef } from "$lib/types";
  import { linesOf, computeHunks, type Hunk } from "$lib/sbs-hunks";
  import { segments, rangesFor, buildDetailModel } from "$lib/sbs-diff";
  import { fmtDate } from "$lib/format";
  import FileViewHeader from "./FileViewHeader.svelte";

  let {
    initial,
    left,
    right,
    leftMeta,
    rightMeta,
    name = "",
    leftDate = null,
    rightDate = null,
    leftCreated = null,
    rightCreated = null,
    readOnly = false,
    onkeep,
    onskip,
    onsaved,
    onback,
    onregisterexit,
  }: {
    initial: FileDiff;
    /** Each side as a source + relative path — what `save_file` writes through (any writable backend). */
    left: FileRef;
    right: FileRef;
    /** Per-side metadata at read time (fingerprint + EOL/final-newline) for conflict detection + faithful saves. */
    leftMeta: FileMeta;
    rightMeta: FileMeta;
    /** File name/path shown in the header. */
    name?: string;
    /** Per-side modified time (epoch ms), shown above each pane; orange when they differ. */
    leftDate?: number | null;
    rightDate?: number | null;
    /** Per-side creation time (epoch ms), shown next to modified when the OS exposes it. */
    leftCreated?: number | null;
    rightCreated?: number | null;
    /** Read-only inspection mode: hides all merge/save actions, keeps diff display + navigation.
     *  Used by Migrate's plan review (the decision happens in the plan, not here). */
    readOnly?: boolean;
    /** Keep / Skip callbacks for Migrate plan review (readOnly mode only). */
    onkeep?: () => void;
    onskip?: () => void;
    /** Fired after save(s) hit disk, so the parent can refresh a stale folder listing + announce it. */
    onsaved?: (what: "left" | "right" | "both") => void;
    /** If provided, a "← Back" button appears (folder-tree open); omitted in files mode. */
    onback?: () => void;
    /** The parent registers a callback here so its Escape handler can delegate to this component's
     *  unsaved-changes guard instead of closing directly. */
    onregisterexit?: (fn: (() => void) | null) => void;
  } = $props();

  // Seeded once from `initial`; the parent wraps us in {#key leftPath+rightPath}, so a new file pair
  // re-instantiates this component (fresh `diff`/dirty state). The one-time capture is intended.
  // svelte-ignore state_referenced_locally
  let diff = $state(initial);
  let leftDirty = $state(false);
  let rightDirty = $state(false);
  let busy = $state(false);
  let err = $state("");

  // Pristine copy of the as-opened diff, for Cancel/reset (discard in-memory edits). Intentional one-time
  // capture of the initial value.
  // svelte-ignore state_referenced_locally
  const pristine = $state.snapshot(initial);
  // Both sides' modified times known and differing → flag the dates (orange), like the tree.
  const datesDiffer = $derived(leftDate != null && rightDate != null && leftDate !== rightDate);

  // Cancel: discard all in-memory edits and restore the file exactly as opened.
  function reset() {
    diff = $state.snapshot(pristine);
    leftDirty = false;
    rightDirty = false;
    conflict = null;
    err = "";
  }

  // Live fingerprints for conflict detection; seeded once from props, refreshed after each successful save.
  // svelte-ignore state_referenced_locally
  let lfp = $state(leftMeta.fp);
  // svelte-ignore state_referenced_locally
  let rfp = $state(rightMeta.fp);
  // A save was refused because the file changed on disk; holds which side awaits an overwrite decision.
  let conflict = $state<"left" | "right" | null>(null);

  // Sub-block move: a text selection within one side → a floating arrow to move just those rows.
  let sel = $state<{ side: "left" | "right"; r0: number; r1: number; x: number; y: number } | null>(
    null,
  );
  let sbsEl: HTMLDivElement | undefined = $state();

  const hunkByStart = $derived(new Map(computeHunks(diff).map((h) => [h.startRow, h])));
  const hasDiff = $derived(diff.rows.some((r) => r.kind !== "equal"));

  // --- Fixed-height virtualization: only viewport rows hit the DOM (rows are single-line / no-wrap) ---
  const ROW_H = 20;
  const OVERSCAN = 12;
  let scrollTop = $state(0);
  let viewportH = $state(480);
  const vStart = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const vEnd = $derived(
    Math.min(diff.rows.length, vStart + Math.ceil(viewportH / ROW_H) + OVERSCAN * 2),
  );
  const visible = $derived(diff.rows.slice(vStart, vEnd).map((row, k) => ({ i: vStart + k, row })));

  function onScroll(e: Event) {
    scrollTop = (e.currentTarget as HTMLElement).scrollTop;
    repositionSel(); // keep the floating selection arrow glued to its rows
  }

  // --- Current change (the selected "current difference") + navigation/merge over it (nav wraps around) ---
  const hunkStarts = $derived([...hunkByStart.keys()].sort((a, b) => a - b));
  // Index of the selected change. `curClamped` keeps it valid as hunks come and go after edits.
  let curHunk = $state(0); // navigation wraps (last → first) and is enabled with ≥1 change; see gotoHunk
  const curClamped = $derived(
    hunkStarts.length === 0 ? -1 : Math.max(0, Math.min(curHunk, hunkStarts.length - 1)),
  );
  const canNav = $derived(hunkStarts.length > 0); // enabled with ≥1 change — "next" must reach a lone change
  // Row span [start, end) of the current change — drives the highlight frame.
  const curRange = $derived.by(() => {
    if (curClamped < 0) return null;
    const h = hunkByStart.get(hunkStarts[curClamped]);
    return h ? { start: h.startRow, end: h.endRow } : null;
  });

  // Scroll the current change into view only if it isn't comfortably visible (no needless scroll when
  // it already fits — the fix for the "strange scroll" feel).
  function scrollCurrentIntoView() {
    if (!sbsEl || !curRange) return;
    const top = curRange.start * ROW_H;
    const bot = curRange.end * ROW_H;
    const viewTop = sbsEl.scrollTop;
    const viewBot = viewTop + sbsEl.clientHeight;
    if (top < viewTop + ROW_H || bot > viewBot - ROW_H) {
      sbsEl.scrollTo({ top: Math.max(0, top - ROW_H * 3) }); // a little context above
    }
  }
  function gotoHunk(i: number) {
    const n = hunkStarts.length;
    if (n === 0) return;
    curHunk = ((i % n) + n) % n; // wrap around: last → first, first → last
    scrollCurrentIntoView();
  }
  const nextChange = () => gotoHunk(curClamped + 1);
  const prevChange = () => gotoHunk(curClamped - 1);

  // Select the change containing a given row (single-click in the panes; P4-B5). No-op on equal rows.
  function selectChangeAt(row: number) {
    for (let i = 0; i < hunkStarts.length; i++) {
      const h = hunkByStart.get(hunkStarts[i]);
      if (h && row >= h.startRow && row < h.endRow) {
        curHunk = i;
        return;
      }
    }
  }

  // Copy a specific hunk (clicked in the gutter); selects it as current, copies, then re-clamps.
  async function copyHunk(startRow: number, dir: "lr" | "rl") {
    const h = hunkByStart.get(startRow);
    if (!h) return;
    curHunk = Math.max(0, hunkStarts.indexOf(startRow));
    await copy(h, dir);
    curHunk = Math.min(curHunk, Math.max(0, hunkStarts.length - 1));
  }
  // Copy the current change to a side; with `advance`, scroll the new current change into view. (After
  // a copy the resolved hunk disappears, so the same index already points at the next change.)
  async function copyCurrent(dir: "lr" | "rl", advance: boolean) {
    if (curClamped < 0) return;
    const h = hunkByStart.get(hunkStarts[curClamped]);
    if (!h) return;
    await copy(h, dir);
    curHunk = Math.min(curHunk, Math.max(0, hunkStarts.length - 1));
    if (advance) scrollCurrentIntoView();
  }

  function onKey(e: KeyboardEvent) {
    if (exitConfirm) {
      const actions = [saveAndExit, exitWithoutSaving, () => (exitConfirm = false)];
      if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        confirmFocus = (confirmFocus + (e.key === "ArrowRight" ? 1 : 2)) % 3;
      } else if (e.key === "Enter") {
        e.preventDefault();
        actions[confirmFocus]();
      }
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      nextChange();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      prevChange();
    } else if (e.key === "ArrowRight" && !readOnly) {
      e.preventDefault();
      copyCurrent("lr", true);
    } else if (e.key === "ArrowLeft" && !readOnly) {
      e.preventDefault();
      copyCurrent("rl", true);
    }
  }

  // --- Line/block detail pane (P4-B1): the current change as a UNIFIED block (- removed / + added lines),
  //     gap-padded so the two sides stay column-aligned, full-width with one synced horizontal scroll,
  //     plus a circular stepper over its edits. ---
  let showDetail = $state(true);
  let wordMode = $state(true); // highlight whole changed words (true) vs individual characters (false); top-bar toggle
  let activeStopIdx = $state(0); // index of the active edit (0..stopCount-1) in the detail block
  let detailBodyEl: HTMLDivElement | undefined = $state();

  // Detail block (gap-aligned − / + groups + per-edit stops) — built by a pure helper in `lib/sbs-diff`.
  // The detail pane renders every line of the current change *unvirtualized*, and `buildDetailModel`
  // does a char-level alignment per line — so a single huge hunk (e.g. a CSV where every one of 5000
  // lines differs, seen as one change) would blow up both CPU and DOM. Cap it: feed only the first
  // MAX_DETAIL_ROWS of the hunk to the detail (the main pane, virtualized, still shows the whole thing),
  // and surface how many lines were withheld.
  const MAX_DETAIL_ROWS = 200;
  const detailRange = $derived.by(() => {
    if (!curRange) return null;
    const total = curRange.end - curRange.start;
    return total <= MAX_DETAIL_ROWS
      ? curRange
      : { start: curRange.start, end: curRange.start + MAX_DETAIL_ROWS };
  });
  const detailHidden = $derived(
    curRange && detailRange ? curRange.end - detailRange.end : 0,
  );
  const detailModel = $derived(buildDetailModel(diff.rows, detailRange, wordMode));
  const stopCount = $derived(detailModel.stopCount); // number of edits the stepper walks

  // Edit stepper — circular (last → first, first → last); always usable with ≥1 edit. `tick()` lets the
  // active marker render before we scroll, so re-pressing on a lone edit still re-centres it.
  async function prevWord() {
    if (!stopCount) return;
    activeStopIdx = (activeStopIdx - 1 + stopCount) % stopCount;
    await tick();
    scrollToActiveStop();
  }
  async function nextWord() {
    if (!stopCount) return;
    activeStopIdx = (activeStopIdx + 1) % stopCount;
    await tick();
    scrollToActiveStop();
  }

  // Bring the active edit into view by scrolling the single block container (every line moves together;
  // because the lines are gap-padded, the − / + columns stay aligned at any scroll position).
  function scrollToActiveStop() {
    if (!detailBodyEl) return;
    const el = detailBodyEl.querySelector(`[data-stop="${activeStopIdx}"]`) as HTMLElement | null;
    if (!el) return;
    const line = el.closest(".dline") as HTMLElement | null;
    if (line) {
      detailBodyEl.scrollTop = Math.max(
        0,
        line.offsetTop - detailBodyEl.clientHeight / 2 + line.offsetHeight / 2,
      );
    }
    detailBodyEl.scrollLeft = Math.max(0, el.offsetLeft - detailBodyEl.clientWidth / 2 + el.offsetWidth / 2);
  }

  // Reset the cursor when the current change moves to a different block.
  $effect(() => {
    void curRange?.start;
    activeStopIdx = 0;
  });
  // Re-centre when the active edit (or the block) changes.
  $effect(() => {
    void activeStopIdx;
    void detailModel;
    scrollToActiveStop();
  });

  async function rediff(left: string[], right: string[]) {
    busy = true;
    err = "";
    sel = null; // row indices change after a re-diff → drop any stale selection arrow
    try {
      diff = await commands.diffStrings(left.join("\n"), right.join("\n"));
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
    }
  }

  async function copy(h: Hunk, dir: "lr" | "rl") {
    const cur = linesOf(diff);
    if (dir === "lr") {
      const right = [...cur.right];
      right.splice(h.rightBefore, h.rightLines.length, ...h.leftLines);
      rightDirty = true;
      await rediff(cur.left, right);
    } else {
      const left = [...cur.left];
      left.splice(h.leftBefore, h.leftLines.length, ...h.rightLines);
      leftDirty = true;
      await rediff(left, cur.right);
    }
  }

  // Copy *every* remaining change one way at once. Since already-resolved regions are equal, making
  // one side mirror the other only touches the still-differing hunks (it preserves prior per-hunk edits).
  async function copyAll(dir: "lr" | "rl") {
    if (!hasDiff) return;
    const cur = linesOf(diff);
    if (dir === "lr") {
      rightDirty = true;
      await rediff(cur.left, [...cur.left]);
    } else {
      leftDirty = true;
      await rediff([...cur.right], cur.right);
    }
  }

  // --- Sub-block move via text selection ---
  // Walk up from a selection endpoint to the row cell that carries its row index + side.
  function nodeRowSide(node: Node | null): { row: number; side: "left" | "right" } | null {
    let el: Element | null = node instanceof Element ? node : (node?.parentElement ?? null);
    while (el && !(el instanceof HTMLElement && el.dataset.row !== undefined)) el = el.parentElement;
    if (!(el instanceof HTMLElement) || el.dataset.row === undefined) return null;
    return { row: Number(el.dataset.row), side: el.dataset.side as "left" | "right" };
  }

  function onSelChange() {
    if (readOnly) return; // no move-selection affordance in plan-review mode — skip the work entirely
    const s = document.getSelection();
    if (!s || s.rangeCount === 0 || s.isCollapsed) {
      sel = null;
      return;
    }
    const a = nodeRowSide(s.anchorNode);
    const f = nodeRowSide(s.focusNode);
    const host = sbsEl?.getBoundingClientRect();
    if (!a || !f || !host) {
      sel = null;
      return;
    }
    const r0 = Math.min(a.row, f.row);
    const r1 = Math.max(a.row, f.row);
    // Only offer the move if the selection actually spans a change (not an all-equal region).
    if (!diff.rows.slice(r0, r1 + 1).some((r) => r.kind !== "equal")) {
      sel = null;
      return;
    }
    // Source side = where the drag started (anchor). Float the arrow over the center gutter.
    const rect = s.getRangeAt(0).getBoundingClientRect();
    sel = { side: a.side, r0, r1, x: host.left + host.width / 2, y: rect.top + rect.height / 2 };
  }

  // The inner pane scrolls without firing selectionchange → keep the floating arrow glued to the rows.
  function repositionSel() {
    if (!sel) return;
    const s = document.getSelection();
    const host = sbsEl?.getBoundingClientRect();
    if (!s || s.rangeCount === 0 || s.isCollapsed || !host) {
      sel = null;
      return;
    }
    const rect = s.getRangeAt(0).getBoundingClientRect();
    sel = { ...sel, x: host.left + host.width / 2, y: rect.top + rect.height / 2 };
  }

  function clearSel() {
    document.getSelection()?.removeAllRanges();
    sel = null;
  }

  // Move only the selected rows from their side to the other (generalizes the per-hunk copy to a row
  // range): replace the destination region aligned with rows [r0..r1] by the source lines, then re-diff.
  async function moveSelection() {
    if (!sel) return;
    const { side, r0, r1 } = sel;
    const cur = linesOf(diff);
    let leftStart = 0;
    let rightStart = 0;
    for (let k = 0; k < r0; k++) {
      if (diff.rows[k].left_no !== null) leftStart++;
      if (diff.rows[k].right_no !== null) rightStart++;
    }
    const leftSeg: string[] = [];
    const rightSeg: string[] = [];
    for (let k = r0; k <= r1; k++) {
      const rw = diff.rows[k];
      if (rw.left_no !== null) leftSeg.push(rw.left ?? "");
      if (rw.right_no !== null) rightSeg.push(rw.right ?? "");
    }
    clearSel();
    if (side === "left") {
      const right = [...cur.right];
      right.splice(rightStart, rightSeg.length, ...leftSeg);
      rightDirty = true;
      await rediff(cur.left, right);
    } else {
      const left = [...cur.left];
      left.splice(leftStart, leftSeg.length, ...rightSeg);
      leftDirty = true;
      await rediff(left, cur.right);
    }
  }

  // Returns "saved" | "conflict" | "error". On "conflict" the file changed on disk and nothing was
  // written; `conflict` is set so the UI can offer an overwrite. `force` skips the on-disk check.
  async function save(side: "left" | "right", notify = true, force = false) {
    const cur = linesOf(diff);
    const file = side === "left" ? left : right;
    // Reconstruct with the side's original EOL + trailing newline so an unchanged file round-trips
    // byte-for-byte (no lost final `\n`, no silent CRLF→LF) — otherwise a "merged" side stays "different".
    const meta = side === "left" ? leftMeta : rightMeta;
    const lines = side === "left" ? cur.left : cur.right;
    const contents = lines.join(meta.eol) + (meta.final_nl ? meta.eol : "");
    const expect = side === "left" ? lfp : rfp;
    busy = true;
    err = "";
    try {
      const res = await commands.saveFile(file, contents, expect, force);
      if (res.kind === "conflict") {
        conflict = side;
        return "conflict";
      }
      if (side === "left") {
        lfp = res.fp;
        leftDirty = false;
      } else {
        rfp = res.fp;
        rightDirty = false;
      }
      if (notify) onsaved?.(side);
      return "saved";
    } catch (e) {
      err = String(e);
      return "error";
    } finally {
      busy = false;
    }
  }

  // Save every dirty side, then notify the parent once (so it refreshes/announces a single "both").
  // Stops at the first conflict/error so the user can resolve it before the rest is written.
  async function saveBoth() {
    const sides: ("left" | "right")[] = [];
    if (leftDirty) sides.push("left");
    if (rightDirty) sides.push("right");
    for (const s of sides) {
      if ((await save(s, false)) !== "saved") return;
    }
    if (sides.length > 0) onsaved?.(sides.length === 2 ? "both" : sides[0]);
  }

  // Overwrite the conflicting side anyway (the user accepted clobbering the on-disk change).
  async function forceSave() {
    const side = conflict;
    conflict = null;
    if (side) await save(side, true, true);
  }

  let exitConfirm = $state(false);
  let confirmFocus = $state(0);

  function attemptExit() {
    if (exitConfirm) {
      exitConfirm = false;
      return;
    }
    if (!readOnly && (leftDirty || rightDirty)) {
      confirmFocus = 0;
      exitConfirm = true;
    } else {
      onback?.();
    }
  }

  async function saveAndExit() {
    exitConfirm = false;
    await saveBoth();
    onback?.();
  }

  function exitWithoutSaving() {
    exitConfirm = false;
    onback?.();
  }

  $effect(() => {
    onregisterexit?.(attemptExit);
    return () => onregisterexit?.(null);
  });

</script>

<svelte:document onselectionchange={onSelChange} />
<svelte:window onkeydown={onKey} />

<div class="sbs-wrapper">
<FileViewHeader {name} onback={attemptExit} {readOnly} {onkeep} {onskip}>
  {#snippet right()}
    <span class="fcount">
      <span class="s-rep">{diff.summary.replaced} changed</span>
      <span class="s-ins">{diff.summary.inserted} added</span>
      <span class="s-del">{diff.summary.deleted} removed</span>
    </span>
  {/snippet}
</FileViewHeader>

<div class="bar">
  <span class="grp">
    <button class="ghost nav" disabled={!canNav} title="previous change (↑)" aria-label="previous change" onclick={prevChange}>↑</button>
    <button class="ghost nav" disabled={!canNav} title="next change (↓)" aria-label="next change" onclick={nextChange}>↓</button>
    <span class="nav-count" title="current change / total">{curClamped >= 0 ? `${curClamped + 1} / ${hunkStarts.length}` : "—"}</span>
    <button class="ghost nav" class:on={showDetail} title="toggle the line detail pane" onclick={() => (showDetail = !showDetail)}>⊟</button>
    <button class="ghost nav wc-toggle" class:on={wordMode} title="word- vs character-level diff" onclick={() => (wordMode = !wordMode)}>{wordMode ? "W" : "C"}</button>
  </span>
  {#if !readOnly}
  <span class="grp">
    <button class="ghost" disabled={curClamped < 0 || busy} title="copy current change → right (→)" onclick={() => copyCurrent("lr", false)}>→</button>
    <button class="ghost" disabled={curClamped < 0 || busy} title="copy current change → left (←)" onclick={() => copyCurrent("rl", false)}>←</button>
    <button class="ghost" disabled={curClamped < 0 || busy} title="copy current → right, then next" onclick={() => copyCurrent("lr", true)}>→»</button>
    <button class="ghost" disabled={curClamped < 0 || busy} title="copy current → left, then next" onclick={() => copyCurrent("rl", true)}>«←</button>
    <button class="ghost" disabled={!hasDiff || busy} title="copy every change → right" onclick={() => copyAll("lr")}>⇉</button>
    <button class="ghost" disabled={!hasDiff || busy} title="copy every change → left" onclick={() => copyAll("rl")}>⇇</button>
  </span>
  {/if}
  <span class="spacer"></span>
  {#if err}<span class="err">{err}</span>{/if}
  {#if busy}<span class="busy">…</span>{/if}
  {#if !readOnly}
    <button class="save" disabled={!leftDirty || busy} title="save the left file" onclick={() => save("left")}>💾←{leftDirty ? "*" : ""}</button>
    <button class="save" disabled={!rightDirty || busy} title="save the right file" onclick={() => save("right")}>💾→{rightDirty ? "*" : ""}</button>
    <button class="save" disabled={!(leftDirty && rightDirty) || busy} title="save both files" onclick={saveBoth}>💾⇆</button>
    <button class="ghost" disabled={(!leftDirty && !rightDirty) || busy} title="discard changes — restore the files as opened" aria-label="cancel changes" onclick={reset}>↺</button>
  {/if}
</div>

<div class="dates">
  <span class="dhalf" class:diff={datesDiffer}>{leftDate != null ? `modified ${fmtDate(leftDate)}` : ""}{leftCreated != null ? ` · created ${fmtDate(leftCreated)}` : ""}</span>
  <span class="dgut"></span>
  <span class="dhalf" class:diff={datesDiffer}>{rightDate != null ? `modified ${fmtDate(rightDate)}` : ""}{rightCreated != null ? ` · created ${fmtDate(rightCreated)}` : ""}</span>
</div>

{#if conflict}
  <div class="conflict" role="alert">
    <span>⚠️ The {conflict} file changed on disk since you opened it. Overwrite anyway?</span>
    <span class="spacer"></span>
    <button class="danger" disabled={busy} onclick={forceSave}>Overwrite</button>
    <button class="ghost" disabled={busy} onclick={() => (conflict = null)}>Cancel</button>
  </div>
{/if}

<div class="sbs" bind:this={sbsEl} onscroll={onScroll} bind:clientHeight={viewportH}>
  <div class="sbs-spacer" style="height: {diff.rows.length * ROW_H}px">
    {#each visible as v (v.i)}
      {@const ew = rangesFor(v.row, wordMode)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="srow kind-{v.row.kind}" style="top: {v.i * ROW_H}px" onclick={() => selectChangeAt(v.i)}>
        <div class="half left" data-row={v.i} data-side="left">
          <span class="ln">{v.row.left_no ?? ""}</span>
          <code>{#each segments(v.row.left, ew.left) as seg}<span class:wc={seg.changed}>{seg.text}</span>{/each}</code>
        </div>
        <div class="gutter">
          {#if hunkByStart.has(v.i) && !readOnly}
            <button class="mbtn" title="copy this change → right" disabled={busy} onclick={(e) => { e.stopPropagation(); copyHunk(v.i, "lr"); }}>→</button>
            <button class="mbtn" title="copy this change → left" disabled={busy} onclick={(e) => { e.stopPropagation(); copyHunk(v.i, "rl"); }}>←</button>
          {/if}
        </div>
        <div class="half right" data-row={v.i} data-side="right">
          <span class="ln">{v.row.right_no ?? ""}</span>
          <code>{#each segments(v.row.right, ew.right) as seg}<span class:wc={seg.changed}>{seg.text}</span>{/each}</code>
        </div>
      </div>
    {/each}
    {#if curRange}
      <div
        class="current-frame"
        style="top: {curRange.start * ROW_H}px; height: {(curRange.end - curRange.start) * ROW_H}px"
      ></div>
    {/if}
  </div>
</div>

{#if showDetail}
  <div class="detail">
    <div class="detail-bar">
      <button class="ghost nav" disabled={!stopCount} title="previous edit — wraps around" onclick={prevWord}>‹</button>
      <span class="nav-count">{stopCount ? `${activeStopIdx + 1} / ${stopCount}` : "0"} edits</span>
      <button class="ghost nav" disabled={!stopCount} title="next edit — wraps around" onclick={nextWord}>›</button>
    </div>
    {#if detailModel.groups.length}
      <div class="detail-lines" bind:this={detailBodyEl}>
        {#each detailModel.groups as g, gi (gi)}
          <div class="dgroup" class:pair={g.length === 2}>
            {#each g as line, li (li)}
              <div class="dline {line.sign === '-' ? 'del' : 'ins'}">
                <span class="sign">{line.sign}</span><code>{#each line.cells as c}<span class="dcell {c.cls}" class:active-wc={c.stop >= 0 && c.stop === activeStopIdx} data-stop={c.stop >= 0 ? c.stop : undefined}>{c.text}</span>{/each}</code>
              </div>
            {/each}
          </div>
        {/each}
        {#if detailHidden > 0}
          <div class="detail-truncated">… +{detailHidden} more line{detailHidden === 1 ? "" : "s"} in this change — too large for full detail (see the panes above).</div>
        {/if}
      </div>
    {:else}
      <div class="detail-empty">Select a change (↑/↓) to inspect it here.</div>
    {/if}
  </div>
{/if}

{#if sel && !readOnly}
  <!-- Floating arrow over the selection: moves only the selected rows to the other side. mousedown is
       prevented so clicking the button doesn't collapse the text selection before onclick runs. -->
  <button
    class="floatmove"
    style="left: {sel.x}px; top: {sel.y}px;"
    title={sel.side === "left" ? "move selection to the right" : "move selection to the left"}
    onmousedown={(e) => e.preventDefault()}
    onclick={moveSelection}
  >{sel.side === "left" ? "→" : "←"}</button>
{/if}

{#if exitConfirm}
<div class="exit-overlay" role="dialog" aria-modal="true">
  <div class="exit-dialog">
    <p>You have unsaved changes. Save before leaving?</p>
    <div class="exit-actions">
      <button class="save" class:focused={confirmFocus === 0} disabled={busy} onclick={saveAndExit}>Save and exit</button>
      <button class="danger" class:focused={confirmFocus === 1} onclick={exitWithoutSaving}>Exit without saving</button>
      <button class="ghost" class:focused={confirmFocus === 2} onclick={() => (exitConfirm = false)}>Cancel</button>
    </div>
  </div>
</div>
{/if}
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.8rem;
    padding: 0.4rem 0;
  }
  .fcount {
    display: flex;
    gap: 0.8rem;
    font-size: 0.82rem;
    font-weight: 600;
    margin-left: auto;
    white-space: nowrap;
  }
  .s-rep { color: #c47d1a; }
  .s-ins { color: #2bb3a3; }
  .s-del { color: #e5484d; }
  .bar .spacer { flex: 1; }
  .err { color: #e5484d; font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .busy { opacity: 0.6; }
  .save {
    border-radius: 7px;
    border: 1px solid #396cd8;
    background: #396cd8;
    color: #fff;
    font-weight: 600;
    padding: 0.3em 0.7em;
    font-size: 0.82rem;
    cursor: pointer;
  }
  .save:disabled { opacity: 0.45; cursor: not-allowed; }
  .grp { display: inline-flex; align-items: center; gap: 0.3rem; }
  .nav { padding: 0.2em 0.55em; line-height: 1; font-size: 0.9rem; }
  .nav-count {
    font-size: 0.78rem;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
    min-width: 3.2em;
    text-align: center;
    white-space: nowrap;
  }
  /* Per-side modified dates above the panes (grid mirrors the row layout). */
  .dates {
    display: grid;
    grid-template-columns: 1fr 3rem 1fr;
    font-size: 0.72rem;
    opacity: 0.6;
    padding: 0.1rem 0 0.2rem;
  }
  .dhalf {
    padding: 0 0.9em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dhalf.diff {
    color: #e8a33d;
    opacity: 1;
  }
  .ghost {
    border-radius: 7px;
    border: 1px solid #aaa;
    background: transparent;
    color: inherit;
    font-weight: 600;
    padding: 0.3em 0.7em;
    font-size: 0.82rem;
    cursor: pointer;
  }
  .ghost:disabled { opacity: 0.4; cursor: not-allowed; }
  .danger {
    border-radius: 7px;
    border: 1px solid #e5484d;
    background: #e5484d;
    color: #fff;
    font-weight: 600;
    padding: 0.3em 0.7em;
    font-size: 0.82rem;
    cursor: pointer;
  }
  .danger:disabled { opacity: 0.45; cursor: not-allowed; }
  .conflict {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.8rem;
    margin: 0.4rem 0;
    border-radius: 7px;
    background: #fdecec;
    border: 1px solid #e5484d;
    font-size: 0.85rem;
  }
  .conflict .spacer { flex: 1; }
  @media (prefers-color-scheme: dark) {
    .conflict { background: #3a1f20; }
  }

  .nav.on {
    background: #396cd8;
    color: #fff;
    border-color: #396cd8;
  }
  .wc-toggle { min-width: 3.4em; }
  /* Container that fills the flex slot given by main, then manages fhead/bar/dates/sbs/detail
     vertically. This makes .detail visible below .sbs without overflow:hidden clipping it. */
  .sbs-wrapper {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .sbs {
    position: relative;
    border: 1px solid #ddd;
    border-radius: 8px;
    overflow: auto;
    background: #fff;
    flex: 1 1 auto;
    min-height: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.82rem;
  }

  /* Line detail pane — flex:none so .sbs keeps its flex:1 height and detail sits below it. */
  .detail {
    flex: none;
    margin-top: 0.5rem;
    border: 1px solid #ddd;
    border-radius: 8px;
    background: #fff;
    overflow: hidden;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.82rem;
  }
  .detail-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.25rem 0.5rem;
    border-bottom: 1px solid #eee;
    background: rgba(128, 128, 128, 0.06);
  }
  /* single scroll container → all lines scroll together (the − / + stay aligned) */
  .detail-lines {
    position: relative;
    max-height: 28vh;
    overflow: auto;
  }
  /* one unified line: plain text; only changed words are coloured. Width grows with content; short
     lines still fill the width. No per-line scroll — the container scrolls them all in sync. */
  .dline {
    white-space: pre;
    width: max-content;
    min-width: 100%;
    height: 22px;
    line-height: 22px;
  }
  .dline .sign {
    position: sticky;
    left: 0;
    z-index: 1;
    display: inline-block;
    width: 1.6em;
    text-align: center;
    font-weight: 700;
    background: #fff; /* opaque so text scrolls underneath it */
  }
  .dline.del .sign { color: #e5484d; }
  .dline.ins .sign { color: #2bb3a3; }
  /* Colour scheme (mirrors the side-by-side): modified-line context = light orange, a changed word =
     dark orange, an addition = green, a deletion = red. Gap (padding) cells stay unshaded so they read
     as holes, not real spaces — keeping the − / + lines column-aligned. */
  .dcell { border-radius: 2px; }
  .dcell.eq { background: rgba(232, 163, 61, 0.18); }
  .dcell.rep { background: rgba(232, 163, 61, 0.45); }
  .dcell.del { background: rgba(229, 72, 77, 0.4); }
  .dcell.ins { background: rgba(43, 179, 163, 0.4); }
  .dcell.rep.active-wc { background: rgba(232, 163, 61, 0.75); }
  .dcell.del.active-wc { background: rgba(229, 72, 77, 0.7); }
  .dcell.ins.active-wc { background: rgba(43, 179, 163, 0.72); }
  .dcell.active-wc { outline: 2px solid #e8a33d; }
  /* group related lines: a small gap between groups, and a thin vertical bar joining a − / + pair,
     drawn in the sticky sign column so it stays put while scrolling horizontally, with a few px of
     inset at the very top and bottom so it doesn't touch the neighbouring groups */
  .dgroup + .dgroup { margin-top: 5px; }
  .dgroup.pair .sign::after {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: rgba(140, 140, 140, 0.75);
  }
  .dgroup.pair .dline:first-child .sign::after { top: 3px; }
  .dgroup.pair .dline:last-child .sign::after { bottom: 3px; }
  .detail-empty {
    padding: 0.6rem;
    opacity: 0.6;
    font-size: 0.8rem;
  }
  .detail-truncated {
    padding: 0.4rem 0.6rem;
    opacity: 0.6;
    font-size: 0.75rem;
    font-style: italic;
    border-top: 1px dashed rgba(128, 128, 128, 0.3);
  }
  .sbs-spacer {
    position: relative;
    width: 100%;
  }
  /* frame around the current (selected) change — drawn as an overlay so it doesn't disturb row colors */
  .current-frame {
    position: absolute;
    left: 0;
    right: 0;
    box-sizing: border-box;
    border: 2px solid #396cd8;
    border-radius: 3px;
    pointer-events: none;
    z-index: 1;
  }
  .srow {
    position: absolute;
    left: 0;
    right: 0;
    height: 20px;
    display: grid;
    /* fixed center column → the gutter always reserves room for the →/← buttons, so panes don't
       jump width between rows that have buttons and rows that don't */
    grid-template-columns: 1fr 3rem 1fr;
  }
  .half {
    display: flex;
    align-items: center;
    gap: 0.5em;
    padding: 0 0.4em;
    min-width: 0;
    overflow: hidden;
  }
  .ln {
    flex: none;
    width: 3.2em;
    text-align: right;
    color: #999;
    user-select: none;
    padding-right: 0.5em;
    border-right: 1px solid rgba(128, 128, 128, 0.25);
  }
  code {
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wc {
    background: rgba(232, 163, 61, 0.45);
    border-radius: 2px;
  }
  .gutter {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1px;
    padding: 0 2px;
    overflow: hidden;
    border-left: 1px solid #bbb;
    border-right: 1px solid #bbb;
    background: rgba(128, 128, 128, 0.06);
  }
  .mbtn {
    border: none;
    background: rgba(128, 128, 128, 0.2);
    color: inherit;
    cursor: pointer;
    font-size: 0.75rem;
    line-height: 1.4;
    padding: 0 0.35em;
    border-radius: 3px;
  }
  .mbtn:disabled { opacity: 0.4; cursor: not-allowed; }
  .floatmove {
    position: fixed;
    transform: translate(-50%, -50%);
    z-index: 20;
    width: 2rem;
    height: 2rem;
    border-radius: 999px;
    border: 1px solid #2b6cb0;
    background: #396cd8;
    color: #fff;
    font-size: 1rem;
    font-weight: 700;
    line-height: 1;
    cursor: pointer;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  }

  .kind-delete .left { background: rgba(229, 72, 77, 0.22); }
  .kind-insert .right { background: rgba(43, 179, 163, 0.22); }
  .kind-replace .left,
  .kind-replace .right { background: rgba(232, 163, 61, 0.12); }
  .kind-delete .right,
  .kind-insert .left { background: rgba(128, 128, 128, 0.06); }

  .exit-overlay {
    position: absolute;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
  }
  .exit-dialog {
    background: #fff;
    border: 1px solid #ddd;
    border-radius: 10px;
    padding: 1.2rem 1.5rem;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.25);
    max-width: 24rem;
  }
  .exit-dialog p { margin: 0 0 0.8rem; font-size: 0.9rem; }
  .exit-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .exit-actions .focused { outline: 3px solid #396cd8; outline-offset: 1px; }

  @media (prefers-color-scheme: dark) {
    .sbs { background: #232323; border-color: #3a3a3a; }
    .ln { border-right-color: #444; }
    .gutter { border-color: #555; }
    .detail { background: #232323; border-color: #3a3a3a; }
    .detail-bar { border-bottom-color: #3a3a3a; }
    .dline .sign { background: #232323; }
    .exit-overlay { background: rgba(0, 0, 0, 0.6); }
    .exit-dialog { background: #2a2a2a; border-color: #444; color: #eee; }
  }
</style>
