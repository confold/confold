<script lang="ts">
  // The Sync config modal: comparison method, exceptions, the two "trust" checkboxes, the conflict rule
  // (only when both sides are trusted), the "delete differences" option (only when exactly one side is
  // trusted), and the backup ack. Mirrors MigrateConfigPanel's shape; Sync is inherently destructive
  // (it overwrites the losing side of a conflict), so the backup ack is always required once a side is
  // trusted. At least one side must be trusted to preview.
  import type { CompareOpts, SourceSpec } from "$lib/types";
  import { slugOf } from "$lib/sources";

  type ConflictRule = "newer" | "larger" | "manual";
  type Flags = {
    trust_left: boolean;
    trust_right: boolean;
    delete_diffs: boolean;
    conflict_rule: ConflictRule;
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
    onchange: () => void;
    onpreview: () => void;
    onclose: () => void;
  } = $props();

  const trustedCount = $derived((flags.trust_left ? 1 : 0) + (flags.trust_right ? 1 : 0));
  const bothTrusted = $derived(trustedCount === 2);
  const oneTrusted = $derived(trustedCount === 1);
  // The trusted side(s) propagate over the other → Sync can overwrite. Backup ack required once valid.
  const valid = $derived(trustedCount >= 1);

  function onTrustChange() {
    // "Delete differences" only makes sense with a single authoritative side; clear it otherwise.
    if ((flags.trust_left ? 1 : 0) + (flags.trust_right ? 1 : 0) !== 1) flags.delete_diffs = false;
    onchange();
  }
</script>

<div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal mig">
    <h2>Sync</h2>

    <p class="mig-dir">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span> {slugOf(originSpec).label}{/if}
      <span class="arrow">⇄</span>
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
      <legend>Which side(s) do you trust?</legend>
      <label><input type="checkbox" bind:checked={flags.trust_left} onchange={onTrustChange} />
        Trust {originSpec ? slugOf(originSpec).label : "left"} — its content is pushed to the other side</label>
      <label><input type="checkbox" bind:checked={flags.trust_right} onchange={onTrustChange} />
        Trust {destSpec ? slugOf(destSpec).label : "right"} — its content is pushed to the other side</label>
      {#if trustedCount === 0}
        <p class="mig-warn">Select at least one trusted side to preview.</p>
      {/if}

      {#if bothTrusted}
        <hr class="mig-sep" />
        <label class="mig-rule">When a file differs on <em>both</em> sides (conflict), keep:
          <select bind:value={flags.conflict_rule} onchange={onchange}>
            <option value="manual">neither — I'll resolve each by hand (default)</option>
            <option value="newer">the newer one (by date)</option>
            <option value="larger">the larger one (by size)</option>
          </select>
        </label>
        <p class="mig-move-note">Conflicts that can't be auto-resolved (manual rule, or a tie) are left
          untouched and surfaced in the comparison view afterward, for you to merge by hand.</p>
      {/if}

      {#if oneTrusted}
        <hr class="mig-sep" />
        <label><input type="checkbox" bind:checked={flags.delete_diffs} onchange={onchange} />
          <strong>Delete differences</strong> — remove items the trusted side doesn't have
          <span class="mig-danger">⚠ deletes</span></label>
      {/if}
    </fieldset>

    {#if valid}
      <div class="mig-disclaimer">
        <p>Sync can <strong>overwrite</strong> the losing side of a conflict{flags.delete_diffs ? " and <strong>delete</strong> the untrusted side's extras" : ""}, and may fail partway.
           <strong>Back up both sides first.</strong></p>
        <label><input type="checkbox" bind:checked={backupAck} /> I have a backup of both sides</label>
      </div>
    {/if}

    <div class="modal-actions">
      <button class="ghost" onclick={onclose}>Close</button>
      <button onclick={onpreview} disabled={busy || !valid || !backupAck}>
        {busy ? "Scanning…" : "Preview sync"}
      </button>
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
  .mig-rule {
    flex-direction: column;
    align-items: flex-start;
    gap: 0.25rem;
    font-weight: 600;
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
    width: 100%;
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
