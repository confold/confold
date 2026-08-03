<script lang="ts">
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import AppModal from "./AppModal.svelte";

  let {
    onDismiss,
    onOpenSettings,
  }: {
    onDismiss: () => void | Promise<void>;
    onOpenSettings: () => void | Promise<void>;
  } = $props();
</script>

<AppModal id="ai-provider-required" title="AI provider required" {onDismiss}>
  <div class="setup-message">
    <span class="icon" aria-hidden="true">
      <Sparkles size={22} strokeWidth={2} />
    </span>
    <div>
      <h3>Configure and verify an AI provider</h3>
      <p>
        Semantic review needs a tested model before Confold can compare the meaning of these
        documents.
      </p>
    </div>
  </div>
  <p class="privacy">The normal file comparison remains available and no files will be changed.</p>
  <div class="actions">
    <button type="button" class="secondary" onclick={() => void onDismiss()}>Not now</button>
    <button type="button" class="primary" onclick={() => void onOpenSettings()}>Open Settings</button>
  </div>
</AppModal>

<style>
  .setup-message {
    display: flex;
    align-items: flex-start;
    gap: 0.85rem;
  }

  .icon {
    display: inline-grid;
    flex: 0 0 auto;
    width: 2.45rem;
    height: 2.45rem;
    border-radius: 9px;
    background: #eaf0fc;
    color: #2d5aa5;
    place-items: center;
  }

  h3,
  p {
    margin: 0;
  }

  h3 {
    font-size: 0.96rem;
  }

  .setup-message p,
  .privacy {
    color: #687180;
    font-size: 0.82rem;
    line-height: 1.5;
  }

  .setup-message p {
    margin-top: 0.3rem;
  }

  .privacy {
    margin-top: 1rem;
    padding: 0.72rem 0.8rem;
    border-left: 3px solid #9db4e8;
    background: #f5f7fc;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.55rem;
    margin-top: 1rem;
  }

  button {
    min-height: 2.15rem;
    padding: 0.45rem 0.72rem;
    border-radius: 7px;
    font: 650 0.78rem/1.2 inherit;
  }

  button:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
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

  @media (prefers-color-scheme: dark) {
    .icon {
      background: #293b61;
      color: #b7d0ff;
    }

    .setup-message p,
    .privacy {
      color: #b7c0d2;
    }

    .privacy {
      background: #29303e;
    }

    .secondary {
      border-color: #505762;
      background: #30343c;
      color: #f0f2f5;
    }
  }
</style>
