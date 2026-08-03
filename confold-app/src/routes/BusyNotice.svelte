<script lang="ts">
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";

  let {
    title,
    detail,
  }: {
    title: string;
    detail: string;
  } = $props();
</script>

<div class="busy-notice" role="status" aria-live="polite" aria-busy="true">
  <span class="spinner" aria-hidden="true">
    <LoaderCircle size={24} strokeWidth={2} />
  </span>
  <span>
    <strong>{title}</strong>
    <small>{detail}</small>
  </span>
</div>

<style>
  .busy-notice {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.85rem 0.95rem;
    border: 1px solid #cfdaf3;
    border-radius: 9px;
    background: #f4f7fd;
    color: #33415c;
  }

  .busy-notice > span:last-child {
    display: grid;
    gap: 0.18rem;
    min-width: 0;
  }

  strong {
    font-size: 0.86rem;
    overflow-wrap: anywhere;
  }

  small {
    color: #667085;
    font-size: 0.76rem;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .spinner {
    display: inline-grid;
    flex: 0 0 auto;
    color: #396cd8;
    place-items: center;
    animation: spin 900ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }

  @media (prefers-color-scheme: dark) {
    .busy-notice {
      border-color: #3c4b68;
      background: #282f3c;
      color: #edf2ff;
    }

    small {
      color: #b7c0d2;
    }

    .spinner {
      color: #8eb2f2;
    }
  }
</style>
