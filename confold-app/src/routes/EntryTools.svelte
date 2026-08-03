<script lang="ts">
  import CircleHelp from "@lucide/svelte/icons/circle-help";
  import ExternalLink from "@lucide/svelte/icons/external-link";
  import Settings from "@lucide/svelte/icons/settings";
  import { tick } from "svelte";
  import AppModal from "./AppModal.svelte";
  import ProviderSettings from "./ProviderSettings.svelte";

  let {
    visible = true,
    appVersion,
    shellSupported,
    shellInstalled,
    shellBusy,
    shellMessage,
    onToggleShell,
    onregistersettings,
  }: {
    visible?: boolean;
    appVersion: string;
    shellSupported: boolean | null;
    shellInstalled: boolean;
    shellBusy: boolean;
    shellMessage: string;
    onToggleShell: () => void | Promise<void>;
    onregistersettings?: (open: ((trigger: HTMLButtonElement) => void) | null) => void;
  } = $props();

  type ModalName = "settings" | "help";
  let activeModal = $state<ModalName | null>(null);
  let settingsButton = $state<HTMLButtonElement>();
  let helpButton = $state<HTMLButtonElement>();
  let returnFocus: HTMLButtonElement | undefined;

  function openModal(modal: ModalName, trigger: HTMLButtonElement) {
    returnFocus = trigger;
    activeModal = modal;
  }

  function openSettings(trigger: HTMLButtonElement) {
    openModal("settings", trigger);
  }

  async function closeModal() {
    activeModal = null;
    await tick();
    returnFocus?.focus();
  }

  $effect(() => {
    onregistersettings?.(openSettings);
    return () => onregistersettings?.(null);
  });
</script>

