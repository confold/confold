<script lang="ts">
  let {
    icon,
    alt = "",
    size = "normal",
    inverse = false,
  }: { icon: string; alt?: string; size?: "normal" | "large" | "compact"; inverse?: boolean } = $props();
</script>

<span class="ico" class:large={size === "large"} class:compact={size === "compact"} aria-hidden={alt ? undefined : "true"}>
  {#if icon === "nuxeo"}
    <img class:inverse src="/nuxeo-logo.svg" {alt} />
  {:else}
    {icon}
  {/if}
</span>

<style>
  .ico {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    line-height: 1;
    font-size: 1.1em;
  }

  .ico.large {
    font-size: 1.2em;
  }

  .ico.compact {
    font-size: 0.9em;
  }

  img {
    display: block;
    width: auto;
    max-width: 5.6em;
    height: 1.05em;
    object-fit: contain;
  }

  img.inverse {
    filter: brightness(0) invert(1);
  }

  .ico.compact img {
    max-width: 3.8em;
    height: 1em;
  }
</style>
