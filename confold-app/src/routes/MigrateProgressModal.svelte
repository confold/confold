<script lang="ts">
  // The Migrate apply-progress modal: live streamed outcomes + final summary. Extracted from +page.svelte
  // (C1-full). Pure display + two callbacks; the per-category counters are derived here from `outcomes`
  // via the shared pure helpers, so the parent only passes raw state.
  import { slugOf } from "$lib/sources";
  import type { SourceSpec } from "$lib/types";
  import {
    appliedByReason,
    appliedCount,
    skippedCount,
    type MigrateOutcome,
    type MigrateSummary,
    type MoveSummary,
  } from "$lib/migrate";

  let {
    outcomes,
    applying,
    doneSummary,
    moveSummary,
    phase,
    originSpec,
    destSpec,
    checkedTotal,
    oncancel,
    oncontinue,
  }: {
    /** Per-operation outcomes (applied + pre-populated skipped), in arrival order. */
    outcomes: MigrateOutcome[];
    /** True while the background apply is still running (shows the spinner + Cancel). */
    applying: boolean;
    /** Final tally once done/cancelled; null while still applying. */
    doneSummary: MigrateSummary | null;
    /** MOVE (M2) result, present only when a move was requested; null otherwise. */
    moveSummary: MoveSummary | null;
    /** Current long backend phase, for the status line during the M2 re-verification. */
    phase: "" | "verifying" | "emptying_origin";
    originSpec: SourceSpec | null;
    destSpec: SourceSpec | null;
    /** Sum of item_count over the checked plan actions — the "≈N" reference total. */
    checkedTotal: number;
    oncancel: () => void;
    oncontinue: () => void;
  } = $props();

  const progressNew = $derived(appliedByReason(outcomes, "new"));
  const progressDifferent = $derived(appliedByReason(outcomes, "different"));
  const progressDeletes = $derived(appliedByReason(outcomes, "extra"));
  const progressApplied = $derived(appliedCount(outcomes));
  const progressSkipped = $derived(skippedCount(outcomes));
  // Origin-delete (move) operations are tallied separately: they aren't part of the plan's item_count,
  // so folding them into "applied / ≈total" would make the ratio nonsensical (e.g. 269 / ≈6).
  const progressMoved = $derived(appliedByReason(outcomes, "moved"));
  const planApplied = $derived(progressNew + progressDifferent + progressDeletes);
  // Index of the first origin-delete outcome → where to insert the "re-verified" divider, marking the
  // boundary between the migration ops and the post-verify origin cleanup.
  const firstMovedIdx = $derived(outcomes.findIndex((o) => o.reason === "moved" && !o.skipped));

  // Per-row label carries direction (← / →) from the op, so the shared modal reads correctly for both
  // one-directional Migrate and bidirectional Sync. Falls back to a reason-based label if op is absent.
  function opLabel(o: MigrateOutcome): string {
    if (o.reason === "moved") return "✕ origin";
    switch (o.op) {
      case "copy_left_to_right": return o.reason === "different" ? "→ override" : "→ copy";
      case "copy_right_to_left": return o.reason === "different" ? "← override" : "← copy";
      case "delete_left": return "✕ left";
      case "delete_right": return "✕ right";
      default:
        return o.reason === "new" ? "→ copy" : o.reason === "different" ? "→ override" : "✕ delete";
    }
  }

  let listEl = $state<HTMLElement | undefined>();
  // Auto-scroll to the bottom as outcomes arrive. Gated only on the element existing, so the final item
  // still scrolls in when the last event lands in the same tick as applying flips off.
  $effect(() => {
    outcomes.length; // track
    if (listEl) listEl.scrollTop = listEl.scrollHeight;
  });
</script>

