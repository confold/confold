<script lang="ts">
  // Modal to configure one side's data source: paste a URL to autofill, or pick a type → fill its form
  // (rendered from the backend `source_types()` catalog). Emits the assembled `SourceSpec` on confirm.
  import { commands } from "$lib/commands";
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import type { SourceTypeInfo, SourceSpec } from "$lib/types";
  import {
    parseSourceUrl,
    buildSpec,
    visibleFields,
    missingRequired,
    type FieldValues,
  } from "$lib/sources";

  let {
    title,
    types,
    initial = null,
    onconfirm,
    oncancel,
  }: {
    /** "Origen" / "Destino" — what we're configuring. */
    title: string;
    /** The source-type catalog (from the `source_types()` command). */
    types: SourceTypeInfo[];
    /** Pre-fill the form with this spec (e.g. a reused recent missing its secret) — secret fields stay empty. */
    initial?: SourceSpec | null;
    onconfirm: (spec: SourceSpec, isDir: boolean) => void;
    oncancel: () => void;
  } = $props();

  let url = $state("");
  let urlInput = $state<HTMLInputElement | undefined>();
  // Focus the URL field as soon as the picker opens, so you can paste straight away. If opened with an
  // `initial` spec (a reused recent), pre-select its type and fill its non-secret fields; the user just
  // re-enters the missing secret (which we never persisted).
  onMount(() => {
    urlInput?.focus();
    if (initial) {
      typeId = initial.kind;
      const t = types.find((x) => x.id === initial.kind);
      values = { ...(t ? seedDefaults(t) : {}), ...initial.fields };
    }
  });
  let typeId = $state<string | null>(null);
  let values = $state<FieldValues>({});

  // Connection auto-test: runs (debounced) once the required fields are filled; its result gates "Select".
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; is_dir: boolean; message: string } | null>(null);
  let testSeq = 0;
  // Set to true when the user pressed Enter while the test was still running — auto-confirms on success.
  let pendingConfirm = $state(false);

  const info = $derived(types.find((t) => t.id === typeId) ?? null);
  const fields = $derived(info ? visibleFields(info, values) : []);
  const missing = $derived(info ? missingRequired(info, values) : []);
  const canConfirm = $derived(info !== null && missing.length === 0 && testResult?.ok === true);

  // Whenever the form becomes complete, probe the connection (debounced; latest call wins via `testSeq`).
  // Also resets pendingConfirm so a queued Enter from a previous form state doesn't fire for a new one.
  $effect(() => {
    const ready = info !== null && missingRequired(info, values).length === 0;
    pendingConfirm = false;
    if (!ready) {
      testResult = null;
      testing = false;
      return;
    }
    const spec = buildSpec(typeId!, { ...values });
    const seq = ++testSeq;
    testing = true;
    testResult = null;
    const timer = setTimeout(async () => {
      try {
        const r = await commands.testSource(spec);
        if (seq === testSeq) {
          testResult = { ok: r.ok, is_dir: r.is_dir, message: r.message };
          testing = false;
        }
      } catch (e) {
        if (seq === testSeq) {
          testResult = { ok: false, is_dir: false, message: String(e) };
          testing = false;
        }
      }
    }, 450);
    return () => clearTimeout(timer);
  });

  // Auto-confirm once the test passes when the user had already pressed Enter.
  $effect(() => {
    if (pendingConfirm && canConfirm) {
      pendingConfirm = false;
      confirm();
    }
  });

  // Enter in any text/number/password input: confirm immediately if ready, else queue for test completion.
  function onEnter(e: KeyboardEvent) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    applyUrl(); // flush any pending URL edit before checking canConfirm
    if (canConfirm) {
      confirm();
    } else if (info !== null && missing.length === 0) {
      pendingConfirm = true;
    }
  }

  // Seed a type's declared defaults (e.g. SFTP port 22, root "/", auth method "password").
  function seedDefaults(t: SourceTypeInfo): FieldValues {
    const v: FieldValues = {};
    for (const f of t.fields) if (f.default != null) v[f.key] = f.default;
    return v;
  }

  function selectType(id: string) {
    typeId = id;
    const t = types.find((x) => x.id === id);
    values = t ? seedDefaults(t) : {};
  }

  // Paste-a-URL autofill: parse → select the type and prefill its fields over the type defaults.
  // Idempotent on the URL text: re-firing on blur with an unchanged value is a no-op, so it doesn't
  // reassign `values` (which would needlessly re-run the connection test) or clobber manual field edits.
  let lastAppliedUrl = "";
  function applyUrl() {
    if (url === lastAppliedUrl) return;
    lastAppliedUrl = url;
    const parsed = parseSourceUrl(url);
    if (!parsed) return;
    const t = types.find((x) => x.id === parsed.kind);
    values = { ...(t ? seedDefaults(t) : {}), ...parsed.fields };
    typeId = parsed.kind;
  }

  async function browse(key: string, directory: boolean) {
    const sel = await openDialog({
      directory,
      multiple: false,
      title: `Choose a ${directory ? "folder" : "file"}`,
    });
    if (typeof sel === "string") values[key] = sel;
  }

  function inputType(kind: string): string {
    return kind === "number" ? "number" : kind === "password" ? "password" : "text";
  }

  function confirm() {
    if (canConfirm && typeId) onconfirm(buildSpec(typeId, values), testResult?.is_dir ?? true);
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) oncancel();
  }}
