<script lang="ts">
  // Shared chrome for every file view (text / hex / image): a back button, the file name, an optional
  // view-specific right slot, and the plan-review OK/Skip (or a read-only badge). Keeps the three views
  // visually consistent; each view renders its own content-specific controls bar BELOW this header.
  import type { Snippet } from "svelte";

  let {
    name,
    onback,
    readOnly = false,
    onkeep,
    onskip,
    right,
  }: {
    name: string;
    onback?: () => void;
    /** Read-only inspection (no OK/Skip) → show a "read-only" badge instead. */
    readOnly?: boolean;
    /** Plan-review decision callbacks (shown as OK / ✕ Skip when provided). */
    onkeep?: () => void;
    onskip?: () => void;
    /** Optional view-specific right-side content (e.g. the text view's change counts). */
    right?: Snippet;
  } = $props();
</script>

<div class="fvh">
  {#if onback}
    <button class="ghost back" type="button" onclick={onback} title="Back" aria-label="back">←</button>
  {/if}
  <span class="fvh-name">{name}</span>
  <span class="spacer"></span>
  {#if right}{@render right()}{/if}
  {#if onskip || onkeep}
    {#if onskip}<button class="ghost skip-btn" onclick={onskip}>✕ Skip</button>{/if}
    {#if onkeep}<button class="keep-btn" onclick={onkeep}>OK</button>{/if}
  {:else if readOnly}
    <span class="ro-badge">read-only</span>
  {/if}
</div>

<style>
  .fvh {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0;
    flex: none;
  }
  .fvh-name {
    font-family: ui-monospace, monospace;
    font-size: 0.9rem;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1 1 auto;
  }
  button {
    border-radius: 7px;
    border: 1px solid #aaa;
    font-family: inherit;
    cursor: pointer;
    font-weight: 600;
  }
  .ghost {
    background: transparent;
    color: inherit;
    padding: 0.3em 0.7em;
    font-size: 0.82rem;
  }
  .back {
    padding: 0.15em 0.55em;
    line-height: 1;
    font-size: 1rem;
  }
  .keep-btn {
    border-color: #396cd8;
    background: #396cd8;
    color: #fff;
    font-size: 0.82rem;
    padding: 0.3em 0.7em;
  }
  .skip-btn {
    border-color: #e5484d;
    background: transparent;
    color: #e5484d;
    font-size: 0.82rem;
    padding: 0.3em 0.7em;
  }
  .ro-badge {
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.55;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 4px;
    padding: 0.1em 0.4em;
  }
</style>
