<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$lib/commands";
  import BusyNotice from "./BusyNotice.svelte";
  import type {
    ProviderKind,
    ProviderProfile,
    ProviderSettings,
    SaveProviderSettingsRequest,
    SemanticLanguage,
  } from "$lib/commands";

  const OLLAMA_BASE_URL = "http://127.0.0.1:11434/v1";
  const CUSTOM_BASE_URL = "http://127.0.0.1:8080/v1";

  let profile = $state<ProviderProfile>({
    kind: "ollama",
    baseUrl: OLLAMA_BASE_URL,
    model: "",
    useApiKey: false,
  });
  let models = $state<string[]>([]);
  let manualModel = $state(false);
  let apiKey = $state("");
  let customHeaders = $state("");
  let hasApiKey = $state(false);
  let hasCustomHeaders = $state(false);
  let clearApiKey = $state(false);
  let clearCustomHeaders = $state(false);
  let busy = $state<"loading" | "saving" | "discovering" | "testing" | null>("loading");
  let message = $state("");
  let error = $state("");
  let language = $state<SemanticLanguage>("en");
  let languageSaving = $state(false);

  onMount(async () => {
    try {
      const [settings, preferences] = await Promise.all([
        commands.loadProviderSettings(),
        commands.loadSemanticPreferences(),
      ]);
      applySettings(settings);
      language = preferences.language;
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = null;
    }
  });

  function applySettings(settings: ProviderSettings) {
    profile = { ...settings.profile };
    hasApiKey = settings.hasApiKey;
    hasCustomHeaders = settings.hasCustomHeaders;
  }

  function changeProvider(event: Event) {
    const next = (event.currentTarget as HTMLSelectElement).value as ProviderKind;
    if (next === profile.kind) return;
    if (next === "ollama" && profile.baseUrl === CUSTOM_BASE_URL) {
      profile.baseUrl = OLLAMA_BASE_URL;
    } else if (next === "open_ai_compatible" && profile.baseUrl === OLLAMA_BASE_URL) {
      profile.baseUrl = CUSTOM_BASE_URL;
    }
    profile.kind = next;
    models = [];
    clearFeedback();
  }

  function clearFeedback() {
    message = "";
    error = "";
  }

  async function changeLanguage(event: Event & { currentTarget: HTMLSelectElement }) {
    const value = event.currentTarget.value;
    if (value !== "en" && value !== "es") return;
    language = value;
    languageSaving = true;
    message = "";
    error = "";
    try {
      const saved = await commands.saveSemanticPreferences({ language });
      language = saved.language;
      message = "Communication language saved.";
    } catch (cause) {
      error = String(cause);
    } finally {
      languageSaving = false;
    }
  }

  function chooseModel(event: Event) {
    const selected = (event.currentTarget as HTMLSelectElement).value;
    if (selected === "__manual__") {
      profile.model = "";
      manualModel = true;
    } else {
      profile.model = selected;
    }
    clearFeedback();
  }

  function parseCustomHeaders(value: string): Record<string, string> | undefined {
    if (!value.trim()) return undefined;
    const parsed: Record<string, string> = {};
    for (const line of value.split("\n")) {
      if (!line.trim()) continue;
      const separator = line.indexOf(":");
      if (separator <= 0) {
        throw new Error("Custom headers must use one “Name: value” entry per line.");
      }
      const name = line.slice(0, separator).trim();
      const headerValue = line.slice(separator + 1).trim();
      if (!name || !headerValue) {
        throw new Error("Custom header names and values cannot be empty.");
      }
      parsed[name] = headerValue;
    }
    return parsed;
  }

  function request(): SaveProviderSettingsRequest {
    return {
      profile: { ...profile },
      apiKey: apiKey.trim() || undefined,
      clearApiKey,
      customHeaders: parseCustomHeaders(customHeaders),
      clearCustomHeaders,
    };
  }

  async function persist() {
    const settings = await commands.saveProviderSettings(request());
    applySettings(settings);
    apiKey = "";
    customHeaders = "";
    clearApiKey = false;
    clearCustomHeaders = false;
  }

  async function save() {
    busy = "saving";
    message = "";
    error = "";
    try {
      await persist();
      message = "Provider settings saved.";
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = null;
    }
  }

  async function discover() {
    busy = "discovering";
    message = "";
    error = "";
    try {
      await persist();
      models = await commands.listProviderModels();
      if (!profile.model && models.length > 0) profile.model = models[0];
      manualModel = Boolean(profile.model) && !models.includes(profile.model);
      message =
        models.length === 0
          ? "The endpoint returned no models. You can still enter a model identifier."
          : `Found ${models.length} model${models.length === 1 ? "" : "s"}.`;
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = null;
    }
  }

  async function saveAndTest() {
    busy = "testing";
    message = "";
    error = "";
    try {
      await persist();
      const report = await commands.testProviderCapability();
      message = `Structured output verified with ${report.model}.`;
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = null;
    }
  }
