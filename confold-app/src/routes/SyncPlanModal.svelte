<script lang="ts">
  // The Sync dry-run plan review modal. Like MigratePlanModal it uses a two-column (left/right) layout:
  // the side being written shows the action (+ copy / override) and the source side shows "no change",
  // so direction reads off the column. Per-item checkboxes + virtualization mirror Migrate; the `checked`
  // SvelteSet is owned by the parent and mutated here.
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { slugOf } from "$lib/sources";
  import type { SourceSpec } from "$lib/types";
  import { uncheckedCount, actionKey, type MigrateAction } from "$lib/migrate";

  let {
    actions,
    checked,
    originSpec,
    destSpec,
    scrollTop = $bindable(0),
    onback,
    onclose,
    onapply,
    onview,
  }: {
    actions: MigrateAction[];
    checked: SvelteSet<string>;
    originSpec: SourceSpec | null;
    destSpec: SourceSpec | null;
    /** Virtualized-list scroll position — owned by the parent so it survives leaving for a file view. */
    scrollTop?: number;
    onback: () => void;
    onclose: () => void;
    onapply: () => void;
    /** Open a conflict (different) action in the read-only side-by-side. */
    onview: (action: MigrateAction) => void;
  } = $props();

  // Item counts by direction (item_count so a dir of 45 files counts as 45).
  const sumBy = (ops: string[]) =>
    actions.filter((a) => ops.includes(a.op)).reduce((s, a) => s + a.item_count, 0);
  const planToRight = $derived(sumBy(["copy_left_to_right"]));
  const planToLeft = $derived(sumBy(["copy_right_to_left"]));
  const planDeletes = $derived(sumBy(["delete_left", "delete_right"]));
  const planTotal = $derived(planToRight + planToLeft + planDeletes);
  const planSkipped = $derived(uncheckedCount(actions, checked));

  // A conflict (resolved overwrite) is a "different" reason; those are clickable to view the diff.
  const isConflict = (a: MigrateAction) => a.reason === "different";

  type Cell = { label: string; cls: string };
  const NO_CHANGE: Cell = { label: "no change", cls: "muted" };
  const ABSENT: Cell = { label: "—", cls: "muted" };
  const OVERRIDE: Cell = { label: "override", cls: "override" };
  const DEL: Cell = { label: "✕ delete", cls: "del" };

  // The two-column view: the side being WRITTEN (or deleted) shows the action; the source side is
  // unchanged ("no change"); a side that doesn't have the item shows "—". Direction is conveyed by which
  // column the action lands in (matching the Migrate plan's Orig/Dest layout); the copy colour also
  // tracks direction (blue → right, purple → left).
  function cells(a: MigrateAction): { left: Cell; right: Cell } {
    switch (a.op) {
      case "copy_left_to_right":
        return { left: NO_CHANGE, right: a.reason === "different" ? OVERRIDE : { label: "+ copy", cls: "copy-right" } };
      case "copy_right_to_left":
        return { left: a.reason === "different" ? OVERRIDE : { label: "+ copy", cls: "copy-left" }, right: NO_CHANGE };
      case "delete_left":
        return { left: DEL, right: ABSENT };
      default: // delete_right
        return { left: ABSENT, right: DEL };
    }
  }

  // Virtualized list (fixed-height rows, absolute-positioned window).
  const ROW_H = 28;
  const OVERSCAN = 4;
  let viewH = $state(320);
  let listEl = $state<HTMLElement | undefined>();
  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const end = $derived(Math.min(actions.length, start + Math.ceil(viewH / ROW_H) + OVERSCAN * 2));
  const visible = $derived(actions.slice(start, end).map((a, i) => ({ a, index: start + i })));

  // Restore the parent-held scroll position on (re)mount — survives opening a file and coming back.
  onMount(() => {
    if (listEl && scrollTop) listEl.scrollTop = scrollTop;
  });

  function toggle(key: string, on: boolean) {
    if (on) checked.add(key);
    else checked.delete(key);
  }
</script>

