<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { commands } from "$lib/commands";
  import { parseUnifiedDiff } from "$lib/unified-diff";
  import type { SemanticReviewOutcome, SemanticTarget } from "$lib/commands";
  import type { FileRef } from "$lib/types";
  import AppModal from "./AppModal.svelte";
  import BusyNotice from "./BusyNotice.svelte";
  import "./SemanticReviewModal.css";

  let {
    outcome,
    error,
    busy,
    model,
    left,
    right,
    onsaved,
    onDismiss,
  }: {
    outcome: SemanticReviewOutcome | null;
    error: string;
    busy: boolean;
    model: string | null;
    left: FileRef;
    right: FileRef;
    onsaved?: (side: "left" | "right") => void | Promise<void>;
    onDismiss: () => void | Promise<void>;
  } = $props();

  const labels = {
    byte_identical: "The files are byte-for-byte identical.",
    formatting_only: "The content is equivalent; only formatting differs.",
    prefer_left: "The deterministic checks recommend the left document.",
    prefer_right: "The deterministic checks recommend the right document.",
  } as const;

  let confirmTarget = $state<SemanticTarget | null>(null);
  let saving = $state(false);
  let saveError = $state("");
  let savedPath = $state("");

  const review = $derived(outcome?.kind === "review" ? outcome : null);
  const targetPath = $derived(
    confirmTarget === "origin" ? review?.origin_path : review?.destination_path,
  );
  const originUnchanged = $derived(review?.proposal.verdict === "prefer_left");
  const destinationUnchanged = $derived(review?.proposal.verdict === "prefer_right");

  function words(value: string) {
    return value.replaceAll("_", " ");
  }

  function providerName(provider: "ollama" | "open_ai_compatible") {
    return provider === "ollama" ? "Ollama" : "OpenAI-compatible";
  }

  function languageName(language: "en" | "es") {
    return language === "es" ? "Español" : "English";
  }

  function defaultMergedPath(origin: string) {
    const separator = Math.max(origin.lastIndexOf("/"), origin.lastIndexOf("\\"));
    const directory = origin.slice(0, separator + 1);
    const filename = origin.slice(separator + 1);
    const dot = filename.lastIndexOf(".");
    if (dot <= 0) return `${directory}${filename}-merged`;
    return `${directory}${filename.slice(0, dot)}-merged${filename.slice(dot)}`;
  }

  async function applyTarget() {
    if (!review || !confirmTarget || saving) return;
    const target = confirmTarget;
    saving = true;
    saveError = "";
    try {
      const report = await commands.semanticApplyTo(left, right, review.proposal, target);
      savedPath = report.output;
      confirmTarget = null;
      await onsaved?.(target === "origin" ? "left" : "right");
    } catch (cause) {
      saveError = String(cause);
    } finally {
      saving = false;
    }
  }

  async function saveAs() {
    if (!review || saving) return;
    saving = true;
    saveError = "";
    try {
      const output = await save({
        title: "Save semantic result as",
        defaultPath: defaultMergedPath(review.origin_path),
      });
      if (!output) return;
      const report = await commands.semanticApplyNew(left, right, review.proposal, output);
      savedPath = report.output;
    } catch (cause) {
      saveError = String(cause);
    } finally {
      saving = false;
    }
  }
</script>

