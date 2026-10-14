<script lang="ts">
  import { onMount } from "svelte";
  import { ArrowLeft, ChevronRight, Folder, LoaderCircle, Search } from "@lucide/svelte";
  import { commands } from "$lib/commands";
  import type { NuxeoRoute, SourceSpec } from "$lib/types";

  let {
    spec,
    startPath,
    onselect,
    oncancel,
  }: {
    spec: SourceSpec;
    startPath: string;
    onselect: (path: string) => void;
    oncancel: () => void;
  } = $props();

  let currentPath = $state("/");
  let query = $state("");
  let entries = $state<NuxeoRoute[]>([]);
  let pageIndex = $state(0);
  let hasMore = $state(false);
  let loading = $state(false);
  let error = $state("");
  let requestSequence = 0;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  const searching = $derived(query.trim().length > 0);
  const breadcrumbs = $derived(pathBreadcrumbs(currentPath));

  onMount(() => {
    currentPath = normalizePath(startPath);
    void loadPage(true);
    return () => {
      requestSequence++;
      if (searchTimer) clearTimeout(searchTimer);
    };
  });

  function normalizePath(path: string): string {
    const parts = path.split("/").filter(Boolean);
    return parts.length ? `/${parts.join("/")}` : "/";
  }

  function pathBreadcrumbs(path: string): Array<{ label: string; path: string }> {
    const crumbs = [{ label: "Repository", path: "/" }];
    let accumulated = "";
    for (const part of path.split("/").filter(Boolean)) {
      accumulated += `/${part}`;
      crumbs.push({ label: part, path: accumulated });
    }
    return crumbs;
  }

  async function loadPage(reset: boolean) {
    const seq = ++requestSequence;
    const index = reset ? 0 : pageIndex;
    const searchText = query.trim();
    if (reset) {
      entries = [];
      pageIndex = 0;
      hasMore = false;
    }
    loading = true;
    error = "";
    try {
      const page = searchText
        ? await commands.nuxeoSearchRoutes(spec, searchText, index)
        : await commands.nuxeoBrowseRoutes(spec, currentPath, index);
      if (seq !== requestSequence) return;
      entries = reset ? page.entries : [...entries, ...page.entries];
      hasMore = page.has_more;
      pageIndex = index + 1;
    } catch (e) {
      if (seq === requestSequence) error = String(e);
    } finally {
      if (seq === requestSequence) loading = false;
    }
  }

  function onSearchInput(value: string) {
    query = value;
    if (searchTimer) clearTimeout(searchTimer);
    // Invalidate the in-flight request BEFORE the debounce, so rows can't update under a label
    // that no longer describes them while the user is still typing.
    requestSequence++;
    loading = false;
    searchTimer = setTimeout(() => void loadPage(true), 300);
  }

  function navigate(path: string) {
    if (searchTimer) clearTimeout(searchTimer);
    query = "";
    currentPath = normalizePath(path);
    void loadPage(true);
  }

  function selectCurrent() {
    onselect(currentPath);
  }
</script>

