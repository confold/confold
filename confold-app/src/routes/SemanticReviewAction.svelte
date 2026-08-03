<script lang="ts">
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import { onMount, tick } from "svelte";
  import { commands } from "$lib/commands";
  import { isSemanticPath } from "$lib/semantic";
  import type { SemanticReviewOutcome } from "$lib/commands";
  import type { FileRef } from "$lib/types";
  import ProviderRequiredModal from "./ProviderRequiredModal.svelte";
  import SemanticReviewModal from "./SemanticReviewModal.svelte";

  let {
    left,
    right,
    disabled = false,
    autostart = false,
    onconfigure,
    onsaved,
  }: {
    left: FileRef;
    right: FileRef;
    disabled?: boolean;
    autostart?: boolean;
    onconfigure?: (trigger: HTMLButtonElement) => void;
    onsaved?: (side: "left" | "right") => void | Promise<void>;
  } = $props();

  let trigger = $state<HTMLButtonElement>();
  let verifiedModel = $state<string | null>(null);
  let loadingSettings = $state(true);
  let busy = $state(false);
  let outcome = $state<SemanticReviewOutcome | null>(null);
  let error = $state("");
  let modalOpen = $state(false);
  let setupOpen = $state(false);

  const eligible = $derived(
    left.source.kind === "fs" &&
      right.source.kind === "fs" &&
      isSemanticPath(left.rel) &&
      isSemanticPath(right.rel),
  );
  const unavailable = $derived(
    disabled || loadingSettings || busy || (!verifiedModel && !onconfigure),
  );
  const title = $derived(
    disabled
      ? "Save or discard edits before semantic review"
      : loadingSettings
        ? "Checking the configured provider"
        : !verifiedModel
          ? "Configure an AI provider for semantic review"
          : `Review with ${verifiedModel}`,
  );

  async function loadSettings() {
    loadingSettings = true;
    try {
      verifiedModel = (await commands.loadProviderSettings()).verifiedModel;
    } catch {
      verifiedModel = null;
    } finally {
      loadingSettings = false;
    }
  }

  async function review() {
    if (unavailable || !verifiedModel) return;
    busy = true;
    error = "";
    outcome = null;
    modalOpen = true;
    try {
      outcome = await commands.semanticReview(left, right);
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
      modalOpen = true;
    }
  }

  async function activate(refreshSettings = true) {
    if (disabled || loadingSettings || busy) return;
    if (refreshSettings) await loadSettings();
    if (!verifiedModel) {
      setupOpen = true;
      return;
    }
    await review();
  }

  async function closeModal() {
    modalOpen = false;
    await tick();
    trigger?.focus();
  }

  async function closeSetup() {
    setupOpen = false;
    await tick();
    trigger?.focus();
  }

  async function openSettings() {
    setupOpen = false;
    await tick();
    if (trigger) onconfigure?.(trigger);
  }

  onMount(async () => {
    await loadSettings();
    await tick();
    if (autostart) await activate(false);
  });
</script>

{#if eligible}
  <button
    bind:this={trigger}
    class="semantic-action"
    type="button"
    disabled={unavailable}
    {title}
    aria-label={verifiedModel ? "Run semantic review" : "Set up AI review"}
    aria-haspopup="dialog"
    onclick={() => void activate()}
    onfocus={() => {
      if (!verifiedModel && !loadingSettings) void loadSettings();
    }}
  >
    <Sparkles size={15} strokeWidth={2} aria-hidden="true" />
    <span>{busy ? "Reviewing..." : verifiedModel ? "Semantic review" : "Set up AI review"}</span>
  </button>
{/if}

{#if modalOpen}
  <SemanticReviewModal
    {outcome}
    {error}
    {busy}
    {left}
    {right}
    {onsaved}
    model={verifiedModel}
    onDismiss={closeModal}
  />
{/if}

{#if setupOpen}
  <ProviderRequiredModal onDismiss={closeSetup} onOpenSettings={openSettings} />
{/if}

<style>
  .semantic-action {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3em 0.65em;
    border: 1px solid #8da9df;
    border-radius: 7px;
    background: #f4f7fd;
    color: #2d5aa5;
    font: 600 0.75rem/1.2 inherit;
    white-space: nowrap;
    cursor: pointer;
  }
  .semantic-action:hover:not(:disabled) {
    border-color: #396cd8;
    background: #eaf0fc;
  }
  .semantic-action:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
  }
  .semantic-action:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }
  @media (max-width: 44rem) {
    .semantic-action span {
      display: none;
    }
    .semantic-action {
      padding: 0.32em 0.48em;
    }
  }
  @media (prefers-color-scheme: dark) {
    .semantic-action {
      border-color: #48658f;
      background: #293344;
      color: #b9d0f6;
    }
    .semantic-action:hover:not(:disabled) {
      border-color: #6d94cf;
      background: #303d51;
    }
  }
</style>
