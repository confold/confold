<script lang="ts">
  // The Migrate dry-run plan review modal: per-item checkboxes, virtualized action list, category counts.
  // Extracted from +page.svelte (C1-full). The `checked` SvelteSet is owned by the parent and mutated
  // here in place — SvelteSet mutations are reactive across the component boundary, so the parent's
  // derived state (apply split, counters) stays in sync. Counts + virtualization are local.
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { slugOf } from "$lib/sources";
  import type { SourceSpec } from "$lib/types";
  import { planItems, uncheckedCount, actionKey, type MigrateAction } from "$lib/migrate";

  let {
    actions,
    checked,
    originSpec,
    destSpec,
    destructive,
    moveRequested,
    scrollTop = $bindable(0),
    onback,
    onclose,
    onapply,
    onview,
  }: {
    /** The planned actions (non-null: the parent only mounts this when a plan exists). */
    actions: MigrateAction[];
    /** Per-action checkbox state, keyed by `actionKey`. Mutated in place (reactive SvelteSet). */
    checked: SvelteSet<string>;
    originSpec: SourceSpec | null;
    destSpec: SourceSpec | null;
    /** Whether a destructive flag is on → show the backup disclaimer. */
    destructive: boolean;
    /** MOVE (M2) requested in config → surface what will happen to the origin (and whether edits void it). */
    moveRequested: boolean;
    onback: () => void;
    onclose: () => void;
    onapply: () => void;
    /** Open a "different" action in the read-only side-by-side. */
    onview: (action: MigrateAction) => void;
    /** Virtualized-list scroll position — owned by the parent so it survives leaving for a file view. */
    scrollTop?: number;
  } = $props();

  // Item counts use item_count (not action count) so a dir of 45 files shows as 45, not 1.
  const planNew = $derived(planItems(actions, "new"));
  const planDifferent = $derived(planItems(actions, "different"));
  const planDeletes = $derived(planItems(actions, "extra"));
  const planTotalItems = $derived(planNew + planDifferent + planDeletes);
  const planSkipped = $derived(uncheckedCount(actions, checked));
  // Move is all-or-nothing: the origin is deleted only if it's requested AND no item is unchecked. So
  // the per-row "origin → delete" indicator is global — it lights up only when the plan is intact, and
  // reverts to "no change" the moment any box is cleared (matches the backend gate).
  const moveActive = $derived(moveRequested && planSkipped === 0);

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
    <h2>Migration plan</h2>

    <p class="mig-dir">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span> {slugOf(originSpec).label}{/if}
      <span class="arrow">→</span>
      {#if destSpec}<span class="ico">{slugOf(destSpec).icon}</span> {slugOf(destSpec).label}{/if}
    </p>

    <!-- Category counts -->
    <div class="plan-counts">
      {#if actions.length === 0}
        <span class="plan-ok">✓ Destination already matches origin for the selected options.</span>
      {:else}
        {#if planNew > 0}<span class="plan-copy">+{planNew} new</span>{/if}
        {#if planDifferent > 0}<span class="plan-override">{planDifferent} override</span>{/if}
        {#if planDeletes > 0}<span class="plan-del">✕ {planDeletes} delete</span>{/if}
        <span class="plan-total">{planTotalItems} item{planTotalItems === 1 ? "" : "s"} total</span>
        {#if planSkipped > 0}<span class="plan-skipped">{planSkipped} unchecked</span>{/if}
      {/if}
    </div>

    <!-- Virtualized action list -->
    {#if actions.length > 0}
      <!-- Column header: ✓ | path | Orig | Dest -->
      <div class="plan-header">
        <span class="plan-check-col"></span>
        <span class="plan-path-col">Path</span>
        <span class="plan-side-label">Orig</span>
        <span class="plan-side-label">Dest</span>
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
              class:plan-row-new={v.a.reason === "new"}
              class:plan-row-diff={v.a.reason === "different"}
              class:plan-row-del={v.a.reason === "extra"}
              class:plan-row-clickable={v.a.reason === "different"}
              class:plan-row-unchecked={!checked.has(actionKey(v.a))}
              style="top: {v.index * ROW_H}px"
              onclick={v.a.reason === "different" ? () => onview(v.a) : undefined}
              title={v.a.reason === "different" ? "Click to view diff" : undefined}
            >
              <!-- Checkbox: stops propagation so clicking it doesn't open the file view -->
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <span class="plan-check-col" onclick={(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={checked.has(actionKey(v.a))}
                  onchange={(e) => toggle(actionKey(v.a), (e.target as HTMLInputElement).checked)}
                />
              </span>
              <!-- Path -->
              <span class="plan-path">{v.a.rel_path.join("/")}{v.a.is_dir ? "/" : ""}{#if v.a.is_dir}<span class="plan-badge">dir • {v.a.item_count} item{v.a.item_count === 1 ? "" : "s"}</span>{/if}</span>
              <!-- Orig: "✕ delete" when Move is active (origin file will be removed), "—" if the file
                   isn't in origin (extra), else "no change". -->
              {#if v.a.reason === "extra"}
                <span class="plan-cell plan-muted">—</span>
              {:else if moveActive}
                <span class="plan-cell plan-danger">✕ delete</span>
              {:else}
                <span class="plan-cell plan-muted">no change</span>
              {/if}
              <!-- Dest column -->
              {#if v.a.reason === "new"}
                <span class="plan-cell plan-copy">→ copy</span>
              {:else if v.a.reason === "different"}
                <span class="plan-cell plan-override">→ override</span>
              {:else}
                <span class="plan-cell plan-danger">✕ delete</span>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <!-- MOVE (M2) status: confirm the origin-delete, or warn that editing the plan voids it. -->
    {#if moveRequested && actions.length > 0}
      {#if planSkipped > 0}
        <div class="mig-move-warn">
          <p>⚠ <strong>Origin-delete skipped.</strong> The migration <strong>still runs as planned</strong>,
             but because you unchecked {planSkipped} item{planSkipped === 1 ? "" : "s"} the plan was modified,
             so the origin is <strong>kept</strong> for you to review later. Re-check every item to delete the
             origin too.</p>
        </div>
      {:else}
        <div class="mig-move-on">
          <p>↪ <strong>Move:</strong> after copying, every file is re-verified and the <strong>entire
             origin is deleted</strong> if it is fully identical in destination (minus exceptions).</p>
        </div>
      {/if}
    {/if}

    <!-- Disclaimer (again before the action button) -->
    {#if destructive}
      <div class="mig-disclaimer">
        <p>This migration can <strong>overwrite and/or delete</strong> data and may fail partway.
           <strong>Back up both source and destination first.</strong></p>
      </div>
    {/if}

    <div class="modal-actions">
      <button class="ghost" onclick={onback}>← Back</button>
      {#if actions.length === 0}
        <button class="ghost" onclick={onclose}>Close</button>
      {:else}
        <button class="danger" onclick={onapply}>Migrate →</button>
      {/if}
    </div>
  </div>
</div>

<style>
  /* Shared modal chrome (copied from +page; +page keeps its own copies for the other modals). */
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
  .mig-move-warn,
  .mig-move-on {
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
  }
  .mig-move-warn p,
  .mig-move-on p { margin: 0; }
  .mig-move-warn { background: #fff4e5; border: 1px solid #e8a33d; }
  .mig-move-on   { background: #fdecec; border: 1px solid #e5484d; }
  /* Category counts */
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
  .plan-counts .plan-copy     { color: #4c8bf5; font-weight: 700; }
  .plan-counts .plan-override { color: #e8a33d; font-weight: 700; }
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
  /* Plan column header */
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
  /* Plan rows — ✓ | path | Orig | Dest */
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
  .plan-row-new  { background: rgba(76, 139, 245, 0.05); }
  .plan-row-diff { background: rgba(232, 163, 61, 0.07); }
  .plan-row-del  { background: rgba(229, 72,  77, 0.05); }
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
  .plan-copy     { color: #4c8bf5; }
  .plan-override { color: #e8a33d; }
  .plan-danger   { color: #e5484d; }
  .plan-muted    { opacity: 0.35; }
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
  /* Buttons */
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
    /* Dark the disclaimer too, or the light text becomes invisible on its light base background. */
    .mig-disclaimer {
      background: #3a1f20;
    }
    .mig-move-warn { background: #3a2e1a; }
    .mig-move-on { background: #3a1f20; }
  }
</style>