<section class="route-picker" aria-label="Choose a Nuxeo document path">
  <div class="route-toolbar">
    <button class="ghost back" type="button" onclick={oncancel}>
      <ArrowLeft size={16} aria-hidden="true" />
      Back to source settings
    </button>
  </div>

  <nav class="breadcrumbs" aria-label="Current Nuxeo path">
    {#each breadcrumbs as crumb, index (crumb.path)}
      {#if index > 0}<ChevronRight size={14} aria-hidden="true" />{/if}
      <button
        type="button"
        class:current={index === breadcrumbs.length - 1 && !searching}
        onclick={() => navigate(crumb.path)}
        aria-current={index === breadcrumbs.length - 1 && !searching ? "location" : undefined}
      >
        {crumb.label}
      </button>
    {/each}
  </nav>

  <label class="route-search">
    <Search size={16} aria-hidden="true" />
    <span class="sr-only">Search folder names or absolute path prefixes</span>
    <input
      type="search"
      value={query}
      oninput={(event) => onSearchInput(event.currentTarget.value)}
      placeholder="Folder name or /absolute/path/prefix"
      maxlength="128"
      spellcheck="false"
    />
  </label>

  {#if searching}
    <p class="route-context">Search results for <strong>{query.trim()}</strong></p>
  {:else}
    <p class="route-context">Choose a folder or select the current path: <code>{currentPath}</code></p>
  {/if}

  <div class="route-results" aria-live="polite" aria-busy={loading}>
    {#if loading && entries.length === 0}
      <p class="route-state"><span class="spin" aria-hidden="true"><LoaderCircle size={16} /></span> Loading folders…</p>
    {:else if error}
      <div class="route-error" role="alert">
        <span>{error}</span>
        <button class="ghost sm" type="button" onclick={() => loadPage(true)}>Retry</button>
      </div>
    {:else if entries.length === 0}
      <p class="route-state">{searching ? "No matching folders." : "No child folders here."}</p>
    {:else if searching}
      <ul>
        {#each entries as route (route.path)}
          <li>
            <button class="route-row" type="button" onclick={() => onselect(route.path)}>
              <Folder size={17} aria-hidden="true" />
              <span class="route-copy">
                <strong>{route.title || route.name}</strong>
                <code>{route.path}</code>
              </span>
              <span class="route-use">Select</span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <ul>
        {#each entries as route (route.path)}
          <li>
            <button class="route-row" type="button" onclick={() => navigate(route.path)}>
              <Folder size={17} aria-hidden="true" />
              <span class="route-copy">
                <strong>{route.title || route.name}</strong>
                <code>{route.path}</code>
              </span>
              <span class="route-arrow" aria-hidden="true"><ChevronRight size={16} /></span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#if hasMore}
    <button class="ghost load-more" type="button" disabled={loading} onclick={() => loadPage(false)}>
      {loading ? "Loading…" : "Load more folders"}
    </button>
  {/if}

  <div class="route-footer">
    <span class="selected-path">Selected path: <code>{searching ? "Choose a result" : currentPath}</code></span>
    <button type="button" disabled={searching} onclick={selectCurrent}>Use this folder</button>
  </div>
</section>

<style>
  .route-picker {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.7rem;
  }

  button {
    border: 1px solid #396cd8;
    border-radius: 7px;
    background: #396cd8;
    color: #ffffff;
    padding: 0.45em 0.8em;
    font: inherit;
    font-size: 0.84rem;
    font-weight: 600;
    cursor: pointer;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  button:focus-visible,
  .route-search:focus-within {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
  }

  button.ghost {
    background: transparent;
    color: inherit;
    border-color: #aaaaaa;
  }

  button.sm {
    white-space: nowrap;
  }

  .route-toolbar,
  .route-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
  }

  .back,
  .route-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .breadcrumbs {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 0.15rem;
    overflow-x: auto;
    white-space: nowrap;
  }

  .breadcrumbs button {
    border: 0;
    background: transparent;
    color: inherit;
    padding: 0.2rem 0.35rem;
    font-size: 0.8rem;
  }

  .breadcrumbs button.current {
    font-weight: 700;
  }

  .route-search {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    border: 1px solid #cccccc;
    border-radius: 7px;
    padding: 0.4rem 0.65rem;
    background: #ffffff;
    color: inherit;
  }

  .route-search input {
    min-width: 0;
    flex: 1;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
  }

  .route-context {
    margin: 0;
    overflow-wrap: anywhere;
    font-size: 0.78rem;
    opacity: 0.72;
  }

  code {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
  }

  .route-results {
    min-height: 4rem;
    max-height: min(34vh, 18rem);
    overflow: auto;
    border: 1px solid rgba(128, 128, 128, 0.28);
    border-radius: 7px;
  }

  ul {
    margin: 0;
    padding: 0.2rem;
    list-style: none;
  }

  .route-row {
    width: 100%;
    min-width: 0;
    justify-content: flex-start;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: inherit;
    padding: 0.5rem 0.55rem;
    text-align: left;
  }

  .route-row:hover:not(:disabled) {
    background: rgba(128, 128, 128, 0.12);
  }

  .route-arrow,
  .route-use {
    flex: none;
    margin-left: auto;
    opacity: 0.65;
  }

  .route-copy {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 0.1rem;
  }

  .route-copy strong,
  .route-copy code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .route-copy strong {
    font-size: 0.86rem;
  }

  .route-copy code {
    opacity: 0.62;
  }

  .route-state,
  .route-error {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    min-height: 3.7rem;
    margin: 0;
    padding: 0.6rem;
    text-align: center;
    font-size: 0.84rem;
  }

  .route-error {
    flex-wrap: wrap;
    color: #e5484d;
  }

  .load-more {
    align-self: center;
  }

  .route-footer {
    flex-wrap: wrap;
    border-top: 1px solid rgba(128, 128, 128, 0.24);
    padding-top: 0.65rem;
  }

  .selected-path {
    min-width: 0;
    flex: 1;
    overflow-wrap: anywhere;
    font-size: 0.76rem;
    opacity: 0.74;
  }

  .selected-path code {
    overflow-wrap: anywhere;
  }

  .spin {
    display: inline-flex;
    animation: spin 0.9s linear infinite;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin { animation: none; }
  }

  @media (prefers-color-scheme: dark) {
    .route-search {
      border-color: #444444;
      background: #1e1e1e;
      color: #f0f0f0;
    }

    button.ghost {
      border-color: #777777;
    }
  }
</style>
