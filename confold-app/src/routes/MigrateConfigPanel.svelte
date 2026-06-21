<script lang="ts">
  // The Migrate config modal: comparison method, exceptions, the 3 op toggles, and the backup ack that
  // gates Preview. Extracted from +page.svelte (C1-full). Two-way state (method/exclude/flags/backupAck)
  // is `$bindable` so the parent keeps ownership; `onchange` lets the parent clear a stale preview on any
  // change. `destructive` is derived locally from the (bound) flags.
  import type { CompareOpts, SourceSpec } from "$lib/types";
  import { slugOf } from "$lib/sources";

  type Flags = {
    copy_new: boolean;
    overwrite_different: boolean;
    delete_extra: boolean;
    /** MOVE (M2): after copy, delete the whole origin if it is fully verified identical in destination. */
    delete_origin: boolean;
  };

  let {
    method = $bindable(),
    exclude = $bindable(),
    flags = $bindable(),
    backupAck = $bindable(),
    busy,
    originSpec,
    destSpec,
    onchange,
    onpreview,
    onclose,
  }: {
    method: CompareOpts["method"];
    exclude: string;
    flags: Flags;
    backupAck: boolean;
    busy: boolean;
    originSpec: SourceSpec | null;
    destSpec: SourceSpec | null;
    /** Called on any config change so the parent can drop a stale preview. */
    onchange: () => void;
    onpreview: () => void;
    onclose: () => void;
  } = $props();

  // Any destructive op selected → the backup acknowledgement is required before Preview.
  const destructive = $derived(flags.overwrite_different || flags.delete_extra || flags.delete_origin);

  // MOVE requires a fully-identical destination, so Copy + Overwrite must be on (else origin can never
  // be verified identical and the move would always abort). Force + lock them while Move is selected.
  function onMoveToggle() {
    if (flags.delete_origin) {
      flags.copy_new = true;
      flags.overwrite_different = true;
    }
    onchange();
  }
</script>

<div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal mig">
    <h2>Migrate</h2>

    <p class="mig-dir">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span> {slugOf(originSpec).label}{/if}
      <span class="arrow">→</span>
      {#if destSpec}<span class="ico">{slugOf(destSpec).icon}</span> {slugOf(destSpec).label}{/if}
    </p>

    <label class="mig-row">Comparison method
      <select bind:value={method} onchange={onchange}>
        <option value="full">full (slow, safe — recommended)</option>
        <option value="quick">quick</option>
        <option value="size">size</option>
        <option value="mtime">mtime</option>
        <option value="size-mtime">size-mtime</option>
      </select>
    </label>
    {#if method !== "full"}
      <p class="mig-warn">⚠ Non-full methods can miss content differences (same size/date, different bytes).</p>
    {/if}

    <label class="mig-row">Exceptions (untouched on both sides)
      <input class="ex" bind:value={exclude} onchange={onchange} placeholder="*.tmp, node_modules" spellcheck="false" />
    </label>

    <fieldset class="mig-ops">
      <legend>What to do (origin → destination)</legend>
      <label><input type="checkbox" bind:checked={flags.copy_new} onchange={onchange} disabled={flags.delete_origin} /> Copy files only in origin</label>
      <label><input type="checkbox" bind:checked={flags.overwrite_different} onchange={onchange} disabled={flags.delete_origin} /> Overwrite files that differ in destination <span class="mig-danger">⚠ overwrites</span></label>
      <label><input type="checkbox" bind:checked={flags.delete_extra} onchange={onchange} /> Delete files only in destination <span class="mig-danger">⚠ deletes</span></label>
      <hr class="mig-sep" />
      <label class="mig-move"><input type="checkbox" bind:checked={flags.delete_origin} onchange={onMoveToggle} /> <strong>Move</strong> — delete the origin after copying <span class="mig-danger">⚠ deletes origin</span></label>
      {#if flags.delete_origin}
        <p class="mig-move-note">After the copy, every file is re-verified (full byte compare). The origin is
          deleted <strong>only if the whole tree is verified identical in destination</strong> (minus
          exceptions) — all-or-nothing. Requires Copy + Overwrite (locked on).</p>
      {/if}
    </fieldset>

    {#if destructive}
      <div class="mig-disclaimer">
        <p>This migration can <strong>overwrite and/or delete</strong> data and may fail partway.
           <strong>Back up both source and destination first.</strong></p>
        {#if flags.delete_origin}
          <p><strong>Move is on:</strong> on success the entire origin will be <strong>deleted</strong>.</p>
        {/if}
        <label><input type="checkbox" bind:checked={backupAck} /> I have a backup of both sides</label>
      </div>
    {/if}

    <div class="modal-actions">
      <button class="ghost" onclick={onclose}>Close</button>
      <button onclick={onpreview} disabled={busy || (destructive && !backupAck)}>
        {busy ? "Scanning…" : "Preview migration"}
      </button>
    </div>
  </div>
</div>

<style>
  /* Shared modal chrome (copied from +page; +page keeps its own copies for the non-migrate modals). */
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
  .mig-row {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-weight: 600;
    font-size: 0.85rem;
  }
  .mig-row .ex {
    width: 100%;
    box-sizing: border-box;
  }
  .mig-warn {
    color: #e8a33d;
    font-size: 0.78rem;
    margin: 0;
  }
  .mig-ops {
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .mig-ops legend {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    opacity: 0.6;
    padding: 0 0.3rem;
  }
  .mig-ops label {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
  }
  .mig-danger {
    color: #e5484d;
    font-size: 0.74rem;
    font-weight: 700;
  }
  .mig-sep {
    border: none;
    border-top: 1px solid rgba(128, 128, 128, 0.25);
    margin: 0.2rem 0;
  }
  .mig-move strong {
    color: #c2410c;
  }
  .mig-move-note {
    margin: 0;
    font-size: 0.76rem;
    opacity: 0.8;
    line-height: 1.35;
  }
  .mig-disclaimer {
    background: #fdecec;
    border: 1px solid #e5484d;
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
  }
  .mig-disclaimer p { margin: 0 0 0.4rem; }
  .mig-disclaimer label { display: flex; align-items: center; gap: 0.45rem; font-weight: 600; }
  /* Form controls (these were styled by +page's global input/select rule). */
  input,
  select {
    border-radius: 7px;
    border: 1px solid #ccc;
    padding: 0.45em 0.7em;
    font-size: 0.9rem;
    font-family: inherit;
    background: #fff;
    color: inherit;
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
    .mig-disclaimer {
      background: #3a1f20;
    }
    input,
    select {
      background: #2a2a2a;
      border-color: #444;
      color: #f0f0f0;
    }
  }
</style>