>
  <div class="modal" role="dialog" aria-modal="true" aria-label={`Configure ${title}`}>
    <h2>{title}</h2>

    <label class="url">
      Paste a URL
      <input
        bind:this={urlInput}
        bind:value={url}
        oninput={applyUrl}
        onpaste={() => setTimeout(applyUrl, 0)}
        onkeydown={onEnter}
        placeholder="sftp://user@host/path  ·  s3://key:secret@host:port/bucket  ·  /local/path"
        spellcheck="false"
      />
    </label>

    <div class="types">
      {#each types as t (t.id)}
        <button class="type" class:on={typeId === t.id} type="button" onclick={() => selectType(t.id)}>
          <span class="ico">{t.icon}</span>{t.name}
        </button>
      {/each}
    </div>

    {#if info}
      <div class="form">
        {#each fields as f (f.key)}
          <label class="field">
            <span class="lbl">
              {f.label}{#if f.required}<span class="req">*</span>{/if}
              {#if f.secret}<span class="lock" title="Secret — not stored">🔒</span>{/if}
            </span>
            {#if f.kind === "select"}
              <select class:missing={missing.includes(f.key)} value={values[f.key] ?? ""} onchange={(e) => (values[f.key] = e.currentTarget.value)}>
                {#each f.options as opt (opt)}
                  <option value={opt}>{opt}</option>
                {/each}
              </select>
            {:else if f.kind === "textarea"}
              <textarea
                class:missing={missing.includes(f.key)}
                rows="4"
                value={values[f.key] ?? ""}
                oninput={(e) => (values[f.key] = e.currentTarget.value)}
                spellcheck="false"
              ></textarea>
            {:else}
              <span class="path-row">
                <input
                  class:missing={missing.includes(f.key)}
                  type={inputType(f.kind)}
                  value={values[f.key] ?? ""}
                  oninput={(e) => (values[f.key] = e.currentTarget.value)}
                  onkeydown={onEnter}
                  spellcheck="false"
                />
                {#if f.kind === "path" && typeId === "fs"}
                  <button class="ghost sm" type="button" onclick={() => browse(f.key, true)}>📁 Folder</button>
                  <button class="ghost sm" type="button" onclick={() => browse(f.key, false)}>📄 File</button>
                {/if}
              </span>
            {/if}
          </label>
        {/each}
      </div>
    {:else}
      <p class="hint">Choose a source type, or paste a URL above.</p>
    {/if}

    {#if info && missing.length === 0}
      <div class="status">
        {#if testing}
          <span class="testing">Testing connection…</span>
        {:else if testResult}
          {#if testResult.ok}
            <span class="ok">✓ {testResult.message}</span>
          {:else}
            <span class="err">✗ {testResult.message}</span>
          {/if}
        {/if}
      </div>
    {/if}

    <div class="actions">
      <button class="ghost" type="button" onclick={oncancel}>Cancel</button>
      <button type="button" disabled={!canConfirm} onclick={confirm}>Select {title.toLowerCase()}</button>
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
    z-index: 10;
  }
  .modal {
    background: #fff;
    color: #1a1a1a;
    border-radius: 10px;
    padding: 1.2rem;
    width: min(540px, 92vw);
    max-height: 86vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }
  h2 {
    margin: 0;
    font-size: 1.1rem;
  }
  label.url {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8rem;
    font-weight: 600;
    opacity: 0.85;
  }
  .types {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .type {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: rgba(128, 128, 128, 0.12);
    color: inherit;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 8px;
    padding: 0.4em 0.8em;
    font-weight: 600;
    cursor: pointer;
  }
  .type.on {
    background: #396cd8;
    color: #fff;
    border-color: #396cd8;
  }
  .type .ico {
    font-size: 1.1em;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.82rem;
    font-weight: 600;
  }
  .req {
    color: #e5484d;
    margin-left: 0.1em;
  }
  .lock {
    margin-left: 0.3em;
    opacity: 0.6;
    font-weight: 400;
  }
  .path-row {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .path-row input {
    flex: 1;
  }
  input,
  select,
  textarea {
    border-radius: 7px;
    border: 1px solid #ccc;
    padding: 0.45em 0.7em;
    font-size: 0.9rem;
    font-family: inherit;
    background: #fff;
    color: inherit;
  }
  textarea {
    font-family: ui-monospace, monospace;
    resize: vertical;
  }
  /* A required field still empty after URL autofill (or manual entry) — flagged so it's clear it's needed. */
  .missing {
    border-color: #e5484d;
    box-shadow: 0 0 0 1px rgba(229, 72, 77, 0.35);
  }
  .hint {
    font-size: 0.85rem;
    opacity: 0.6;
    margin: 0;
  }
  button {
    cursor: pointer;
    font-weight: 600;
    border-radius: 7px;
    border: 1px solid #396cd8;
    background: #396cd8;
    color: #fff;
    padding: 0.45em 0.8em;
    font-size: 0.9rem;
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
  button.sm {
    padding: 0.35em 0.6em;
    font-size: 0.8rem;
    white-space: nowrap;
  }
  .status {
    display: flex;
    justify-content: center;
    min-height: 1.7rem;
    margin-top: 0.2rem;
  }
  .testing {
    font-size: 0.82rem;
    opacity: 0.6;
  }
  .ok,
  .err {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    border-radius: 999px;
    padding: 0.25rem 0.85rem;
    font-size: 0.82rem;
    font-weight: 600;
    max-width: 100%;
  }
  .ok {
    color: #2ea043;
    background: rgba(46, 160, 67, 0.12);
    border: 1px solid rgba(46, 160, 67, 0.5);
  }
  .err {
    color: #e5484d;
    background: rgba(229, 72, 77, 0.1);
    border: 1px solid rgba(229, 72, 77, 0.5);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 0.3rem;
  }
  @media (prefers-color-scheme: dark) {
    .modal {
      background: #2a2a2a;
      color: #f0f0f0;
    }
    input,
    select,
    textarea {
      background: #1e1e1e;
      border-color: #444;
      color: #f0f0f0;
    }
  }
</style>
