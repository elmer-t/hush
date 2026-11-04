<script lang="ts">
  import type { ReadStatus } from "$lib/api";
  import { readMark } from "$lib/readStatus.svelte";

  // A list row's read marker; nothing for unread, which nearly every note is.
  let { status }: { status: ReadStatus } = $props();
  const mark = $derived(readMark(status));
</script>

{#if mark}<span class="read-mark" aria-label={mark.label} title={mark.label}>{mark.glyph}</span>{/if}

<style>
  .read-mark {
    flex: none;
    font-size: 12px;
    color: var(--muted);
  }
</style>
