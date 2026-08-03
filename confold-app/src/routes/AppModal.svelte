<script lang="ts">
  import X from "@lucide/svelte/icons/x";
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";

  let {
    id,
    title,
    wide = false,
    dismissible = true,
    onDismiss,
    children,
  }: {
    id: string;
    title: string;
    wide?: boolean;
    dismissible?: boolean;
    onDismiss: () => void | Promise<void>;
    children: Snippet;
  } = $props();

  let dialog: HTMLDialogElement;
  let closeButton: HTMLButtonElement;

  onMount(() => {
    if (typeof dialog.showModal === "function") {
      dialog.removeAttribute("open");
      dialog.showModal();
    }
    if (dismissible) closeButton.focus();
    else dialog.focus();
  });

  function dismiss() {
    if (!dismissible) return;
    void onDismiss();
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    dismiss();
  }

  function onBackdropClick(event: MouseEvent) {
    if (event.target === event.currentTarget) dismiss();
  }
</script>

<dialog
  bind:this={dialog}
  open
  tabindex="-1"
  aria-labelledby={`${id}-title`}
  oncancel={(event) => {
    event.preventDefault();
    event.stopPropagation();
    dismiss();
  }}
  onkeydown={onKeydown}
  onclick={onBackdropClick}
>
  <section class="modal-panel" class:wide>
    <header>
      <h2 id={`${id}-title`}>{title}</h2>
      <button
        bind:this={closeButton}
        class="close-button"
        type="button"
        aria-label={`Close ${title}`}
        disabled={!dismissible}
        onclick={dismiss}
      >
        <X size={18} strokeWidth={2} aria-hidden="true" />
      </button>
    </header>
    <div class="modal-body">
      {@render children()}
    </div>
  </section>
</dialog>

<style>
  dialog {
    position: fixed;
    inset: 0;
    box-sizing: border-box;
    width: 100vw;
    max-width: none;
    height: 100dvh;
    max-height: none;
    margin: 0;
    padding: 1rem;
    border: 0;
    overflow: hidden;
    background: rgb(17 24 39 / 42%);
    color: inherit;
    place-items: center;
  }

  dialog[open] {
    display: grid;
  }

  dialog::backdrop {
    background: transparent;
  }

  .modal-panel {
    box-sizing: border-box;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
    width: min(35rem, 100%);
    max-height: calc(100dvh - 2rem);
    overflow: hidden;
    border: 1px solid #d7dce5;
    border-radius: 14px;
    background: #ffffff;
    box-shadow: 0 24px 80px rgb(17 24 39 / 24%);
  }

  .modal-panel.wide {
    width: min(60rem, 100%);
  }

  header {
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.15rem 1.25rem 1rem;
    border-bottom: 1px solid #e6e9ef;
    background: #ffffff;
  }

  h2 {
    margin: 0;
    font-size: 1.18rem;
    line-height: 1.25;
  }

  .close-button {
    display: inline-grid;
    flex: 0 0 auto;
    width: 2rem;
    height: 2rem;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: #5c6472;
    place-items: center;
  }

  .close-button:hover:not(:disabled) {
    border-color: #d7dce5;
    background: #f3f5f8;
    color: #1a1a1a;
  }

  .close-button:focus-visible {
    outline: 2px solid #396cd8;
    outline-offset: 2px;
  }

  .close-button:disabled {
    cursor: wait;
    opacity: 0.35;
  }

  .modal-body {
    min-height: 0;
    overflow: auto;
    padding: 1.25rem;
  }

  @media (prefers-color-scheme: dark) {
    dialog {
      background: rgb(0 0 0 / 58%);
    }

    .modal-panel,
    header {
      border-color: #3a3f49;
      background: #24272d;
    }

    header {
      border-bottom-color: #3a3f49;
    }

    .close-button {
      color: #aeb4c0;
    }

    .close-button:hover:not(:disabled) {
      border-color: #505762;
      background: #30343c;
      color: #f6f7f9;
    }
  }

  @media (prefers-reduced-motion: no-preference) {
    .modal-panel {
      animation: modal-in 120ms ease-out;
    }
  }

  @keyframes modal-in {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.99);
    }
  }
</style>