{#if visible}
  <div class="entry-tools" aria-label="Application tools">
    <button
      bind:this={settingsButton}
      class="tool-button settings-button"
      type="button"
      aria-label="Open settings"
      aria-haspopup="dialog"
      onclick={() => {
        if (settingsButton) openSettings(settingsButton);
      }}
    >
      <Settings size={18} strokeWidth={2} aria-hidden="true" />
    </button>
    <button
      bind:this={helpButton}
      class="tool-button help-button"
      type="button"
      aria-label="Open help and about"
      aria-haspopup="dialog"
      onclick={() => {
        if (helpButton) openModal("help", helpButton);
      }}
    >
      <CircleHelp size={18} strokeWidth={2} aria-hidden="true" />
    </button>
  </div>
{/if}

{#if activeModal === "settings"}
  <AppModal id="settings" title="Settings" onDismiss={closeModal}>
    <div class="settings-content">
      <section class="setting-section">
        <div class="section-heading">
          <h3>Folder integration</h3>
          <p>Open Confold directly from your file manager's folder menu.</p>
        </div>
        {#if shellSupported === null}
          <p class="status-note">Checking availability...</p>
        {:else if shellSupported}
          <label class="integration-toggle">
            <span>
              <strong>Folder right-click integration</strong>
              <small>{shellInstalled ? "Installed on this device" : "Not installed"}</small>
            </span>
            <input
              type="checkbox"
              checked={shellInstalled}
              disabled={shellBusy}
              onchange={() => void onToggleShell()}
            />
          </label>
          {#if shellMessage}<p class="success-note" role="status">{shellMessage}</p>{/if}
        {:else}
          <div class="unavailable">
            <strong>Not available on this platform</strong>
            <p>Confold cannot install a folder-menu integration on this operating system.</p>
          </div>
        {/if}
      </section>

      <section class="setting-section">
        <ProviderSettings />
      </section>
    </div>
  </AppModal>
{:else if activeModal === "help"}
  <AppModal id="help-about" title="Help and About" onDismiss={closeModal}>
    <div class="help-content">
      <div class="about-heading">
        <img src="/confold-mark.png" alt="" />
        <div>
          <h3>Confold</h3>
          {#if appVersion}<span>{appVersion}</span>{/if}
        </div>
      </div>
      <p>
        Compare, migrate, and synchronize folders with explicit previews and controlled file operations.
      </p>

      <section class="quick-start">
        <h3>Quick start</h3>
        <ol>
          <li>Select a source and a destination.</li>
          <li>Choose Compare, Migrate, or Sync.</li>
          <li>Review the results before applying any file operations.</li>
        </ol>
      </section>

      <nav class="resource-links" aria-label="Confold resources">
        <a href="https://confold.com" target="_blank" rel="noreferrer">
          <span><strong>Project website</strong><small>Guides and downloads</small></span>
          <ExternalLink size={16} strokeWidth={2} aria-hidden="true" />
        </a>
        <a href="https://github.com/confold/confold" target="_blank" rel="noreferrer">
          <span><strong>Source code</strong><small>Issues and releases</small></span>
          <ExternalLink size={16} strokeWidth={2} aria-hidden="true" />
        </a>
        <a href="https://ko-fi.com/juanyque" target="_blank" rel="noreferrer">
          <span><strong>Support on Ko-fi</strong><small>Help sustain development</small></span>
          <ExternalLink size={16} strokeWidth={2} aria-hidden="true" />
        </a>
        <a href="https://github.com/sponsors/juanyque" target="_blank" rel="noreferrer">
          <span><strong>GitHub Sponsors</strong><small>Recurring project support</small></span>
          <ExternalLink size={16} strokeWidth={2} aria-hidden="true" />
        </a>
      </nav>
    </div>
  </AppModal>
{/if}

<style>
  .entry-tools {
    position: fixed;
    inset: 0;
    z-index: 20;
    pointer-events: none;
  }

  .settings-button {
    position: absolute;
    left: 0.75rem;
    bottom: 0.65rem;
  }

  .help-button {
    position: absolute;
    right: 0.75rem;
    bottom: 0.65rem;
  }

  .tool-button {
    display: inline-grid;
    width: 2.25rem;
    height: 2.25rem;
    padding: 0;
    border: 1px solid #d7dce5;
    border-radius: 9px;
    background: rgb(255 255 255 / 76%);
    box-shadow: 0 2px 8px rgb(17 24 39 / 8%);
    color: #5c6472;
    opacity: 0.72;
    place-items: center;
    pointer-events: auto;
    backdrop-filter: blur(8px);
  }

  .tool-button:hover {
    border-color: #b9c0cc;
    background: #ffffff;
    color: #1a1a1a;
    opacity: 1;
  }

  .tool-button:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
    opacity: 1;
  }

  .settings-content,
  .help-content {
    display: grid;
    gap: 1.25rem;
  }

  .setting-section {
    display: grid;
    gap: 0.9rem;
  }

  .setting-section + .setting-section {
    padding-top: 1.25rem;
    border-top: 1px solid #e6e9ef;
  }

  .section-heading,
  .section-heading p,
  .unavailable p,
  .help-content > p {
    margin: 0;
  }

  .section-heading {
    display: grid;
    gap: 0.28rem;
  }

  .section-heading h3,
  .help-content h3 {
    margin: 0;
    font-size: 0.96rem;
  }

  .section-heading p,
  .help-content > p,
  .unavailable p,
  .status-note {
    color: #687180;
    font-size: 0.88rem;
    line-height: 1.5;
  }

  .integration-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.9rem 1rem;
    border: 1px solid #dfe3ea;
    border-radius: 10px;
    background: #f8f9fb;
    cursor: pointer;
  }

  .integration-toggle > span {
    display: grid;
    gap: 0.18rem;
  }

  .integration-toggle strong,
  .resource-links strong {
    font-size: 0.86rem;
  }

  .integration-toggle small,
  .resource-links small {
    color: #737b89;
    font-size: 0.76rem;
  }

  .integration-toggle input {
    width: 1.05rem;
    height: 1.05rem;
    margin: 0;
    accent-color: #396cd8;
  }

  .unavailable {
    display: grid;
    gap: 0.3rem;
    padding: 0.9rem 1rem;
    border: 1px solid #dfe3ea;
    border-radius: 10px;
    background: #f8f9fb;
  }

  .unavailable strong {
    font-size: 0.86rem;
  }

  .status-note,
  .success-note {
    margin: 0;
  }

  .success-note {
    color: #207a3d;
    font-size: 0.8rem;
  }

  .about-heading {
    display: flex;
    align-items: center;
    gap: 0.8rem;
  }

  .about-heading img {
    width: 2.75rem;
    height: 2.75rem;
    object-fit: contain;
  }

  .about-heading div {
    display: grid;
    gap: 0.14rem;
  }

  .about-heading h3 {
    font-size: 1.15rem;
  }

  .about-heading span {
    color: #737b89;
    font-size: 0.78rem;
  }

  .quick-start {
    display: grid;
    gap: 0.65rem;
    padding-top: 1rem;
    border-top: 1px solid #e6e9ef;
  }

  .quick-start ol {
    display: grid;
    gap: 0.45rem;
    margin: 0;
    padding-left: 1.3rem;
    color: #4f5867;
    font-size: 0.86rem;
    line-height: 1.45;
  }

  .resource-links {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.6rem;
  }

  .resource-links a {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    min-width: 0;
    padding: 0.78rem 0.85rem;
    border: 1px solid #dfe3ea;
    border-radius: 9px;
    color: inherit;
    text-decoration: none;
  }

  .resource-links a:hover {
    border-color: #b9c7e4;
    background: #f7f9fd;
  }

  .resource-links a:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
  }

  .resource-links a > span {
    display: grid;
    min-width: 0;
    gap: 0.2rem;
  }

  @media (max-width: 34rem) {
    .resource-links {
      grid-template-columns: 1fr;
    }
  }

  @media (prefers-color-scheme: dark) {
    .tool-button {
      border-color: #444a55;
      background: rgb(36 39 45 / 78%);
      color: #aeb4c0;
    }

    .tool-button:hover {
      border-color: #626976;
      background: #30343c;
      color: #f6f7f9;
    }

    .setting-section + .setting-section,
    .quick-start {
      border-color: #3a3f49;
    }

    .section-heading p,
    .help-content > p,
    .unavailable p,
    .status-note,
    .integration-toggle small,
    .resource-links small,
    .about-heading span {
      color: #aeb4c0;
    }

    .integration-toggle,
    .unavailable {
      border-color: #3f4550;
      background: #2b2f36;
    }

    .quick-start ol {
      color: #c7cbd3;
    }

    .resource-links a {
      border-color: #3f4550;
    }

    .resource-links a:hover {
      border-color: #566b96;
      background: #292f3d;
    }
  }
</style>