<AppModal id="semantic-review" title="Semantic review" wide dismissible={!busy && !saving} {onDismiss}>
  <div class="review-content">
    <p class="safety-note">
      Review first: the selected local documents were sent to the configured provider, but no files
      have changed yet.
    </p>

    {#if busy}
      <BusyNotice
        title={`Reviewing with ${model ?? "the configured model"}`}
        detail="The model is analysing both documents. This can take a few minutes; Confold is still responsive."
      />
    {:else if error}
      <div class="error-card" role="alert">
        <strong>Review could not be completed</strong>
        <p>{error}</p>
      </div>
    {:else if outcome?.kind === "fast_path"}
      <section class="fast-path">
        <span>Deterministic result</span>
        <h3>{labels[outcome.fast_path]}</h3>
        <p>No model call was needed.</p>
      </section>
    {:else if review}
      <section class="configuration-card" aria-label="AI configuration used">
        <h4>AI configuration used</h4>
        <dl>
          <div><dt>Provider</dt><dd>{providerName(review.configuration.provider)}</dd></div>
          <div><dt>Requested model</dt><dd>{review.configuration.requestedModel}</dd></div>
          <div><dt>Used model</dt><dd>{review.configuration.responseModel}</dd></div>
          <div><dt>Language</dt><dd>{languageName(review.configuration.language)}</dd></div>
          <div class="endpoint"><dt>Endpoint</dt><dd><code>{review.configuration.endpoint}</code></dd></div>
        </dl>
      </section>

      <div class="review-heading">
        <div>
          <span class="eyebrow">Validated proposal</span>
          <h3>{review.report.summary}</h3>
        </div>
        <div class="badges">
          <span class="verdict">{words(review.report.verdict)}</span>
          <span class="model">{review.model}</span>
        </div>
      </div>

      {#if review.report.warnings.length}
        <section class="warnings" aria-label="Review warnings">
          <strong>Warnings</strong>
          <ul>
            {#each review.report.warnings as warning}<li>{warning}</li>{/each}
          </ul>
        </section>
      {/if}

      <section>
        <h4>Contributions</h4>
        <div class="contributions">
          {#each review.report.contributions as contribution}
            <article>
              <div>
                <strong>{contribution.source}</strong>
                <span>{words(contribution.disposition)}</span>
              </div>
              <p>{contribution.intent}</p>
            </article>
          {/each}
        </div>
      </section>

      {#if review.proposal.result}
        <section>
          <h4>Proposed result</h4>
          <pre>{review.proposal.result}</pre>
        </section>
      {/if}

      <section>
        <h4>Deterministic diffs</h4>
        {#each review.report.diffs as diff}
          <details>
            <summary>{diff.source} compared with the proposal</summary>
            <pre class="unified-diff" aria-label={`${diff.source} deterministic diff`}>{#each parseUnifiedDiff(diff.unified_diff) as line}<span class={`diff-line ${line.kind}`}>{line.text}</span>{/each}</pre>
          </details>
        {/each}
      </section>

      {#if saving}
        <BusyNotice
          title="Saving semantic result"
          detail="Confold is revalidating both inputs and writing atomically. The originals remain safe if this operation fails."
        />
      {:else if saveError}
        <div class="error-card" role="alert">
          <strong>Result could not be saved</strong>
          <p>{saveError}</p>
        </div>
      {:else if savedPath}
        <div class="success-card" role="status">
          <strong>Semantic result saved</strong>
          <p>Saved to {savedPath}</p>
        </div>
      {/if}

      {#if confirmTarget && targetPath && !savedPath}
        <section class="confirm-card" aria-label={`Confirm ${confirmTarget} replacement`}>
          <strong>Replace {confirmTarget}?</strong>
          <p>Confold will replace this file after revalidating both inputs:</p>
          <code>{targetPath}</code>
          <p>The other file will remain unchanged.</p>
          <div class="confirm-actions">
            <button type="button" class="secondary" disabled={saving} onclick={() => (confirmTarget = null)}>Cancel</button>
            <button type="button" class="danger" disabled={saving} onclick={() => void applyTarget()}>Replace {confirmTarget}</button>
          </div>
        </section>
      {/if}

      {#if review.report.applicable}
        <footer class="review-actions">
          {#if savedPath}
            <button type="button" class="primary" onclick={() => void onDismiss()}>Close</button>
          {:else}
            <button type="button" class="secondary" disabled={saving} onclick={() => void saveAs()}>Save as…</button>
            <span class="action-spacer"></span>
            <button type="button" class="secondary" disabled={saving || originUnchanged} title={originUnchanged ? "The proposal already matches the origin" : undefined} onclick={() => (confirmTarget = "origin")}>Save to origin</button>
            <button type="button" class="primary" disabled={saving || destinationUnchanged} title={destinationUnchanged ? "The proposal already matches the destination" : undefined} onclick={() => (confirmTarget = "destination")}>Save to destination</button>
          {/if}
        </footer>
      {/if}
    {/if}
  </div>
</AppModal>