<div class="overlay">
  <div class="modal mig">
    <!-- Header -->
    {#if applying && phase === "verifying"}
      <h2>Verifying origin… <span class="spinner mig-spinner"></span></h2>
    {:else if applying && phase === "emptying_origin"}
      <h2>Emptying origin… <span class="spinner mig-spinner"></span></h2>
    {:else if applying}
      <h2>Migrating… <span class="spinner mig-spinner"></span></h2>
    {:else if doneSummary?.cancelled}
      <h2>Migration cancelled</h2>
    {:else if moveSummary?.origin_deleted}
      <h2>Migration complete — origin moved ✓</h2>
    {:else}
      <h2>Migration {doneSummary?.failed ? "completed with errors" : "complete ✓"}</h2>
    {/if}

    <p class="mig-dir">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span> {slugOf(originSpec).label}{/if}
      <span class="arrow">→</span>
      {#if destSpec}<span class="ico">{slugOf(destSpec).icon}</span> {slugOf(destSpec).label}{/if}
    </p>

    <!-- Per-category counts (live while running, final when done) -->
    <div class="plan-counts">
      {#if progressNew > 0}<span class="plan-copy">copy {progressNew}</span>{/if}
      {#if progressDifferent > 0}<span class="plan-override">override {progressDifferent}</span>{/if}
      {#if progressDeletes > 0}<span class="plan-del">✕ delete {progressDeletes}</span>{/if}
      {#if planApplied > 0 || progressSkipped > 0}
        <span class="plan-total">
          {planApplied}{checkedTotal > 0 ? ` / ≈${checkedTotal}` : ""} applied{progressSkipped > 0 ? ` · ${progressSkipped} skipped` : ""}
        </span>
      {/if}
      {#if progressMoved > 0}<span class="plan-del">✕ origin {progressMoved}</span>{/if}
      {#if doneSummary?.failed}<span class="plan-del">{doneSummary.failed} failed</span>{/if}
      {#if doneSummary?.cancelled && !applying}<span class="plan-total">· cancelled</span>{/if}
    </div>

    <!-- Outcome list (scrollable, auto-follows bottom) -->
    <div class="mig-progress-list" bind:this={listEl}>
      {#each outcomes as o, i (o.path + o.reason)}
        {#if i === firstMovedIdx}
          <div class="mig-verify-divider">✓ Re-verified — all origin files identical in destination. Now deleting the origin:</div>
        {/if}
        <div class="mig-outcome" class:mig-ok={o.ok && !o.skipped} class:mig-fail={!o.ok} class:mig-skip={o.skipped}>
          <span class="mig-outcome-icon">{o.skipped ? "—" : o.ok ? "✓" : "✗"}</span>
          <span class="mig-outcome-path">{o.path}</span>
          <span class="mig-outcome-op"
            class:plan-copy={o.reason === "new"}
            class:plan-override={o.reason === "different"}
            class:plan-danger={o.reason === "extra" || o.reason === "moved"}
          >{opLabel(o)}</span>
          {#if o.error}<span class="mig-outcome-err" title={o.error}>⚠</span>{/if}
        </div>
      {/each}
      {#if applying}
        <div class="mig-outcome mig-scanning">
          <span class="spinner"></span>
          <span>{phase === "verifying" ? "re-verifying every file…" : phase === "emptying_origin" ? "deleting origin…" : "applying…"}</span>
        </div>
      {/if}
    </div>

    <!-- MOVE (M2) result -->
    {#if moveSummary && !applying}
      {#if moveSummary.origin_deleted}
        <div class="mig-move-result ok">
          <p>↪ <strong>Origin moved.</strong> Deleted {moveSummary.files_deleted}
            file{moveSummary.files_deleted === 1 ? "" : "s"}{moveSummary.dirs_pruned > 0 ? ` and pruned ${moveSummary.dirs_pruned} empty folder${moveSummary.dirs_pruned === 1 ? "" : "s"}` : ""} from the origin.</p>
        </div>
      {:else if moveSummary.attempted && moveSummary.blockers.length > 0}
        <div class="mig-move-result kept">
          <p>↪ <strong>Origin kept.</strong> The move was skipped because not every item is verified
             identical in destination (all-or-nothing):</p>
          <ul>
            {#each moveSummary.blockers as b}<li>{b}</li>{/each}
          </ul>
        </div>
      {:else if moveSummary.cancelled}
        <div class="mig-move-result kept">
          <p>↪ <strong>Origin kept</strong> — the move was cancelled before deleting anything.</p>
        </div>
      {/if}
    {/if}

    <div class="modal-actions">
      {#if applying}
        <button class="danger" onclick={oncancel}>Cancel</button>
      {:else}
        <button onclick={oncontinue}>Continue → verify comparison</button>
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
  .plan-counts .plan-copy     { color: #4c8bf5; font-weight: 700; }
  .plan-counts .plan-override { color: #e8a33d; font-weight: 700; }
  /* Standalone op colors (used on the per-outcome op chip). */
  .plan-copy     { color: #4c8bf5; }
  .plan-override { color: #e8a33d; }
  .plan-danger   { color: #e5484d; }
  /* Progress list */
  .mig-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(128, 128, 128, 0.25);
    border-top-color: #888;
    border-radius: 50%;
    animation: spin 0.65s linear infinite;
    vertical-align: middle;
    margin-left: 0.3em;
  }
  .mig-progress-list {
    border: 1px solid rgba(128, 128, 128, 0.25);
    border-radius: 8px;
    background: #f9f9f9;
    overflow-y: auto;
    flex: 1 1 auto;
    min-height: 160px;
    font-family: ui-monospace, monospace;
    font-size: 0.76rem;
  }
  .mig-outcome {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.7rem;
    height: 24px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.08);
  }
  .mig-ok   { border-left: 3px solid #2ea043; }
  .mig-fail { border-left: 3px solid #e5484d; background: rgba(229,72,77,0.05); }
  .mig-skip { border-left: 3px solid transparent; opacity: 0.4; }
  .mig-scanning { opacity: 0.6; border-left: 3px solid transparent; gap: 0.4rem; }
  .mig-verify-divider {
    padding: 0.35rem 0.7rem;
    font-size: 0.72rem;
    font-weight: 700;
    color: #2ea043;
    background: rgba(46, 160, 67, 0.08);
    border-top: 1px solid rgba(46, 160, 67, 0.3);
    border-bottom: 1px solid rgba(46, 160, 67, 0.3);
    white-space: normal;
  }
  .mig-outcome-icon { flex: none; width: 1em; font-weight: 700; }
  .mig-ok   .mig-outcome-icon { color: #2ea043; }
  .mig-fail .mig-outcome-icon { color: #e5484d; }
  .mig-outcome-path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mig-outcome-op {
    flex: none;
    font-size: 0.72rem;
    font-weight: 600;
  }
  .mig-outcome-err {
    flex: none;
    color: #e8a33d;
    cursor: help;
    font-size: 0.8rem;
  }
  /* MOVE (M2) result block */
  .mig-move-result {
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
  }
  .mig-move-result p { margin: 0; }
  .mig-move-result ul { margin: 0.3rem 0 0; padding-left: 1.2rem; max-height: 8rem; overflow-y: auto; }
  .mig-move-result li { font-family: ui-monospace, monospace; font-size: 0.74rem; }
  .mig-move-result.ok   { background: #e8f5e9; border: 1px solid #2ea043; }
  .mig-move-result.kept { background: #fff4e5; border: 1px solid #e8a33d; }
  /* Buttons (modal-actions) */
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
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-color-scheme: dark) {
    .modal {
      background: #2a2a2a;
      color: #f0f0f0;
    }
    .mig-progress-list {
      background: #1e1e1e;
      border-color: #3a3a3a;
    }
    .mig-move-result.ok   { background: #18301c; }
    .mig-move-result.kept { background: #3a2e1a; }
  }
</style>