<div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal mig">
    <h2>Sync plan</h2>

    <p class="mig-dir">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span> {slugOf(originSpec).label}{/if}
      <span class="arrow">⇄</span>
      {#if destSpec}<span class="ico">{slugOf(destSpec).icon}</span> {slugOf(destSpec).label}{/if}
    </p>

    <div class="plan-counts">
      {#if actions.length === 0}
        <span class="plan-ok">✓ Both sides already agree for the selected options.</span>
      {:else}
        {#if planToRight > 0}<span class="to-right">→ {planToRight} to {destSpec ? slugOf(destSpec).label : "right"}</span>{/if}
        {#if planToLeft > 0}<span class="to-left">← {planToLeft} to {originSpec ? slugOf(originSpec).label : "left"}</span>{/if}
        {#if planDeletes > 0}<span class="plan-del">✕ {planDeletes} delete</span>{/if}
        <span class="plan-total">{planTotal} item{planTotal === 1 ? "" : "s"} total</span>
        {#if planSkipped > 0}<span class="plan-skipped">{planSkipped} unchecked</span>{/if}
      {/if}
    </div>

    {#if actions.length > 0}
      <div class="plan-header">
        <span class="plan-check-col"></span>
        <span class="plan-path-col">Path</span>
        <span class="plan-side-label">{originSpec ? slugOf(originSpec).label : "Left"}</span>
        <span class="plan-side-label">{destSpec ? slugOf(destSpec).label : "Right"}</span>
      </div>
      <div
        class="plan-list"
        bind:this={listEl}
        onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)}
        bind:clientHeight={viewH}
      >
        <div style="height: {actions.length * ROW_H}px; position: relative;">
          {#each visible as v (v.index)}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
              class="plan-row"
              class:plan-row-clickable={isConflict(v.a)}
              class:plan-row-unchecked={!checked.has(actionKey(v.a))}
              style="top: {v.index * ROW_H}px"
              onclick={isConflict(v.a) ? () => onview(v.a) : undefined}
              title={isConflict(v.a) ? "Conflict — click to view diff" : undefined}
            >
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <span class="plan-check-col" onclick={(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={checked.has(actionKey(v.a))}
                  onchange={(e) => toggle(actionKey(v.a), (e.target as HTMLInputElement).checked)}
                />
              </span>
              <span class="plan-path">{v.a.rel_path.join("/")}{v.a.is_dir ? "/" : ""}{#if v.a.is_dir}<span class="plan-badge">dir • {v.a.item_count} item{v.a.item_count === 1 ? "" : "s"}</span>{/if}</span>
              <span class="plan-cell {cells(v.a).left.cls}">{cells(v.a).left.label}</span>
              <span class="plan-cell {cells(v.a).right.cls}">{cells(v.a).right.label}</span>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <div class="mig-disclaimer">
      <p>Sync can <strong>overwrite and/or delete</strong> data and may fail partway.
         <strong>Back up both sides first.</strong> Unresolved conflicts are left for you to merge in the
         comparison view afterward.</p>
    </div>

    <div class="modal-actions">
      <button class="ghost" onclick={onback}>← Back</button>
      {#if actions.length === 0}
        <button class="ghost" onclick={onclose}>Close</button>
      {:else}
        <button class="danger" onclick={onapply}>Sync →</button>
      {/if}
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .modal {
    background: #fff;
    color: #1a1a1a;
    border-radius: 10px;
    padding: 1.2rem;
    width: min(560px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
  }
  .modal h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }
  .modal.mig {
    gap: 0.7rem;
    width: clamp(360px, 80vw, 1000px);
    max-height: 90vh;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 0.6rem;
  }
  .mig-dir {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-family: ui-monospace, monospace;
    font-size: 0.85rem;
    margin: 0;
    flex-wrap: wrap;
  }
  .arrow {
    opacity: 0.6;
    font-weight: 700;
  }
  .mig-disclaimer {
    background: #fdecec;
    border: 1px solid #e5484d;
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
  }
  .mig-disclaimer p { margin: 0; }
  .plan-counts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 0.8rem;
    font-size: 0.82rem;
    font-weight: 600;
  }
  .plan-del   { color: #e5484d; font-weight: 700; }
  .plan-total { opacity: 0.55; }
  .plan-ok    { color: #2ea043; }
  .to-right { color: #4c8bf5; font-weight: 700; }
  .to-left  { color: #8b5cf6; font-weight: 700; }
  .plan-skipped {
    opacity: 0.5;
    font-style: italic;
  }
  .plan-list {
    border: 1px solid rgba(128, 128, 128, 0.25);
    border-radius: 8px;
    background: #f9f9f9;
    overflow-y: auto;
    flex: 1 1 auto;
    min-height: 160px;
    position: relative;
  }
  .plan-header {
    display: flex;
    align-items: center;
    padding: 0 0.7rem 0 0.5rem;
    gap: 0;
    margin-bottom: 0.15rem;
  }
  .plan-check-col {
    flex: none;
    width: 2rem;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .plan-check-col input[type="checkbox"] {
    margin: 0;
    width: 13px;
    height: 13px;
    cursor: pointer;
    padding: 0;
    border: 1px solid #aaa;
    background: #fff;
  }
  .plan-path-col {
    flex: 1;
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.45;
  }
  .plan-side-label {
    flex: none;
    width: 5.5rem;
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.45;
    text-align: center;
    border-left: 1px solid rgba(128,128,128,0.15);
    padding-left: 0.3rem;
  }
  .plan-row {
    position: absolute;
    left: 0; right: 0;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 0;
    padding: 0;
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
  }
  .plan-row-unchecked { opacity: 0.4; }
  .plan-row-clickable { cursor: pointer; }
  .plan-row-clickable:hover { background: rgba(232, 163, 61, 0.16); }
  .plan-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-right: 0.4rem;
  }
  .plan-cell {
    flex: none;
    width: 5.5rem;
    font-size: 0.73rem;
    font-weight: 600;
    text-align: center;
    border-left: 1px solid rgba(128,128,128,0.15);
    white-space: nowrap;
  }
  .plan-cell.copy-right { color: #4c8bf5; }
  .plan-cell.copy-left  { color: #8b5cf6; }
  .plan-cell.override   { color: #e8a33d; }
  .plan-cell.del        { color: #e5484d; }
  .plan-cell.muted      { opacity: 0.35; }
  .plan-badge {
    display: inline-block;
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.5;
    border: 1px solid rgba(128,128,128,0.3);
    border-radius: 3px;
    padding: 0 0.25em;
    margin-left: 0.3em;
    vertical-align: middle;
  }
  button {
    border-radius: 7px;
    border: 1px solid #396cd8;
    padding: 0.45em 0.7em;
    font-size: 0.9rem;
    font-family: inherit;
    cursor: pointer;
    font-weight: 600;
    background: #396cd8;
    color: #fff;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  button.danger {
    border-color: #e5484d;
    background: #e5484d;
  }
  button.ghost {
    background: transparent;
    color: inherit;
    border-color: #aaa;
  }
  @media (prefers-color-scheme: dark) {
    .modal {
      background: #2a2a2a;
      color: #f0f0f0;
    }
    .plan-list {
      background: #1e1e1e;
      border-color: #3a3a3a;
    }
    .mig-disclaimer {
      background: #3a1f20;
    }
  }
</style>
