<script lang="ts">
  // Skeleton rendered immediately on file click while diff_file is in flight.
  // Matches SideBySide's outer layout exactly so the transition from skeleton → real SideBySide
  // is a seamless in-place fill (same DOM position, same dimensions, same structure).
  let { name, onback }: { name: string; onback: () => void } = $props();
</script>

<div class="sbs-wrapper">
  <!-- Header: same as SideBySide's .fhead -->
  <div class="fhead">
    <button class="ghost back" type="button" onclick={onback} aria-label="back">←</button>
    <span class="fname">{name}</span>
  </div>

  <!-- Navigation bar: same layout as SideBySide's .bar, all controls disabled -->
  <div class="bar">
    <span class="grp">
      <button class="ghost nav" disabled aria-label="previous change">↑</button>
      <button class="ghost nav" disabled aria-label="next change">↓</button>
      <span class="nav-count">—</span>
    </span>
    <span class="spacer"></span>
  </div>

  <!-- Two side-by-side panels with spinners, matching .sbs structure -->
  <div class="sbs-panels">
    <div class="sbs-panel"><span class="spinner"></span></div>
    <div class="sbs-gutter"></div>
    <div class="sbs-panel"><span class="spinner"></span></div>
  </div>
</div>

<style>
  /* Mirror SideBySide's outer layout so the transition is seamless. */
  .sbs-wrapper {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .fhead {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.3rem 0;
  }
  .fname {
    font-family: ui-monospace, monospace;
    font-size: 0.9rem;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 0.5rem 0.8rem;
    padding: 0.4rem 0;
  }
  .grp { display: inline-flex; align-items: center; gap: 0.3rem; }
  .spacer { flex: 1; }
  .nav-count { font-size: 0.78rem; opacity: 0.4; min-width: 3rem; text-align: center; }
  /* Panel area: same border/radius as .sbs */
  .sbs-panels {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    border: 1px solid #ddd;
    border-radius: 8px;
    overflow: hidden;
    background: #fff;
  }
  .sbs-panel {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0.4;
  }
  .sbs-gutter {
    width: 48px;
    flex-shrink: 0;
    border-left: 1px solid #bbb;
    border-right: 1px solid #bbb;
    background: #f8f8f8;
  }
  /* Spinner */
  .spinner {
    display: inline-block;
    width: 24px;
    height: 24px;
    border: 2px solid rgba(128, 128, 128, 0.2);
    border-top-color: #888;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  /* Buttons */
  button {
    border-radius: 7px;
    border: 1px solid #ccc;
    padding: 0.35em 0.7em;
    font-size: 0.9rem;
    font-family: inherit;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  button:disabled { opacity: 0.35; cursor: default; }
  button.ghost { background: transparent; border-color: #aaa; color: inherit; }
  button.back { padding: 0.15em 0.55em; line-height: 1; font-size: 1rem; }
  button.nav  { padding: 0.2em 0.55em; font-size: 0.85rem; }
  /* Dark mode */
  @media (prefers-color-scheme: dark) {
    .sbs-panels { background: #232323; border-color: #3a3a3a; }
    .sbs-gutter  { border-color: #444; background: #1e1e1e; }
  }
</style>
