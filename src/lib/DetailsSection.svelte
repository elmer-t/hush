<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    title: string;
    /** Shown after the title, e.g. a count. */
    badge?: string;
    /** Whether the section is expanded. */
    open: boolean;
    ontoggle: (open: boolean) => void;
    children: Snippet;
  };
  let { title, badge = "", open, ontoggle, children }: Props = $props();
</script>

<!-- A collapsible section of the details panel. -->
<section aria-label={title}>
  <button class="ghost head" aria-expanded={open} onclick={() => ontoggle(!open)}>
    <Icon name={open ? "chevronDown" : "chevronRight"} size={14} strokeWidth={2} />
    <h2>{title}</h2>
    {#if badge}<span class="badge">{badge}</span>{/if}
  </button>
  {#if open}
    {@render children()}
  {/if}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 16px 16px;
    border-top: 1px solid var(--border);
  }

  .head {
    align-self: flex-start;
    gap: 4px;
    margin-left: -6px;
    padding: 0 6px;
    color: var(--text);
  }

  h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }

  .badge {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: normal;
    color: var(--muted);
  }

  section :global(p) {
    margin: 0;
    font-size: 13px;
  }

  section :global(ul) {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
</style>
