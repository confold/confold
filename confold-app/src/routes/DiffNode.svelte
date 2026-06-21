<script lang="ts">
  // A single flat row (one tree entry). The tree is flattened + virtualized in +page.svelte; this
  // component is intentionally NOT recursive.
  import { onDestroy } from "svelte";
  import type { DiffEntry, DiffStatus, EntryMeta } from "$lib/types";
  import { selection, keyOf, toggle } from "$lib/selection.svelte";
  import { isOpen, toggleOpen } from "$lib/expand.svelte";
  import { fmtDate } from "$lib/format";

  let {
    entry,
    depth = 0,
    dateTol = 2000,
    loading = false,
    resolvedStatus = null,
    onopen,
    onexpand,
  }: {
    entry: DiffEntry;
    depth?: number;
    dateTol?: number;
    loading?: boolean;
    resolvedStatus?: DiffStatus | null;
    onopen?: (e: DiffEntry) => void;
    onexpand?: (e: DiffEntry) => void;
  } = $props();

  // Double-click: a file opens (side-by-side if on both sides, single view vs empty if on one — so you can
  // review then create it on the other side); a folder drills in (only when present on both sides).
  const openable = $derived(
    entry.is_dir ? entry.left !== null && entry.right !== null : entry.left !== null || entry.right !== null,
  );
  // A dir present on both sides whose subtree hasn't been loaded yet (lazy/preload mode).
  const isPending = $derived(entry.status === "skipped" && entry.detail === "not descended");
  // A file present on both sides whose verdict is still being computed (streamed in via events).
  const isComparing = $derived(!entry.is_dir && entry.status === "skipped" && entry.detail === "comparing");
  // Still waiting: pending dir (no resolved status yet) or a comparing file (verdict not in yet).
  const stillPending = $derived((isPending && resolvedStatus === null) || isComparing);
  // The status to use for CSS class + marker once the dir has been resolved.
  const effectiveStatus = $derived<DiffStatus>(resolvedStatus ?? entry.status);
  // Show a toggle arrow for dirs with children OR pending dirs (which will have children once expanded).
  const hasChildren = $derived(entry.is_dir && (entry.children.length > 0 || isPending));
  // Files present on both sides whose modified times differ by more than dateTol ms → flag orange.
  const datesDiffer = $derived(
    !entry.is_dir && entry.left !== null && entry.right !== null &&
    Math.abs((entry.left.mtime ?? 0) - (entry.right.mtime ?? 0)) > dateTol,
  );
  const createdDiffer = $derived(
    !entry.is_dir && entry.left !== null && entry.right !== null &&
    entry.left.created !== null && entry.right.created !== null &&
    Math.abs(entry.left.created - entry.right.created) > dateTol,
  );

  // 200ms timer used to distinguish single-click (expand/collapse) from double-click (drill-in) on
  // folders. Cleared on double-click so the expand doesn't fire before the drill-in.
  let clickTimer: ReturnType<typeof setTimeout> | null = null;
  onDestroy(() => { if (clickTimer) clearTimeout(clickTimer); });

  function handleFolderClick() {
    const key = keyOf(entry);
    const wasOpen = isOpen(key);
    toggleOpen(key);
    if (isPending && !wasOpen) onexpand?.(entry);
  }

  function size(meta: EntryMeta | null): string {
    if (!meta) return "—";
    if (meta.kind === "dir") return "";
    const b = meta.size;
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  }

  function cap(s: string): string {
    return s ? s[0].toUpperCase() + s.slice(1) : s;
  }

  function tooltipText(e: DiffEntry): string {
    if (e.is_dir) {
      switch (e.status) {
        case "left_only":  return "Folder only in source";
        case "right_only": return "Folder only in destination";
        case "identical":  return "All contents identical";
        case "different":  return "Folder contains differences";
        case "skipped":    return "Skipped";
        default:           return cap(e.detail ?? e.status);
      }
    }
    const lsz = e.left?.kind !== "dir" ? size(e.left) : null;
    const rsz = e.right?.kind !== "dir" ? size(e.right) : null;
    const ldt = e.left?.mtime != null ? fmtDate(e.left.mtime) : null;
    const rdt = e.right?.mtime != null ? fmtDate(e.right.mtime) : null;
    switch (e.status) {
      case "left_only":
        return ["Only in source", lsz, ldt].filter(Boolean).join(" · ");
      case "right_only":
        return ["Only in destination", rsz, rdt].filter(Boolean).join(" · ");
      case "identical":
        return ["Identical", lsz].filter(Boolean).join(" · ");
      case "different": {
        const label = cap(e.detail ?? "Content differs");
        const szInfo = lsz && rsz && lsz !== rsz ? `${lsz} → ${rsz}` : null;
        return [label, szInfo].filter(Boolean).join(" · ");
      }
      case "skipped": return "Skipped";
      case "error":   return cap(e.detail ?? "Error");
      default:        return cap(e.detail ?? e.status);
    }
  }

  const tooltip = $derived(
    isComparing ? "Comparing…" : tooltipText({ ...entry, status: effectiveStatus }),
  );

  let ttx = $state(0);
  let tty = $state(0);
  let ttVisible = $state(false);

  function onMove(e: MouseEvent) {
    ttx = e.clientX + 14;
    tty = e.clientY + 20;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="node status-{effectiveStatus}" class:openable class:pending={stillPending}
  onclick={(e) => {
    if ((e.target as HTMLElement).closest('.toggle, .sel')) return;
    if (entry.is_dir) {
      if (e.detail !== 1) return; // 2nd click of a double-click: wait for ondblclick to handle it
      // Start a 200ms timer; if ondblclick fires before it, the timer is cleared and we drill in instead.
      if (clickTimer) clearTimeout(clickTimer);
      clickTimer = setTimeout(() => { clickTimer = null; handleFolderClick(); }, 300);
    } else if (openable && e.detail === 1) {
      onopen?.(entry); // files: direct, no delay; ignore 2nd click of a double-click gesture
    }
  }}
  ondblclick={() => {
    // Cancel the pending expand and drill into the folder.
    if (clickTimer) { clearTimeout(clickTimer); clickTimer = null; }
    if (entry.is_dir && openable) onopen?.(entry);
  }}
  onmouseenter={(e) => { ttx = e.clientX + 14; tty = e.clientY + 20; ttVisible = true; }}
  onmouseleave={() => (ttVisible = false)}
  onmousemove={onMove}
>
  <span class="namecol" style="padding-left: {depth * 18 + 4}px">
    <input
      type="checkbox"
      class="sel"
      checked={selection.has(keyOf(entry))}
      onchange={() => toggle(entry)}
      aria-label="select {entry.name}"
    />
    {#if hasChildren}
      <button class="toggle" onclick={() => {
        const key = keyOf(entry);
        const wasOpen = isOpen(key);
        toggleOpen(key);
        if (isPending && !wasOpen) onexpand?.(entry);
      }} aria-label="toggle">{isOpen(keyOf(entry)) ? "▼" : "▶"}</button>
    {:else}
      <span class="toggle spacer"></span>
    {/if}
    <span class="marker" aria-label={effectiveStatus} title={effectiveStatus}>
      {#if stillPending}
        {#if loading}
          <span class="spinner"></span>
        {:else}
          <span class="pend-box"></span>
        {/if}
      {:else if effectiveStatus === "skipped"}
        <span class="dot">·</span>
      {:else}
        <span class="box"><span class="h l"></span><span class="h r"></span></span>
      {/if}
    </span>
    <span class="name">{entry.name}{entry.is_dir ? "/" : ""}</span>
  </span>
  <span class="szcol">{size(entry.left)}</span>
  <span class="szcol">{size(entry.right)}</span>
  <span class="dtcol" class:diff={datesDiffer}>{entry.is_dir || !entry.left ? "" : fmtDate(entry.left.mtime)}</span>
  <span class="dtcol" class:diff={datesDiffer}>{entry.is_dir || !entry.right ? "" : fmtDate(entry.right.mtime)}</span>
  <span class="dtcol" class:diff={createdDiffer}>{entry.is_dir || !entry.left ? "" : fmtDate(entry.left.created)}</span>
  <span class="dtcol" class:diff={createdDiffer}>{entry.is_dir || !entry.right ? "" : fmtDate(entry.right.created)}</span>
</div>

{#if ttVisible}
  <div class="tt" style="left:{ttx}px; top:{tty}px">{tooltip}</div>
{/if}

<style>
  .node {
    display: flex;
    align-items: center;
    height: 24px;
    box-sizing: border-box;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.85rem;
    border-left: 3px solid transparent;
  }
  .node:hover {
    background: rgba(128, 128, 128, 0.12);
  }
  .node.openable .name {
    cursor: pointer;
  }
  .node.openable:hover .name {
    text-decoration: underline;
  }
  .namecol {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.4em;
  }
  .sel {
    flex: none;
  }
  .toggle {
    flex: none;
    width: 1.3em;
    border: none;
    background: none;
    cursor: pointer;
    color: inherit;
    opacity: 0.8;
    padding: 0;
    font-size: 0.68em;
    line-height: 1;
  }
  .toggle.spacer {
    cursor: default;
  }
  .marker {
    flex: none;
    width: 14px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .dot {
    font-weight: 700;
    opacity: 0.6;
  }
  /* Split square: left half = present on the source (origin), right half = present on the destination. */
  .box {
    display: inline-flex;
    width: 12px;
    height: 12px;
    border: 1px solid rgba(128, 128, 128, 0.6);
    border-radius: 2px;
    overflow: hidden;
  }
  .h {
    width: 50%;
    height: 100%;
  }
  .h.l {
    border-right: 1px solid rgba(128, 128, 128, 0.45);
  }
  .status-left_only .h.l {
    background: #4c8bf5;
  }
  .status-right_only .h.r {
    background: #2bb3a3;
  }
  .status-identical .h {
    background: #ffffff;
  }
  .status-different .h {
    background: #e8a33d;
  }
  .status-error .box {
    border-color: #e5484d;
  }
  .status-error .h {
    background: #e5484d;
  }
  .name {
    flex: 0 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* fixed-width, right-aligned size columns with a separating gridline → tabular look */
  .szcol {
    flex: none;
    width: 6em;
    text-align: right;
    padding: 0 0.6em;
    opacity: 0.7;
    font-size: 0.78rem;
    white-space: nowrap;
    overflow: hidden;
    border-left: 1px solid rgba(128, 128, 128, 0.18);
  }
  .dtcol {
    flex: none;
    width: 9.5em;
    text-align: right;
    padding: 0 0.6em;
    opacity: 0.7;
    font-size: 0.74rem;
    white-space: nowrap;
    overflow: hidden;
    border-left: 1px solid rgba(128, 128, 128, 0.18);
  }
  .dtcol.diff {
    color: #e8a33d;
    opacity: 1;
  }
  /* Floating tooltip: position:fixed escapes overflow:auto/hidden containers. */
  /* Inverted scheme: dark tooltip on light UI, light tooltip on dark UI — always readable. */
  .tt {
    position: fixed;
    z-index: 9999;
    pointer-events: none;
    background: #2e2c28;
    color: #f0ece4;
    border: 1px solid #4a4540;
    font-size: 0.8rem;
    padding: 0.3em 0.65em;
    border-radius: 6px;
    white-space: nowrap;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.28);
  }
  @media (prefers-color-scheme: dark) {
    .tt { background: #f0ece4; color: #1e1c18; border-color: #c8c2b8; box-shadow: 0 2px 8px rgba(0,0,0,0.12); }
  }
  /* Responsive: hide date columns first (they're widest), then size columns. */
  @media (max-width: 1200px) {
    .dtcol { display: none; }
  }
  @media (max-width: 900px) {
    .szcol { display: none; }
  }

  /* Status colors */
  .status-identical {
    border-left-color: #9aa0a6;
  }
  .status-identical .marker {
    color: #6b7280;
  }
  .status-different {
    border-left-color: #e8a33d;
  }
  .status-different .marker {
    color: #e8a33d;
  }
  .status-left_only {
    border-left-color: #4c8bf5;
  }
  .status-left_only .marker {
    color: #4c8bf5;
  }
  .status-right_only {
    border-left-color: #2bb3a3;
  }
  .status-right_only .marker {
    color: #2bb3a3;
  }
  .status-skipped {
    border-left-color: #c0c0c0;
    opacity: 0.55;
  }
  /* Pending dirs (lazy mode, not yet scanned) stay fully visible despite skipped status. */
  .node.pending {
    opacity: 1;
    border-left-color: #888;
    border-left-style: dashed;
  }
  /* Hollow dashed box: "exists on both sides, contents unknown". */
  .pend-box {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 1.5px dashed rgba(128, 128, 128, 0.6);
    border-radius: 2px;
  }
  .spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 2px solid rgba(128, 128, 128, 0.25);
    border-top-color: #888;
    border-radius: 50%;
    animation: spin 0.65s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .status-error {
    border-left-color: #e5484d;
  }
  .status-error .marker {
    color: #e5484d;
  }
</style>