</script>

<section class="provider-settings">
  <div class="section-heading">
    <h3>AI providers</h3>
    <p>Connect a model for semantic reconciliation. Confold keeps control of review and apply.</p>
  </div>

  {#if busy === "testing"}
    <BusyNotice
      title={`Testing ${profile.model || "the configured model"}`}
      detail="Confold is checking structured output support. Local models can take a minute to respond."
    />
  {/if}

  {#if busy === "loading"}
    <p class="muted" role="status">Loading provider settings...</p>
  {:else}
    <div class="fields">
      <label>
        <span>Provider profile</span>
        <select value={profile.kind} onchange={changeProvider} disabled={busy !== null}>
          <option value="ollama">Ollama</option>
          <option value="open_ai_compatible">Custom OpenAI-compatible</option>
        </select>
      </label>

      <label>
        <span>Base URL</span>
        <input
          type="url"
          bind:value={profile.baseUrl}
          oninput={clearFeedback}
          spellcheck="false"
          autocomplete="url"
          disabled={busy !== null}
        />
      </label>

      <div class="model-field">
        <label>
          <span>Model</span>
          {#if models.length > 0 && !manualModel}
            <select value={profile.model} onchange={chooseModel} disabled={busy !== null}>
              {#each models as model}
                <option value={model}>{model}</option>
              {/each}
              <option value="__manual__">Enter another identifier…</option>
            </select>
          {:else}
            <input
              bind:value={profile.model}
              oninput={clearFeedback}
              placeholder="Enter a model identifier"
              spellcheck="false"
              autocomplete="off"
              disabled={busy !== null}
            />
          {/if}
        </label>
        <button type="button" class="secondary" disabled={busy !== null} onclick={() => void discover()}>
          {busy === "discovering" ? "Discovering..." : "Discover models"}
        </button>
      </div>

      <label class="communication-field">
        <span>AI communication language</span>
        <select value={language} onchange={(event) => void changeLanguage(event)} disabled={busy !== null || languageSaving}>
          <option value="en">English</option>
          <option value="es">Español</option>
        </select>
        <small>Summaries and warnings use this language. Confold keeps the merged document in its original language.</small>
      </label>

      <label class="check-row">
        <input
          type="checkbox"
          bind:checked={profile.useApiKey}
          disabled={busy !== null}
          onchange={clearFeedback}
        />
        <span>Endpoint requires an API key</span>
      </label>

      {#if profile.useApiKey}
        <div class="secret-field">
          <label>
            <span>API key</span>
            <input
              type="password"
              bind:value={apiKey}
              oninput={clearFeedback}
              placeholder={hasApiKey ? "Stored in the operating-system keychain" : "Enter API key"}
              autocomplete="new-password"
              disabled={busy !== null || clearApiKey}
            />
          </label>
          {#if hasApiKey}
            <label class="clear-row">
              <input
                type="checkbox"
                bind:checked={clearApiKey}
                disabled={busy !== null}
                onchange={clearFeedback}
              />
              <span>Remove stored API key</span>
            </label>
          {/if}
        </div>
      {/if}

      <details>
        <summary>Advanced headers</summary>
        <div class="advanced-fields">
          <label>
            <span>Custom headers</span>
            <textarea
              bind:value={customHeaders}
              oninput={clearFeedback}
              rows="3"
              placeholder={hasCustomHeaders
                ? "Stored securely. Enter replacements as Name: value"
                : "One Name: value header per line"}
              spellcheck="false"
              disabled={busy !== null || clearCustomHeaders}
            ></textarea>
          </label>
          <small>Header values are stored with the API key, never in the provider settings file.</small>
          {#if hasCustomHeaders}
            <label class="clear-row">
              <input
                type="checkbox"
                bind:checked={clearCustomHeaders}
                disabled={busy !== null}
                onchange={clearFeedback}
              />
              <span>Remove stored custom headers</span>
            </label>
          {/if}
        </div>
      </details>
    </div>

    <p class="privacy-note">
      Content sent for reconciliation goes to this endpoint. A local Ollama URL stays on this device;
      remote endpoints follow their operator's privacy policy.
    </p>

    {#if message}<p class="success" role="status">{message}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="actions">
      <button type="button" class="secondary" disabled={busy !== null} onclick={() => void save()}>
        {busy === "saving" ? "Saving..." : "Save"}
      </button>
      <button type="button" class="primary" disabled={busy !== null} onclick={() => void saveAndTest()}>
        {busy === "testing" ? "Testing..." : "Save and test"}
      </button>
    </div>
  {/if}
</section>

<style>
  .provider-settings,
  .fields,
  .advanced-fields,
  .secret-field {
    display: grid;
  }

  .provider-settings {
    gap: 0.9rem;
  }

  .section-heading,
  .section-heading p,
  .privacy-note,
  .muted,
  .success,
  .error {
    margin: 0;
  }

  .section-heading {
    display: grid;
    gap: 0.28rem;
  }

  .section-heading h3 {
    margin: 0;
    font-size: 0.96rem;
  }

  .section-heading p,
  .muted,
  .privacy-note {
    color: #687180;
    font-size: 0.82rem;
    line-height: 1.5;
  }

  .fields {
    gap: 0.78rem;
  }

  label:not(.check-row, .clear-row) {
    display: grid;
    gap: 0.34rem;
  }

  label > span,
  summary {
    color: #3f4754;
    font-size: 0.78rem;
    font-weight: 650;
  }

  input,
  select,
  textarea,
  button {
    font: inherit;
  }

  input:not([type="checkbox"]),
  select,
  textarea {
    box-sizing: border-box;
    width: 100%;
    padding: 0.56rem 0.65rem;
    border: 1px solid #cfd5df;
    border-radius: 7px;
    background: #fff;
    color: #242933;
    font-size: 0.82rem;
  }

  textarea {
    resize: vertical;
  }

  input:focus-visible,
  select:focus-visible,
  textarea:focus-visible,
  button:focus-visible,
  summary:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
  }

  .model-field {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: end;
    gap: 0.55rem;
  }

  .check-row,
  .clear-row {
    display: flex;
    align-items: center;
    gap: 0.48rem;
    width: fit-content;
    color: #4f5867;
    font-size: 0.78rem;
  }

  .check-row input,
  .clear-row input {
    width: 1rem;
    height: 1rem;
    margin: 0;
    accent-color: #396cd8;
  }

  .secret-field,
  .advanced-fields {
    gap: 0.5rem;
  }

  details {
    padding: 0.65rem 0.72rem;
    border: 1px solid #dfe3ea;
    border-radius: 8px;
    background: #fafbfc;
  }

  summary {
    cursor: pointer;
  }

  .advanced-fields {
    padding-top: 0.7rem;
  }

  .advanced-fields small {
    color: #737b89;
    font-size: 0.72rem;
    line-height: 1.4;
  }

  .communication-field small {
    color: #737b89;
    font-size: 0.72rem;
    font-weight: 400;
    line-height: 1.4;
  }

  .privacy-note {
    padding: 0.72rem 0.8rem;
    border-left: 3px solid #9db4e8;
    background: #f5f7fc;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.55rem;
  }

  button {
    min-height: 2.15rem;
    padding: 0.45rem 0.72rem;
    border-radius: 7px;
    font-size: 0.78rem;
    font-weight: 650;
  }

  button:disabled {
    cursor: wait;
    opacity: 0.58;
  }

  .secondary {
    border: 1px solid #cfd5df;
    background: #fff;
    color: #3f4754;
  }

  .primary {
    border: 1px solid #315fb9;
    background: #396cd8;
    color: #fff;
  }

  .success,
  .error {
    font-size: 0.78rem;
    line-height: 1.4;
  }

  .success {
    color: #207a3d;
  }

  .error {
    color: #b42318;
  }

  @media (max-width: 30rem) {
    .model-field {
      grid-template-columns: 1fr;
    }

    .model-field button {
      justify-self: start;
    }
  }

  @media (prefers-color-scheme: dark) {
    .section-heading p,
    .muted,
    .privacy-note,
    .advanced-fields small {
      color: #aeb4c0;
    }

    label > span,
    summary,
    .check-row,
    .clear-row {
      color: #d5d8df;
    }

    input:not([type="checkbox"]),
    select,
    textarea,
    .secondary {
      border-color: #505762;
      background: #30343c;
      color: #f0f2f5;
    }

    details {
      border-color: #444a55;
      background: #2a2e35;
    }

    .privacy-note {
      background: #29303e;
    }

    .success {
      color: #75d99a;
    }

    .error {
      color: #ff928a;
    }
  }
</style>
