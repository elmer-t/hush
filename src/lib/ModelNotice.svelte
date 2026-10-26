<script lang="ts">
  // One line at the point of use saying why a feature is off, linking to Settings → Model.
  import { model, reducedText } from "$lib/modelStatus.svelte";

  let { what }: { /** The feature, e.g. "Related notes". */ what: string } = $props();

  const reason = $derived(reducedText());
</script>

{#if reason}
  <p class="muted notice" role="status">
    {what} need a ready model ({reason}). See
    <button class="link" onclick={() => model.open?.()}>Settings → Model</button>.
  </p>
{/if}

<style>
  .notice {
    margin: 0;
    font-size: 12px;
  }

  .link {
    font-size: inherit;
  }
</style>
