<script lang="ts">
  import type { NoteSummary } from "$lib/api";
  import { shortDate } from "$lib/dates";

  type Props = {
    notes: NoteSummary[];
    selectedId: number | null;
    hasMore: boolean;
    onselect: (id: number) => void;
    /** Asks for the next page when the user nears the end of the list. */
    onmore: () => void;
  };
  let { notes, selectedId, hasMore, onselect, onmore }: Props = $props();

  // Rows have a fixed height, so only the visible slice (plus a margin) is rendered.
  const ROW_HEIGHT = 52;
  const OVERSCAN = 8;

  let scrollTop = $state(0);
  let height = $state(0);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN));
  const last = $derived(Math.min(notes.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN));

  $effect(() => {
    if (hasMore && last >= notes.length - OVERSCAN) onmore();
  });
</script>

<div class="list" bind:clientHeight={height} onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}>
  {#if notes.length === 0}
    <p class="muted empty">No notes here yet.</p>
  {/if}
  <div class="spacer" style:height="{notes.length * ROW_HEIGHT}px">
    {#each notes.slice(first, last) as note, i (note.id)}
      <button
        class="item"
        class:selected={note.id === selectedId}
        aria-current={note.id === selectedId}
        style:top="{(first + i) * ROW_HEIGHT}px"
        style:height="{ROW_HEIGHT}px"
        onclick={() => onselect(note.id)}
      >
        <span class="title">{note.title}</span>
        <span class="sub muted">
          <span class="folder">{note.folder}</span>
          <span>{shortDate(note.updatedAt)}</span>
        </span>
      </button>
    {/each}
  </div>
</div>

<style>
  .list {
    overflow-y: auto;
    min-height: 0;
  }

  .spacer {
    position: relative;
  }

  .empty {
    padding: 0.5rem 0.75rem;
    margin: 0;
  }

  .item {
    position: absolute;
    left: 0;
    right: 0;
    display: grid;
    align-content: center;
    gap: 0.1rem;
    padding: 0 0.75rem;
    border: none;
    border-radius: 0;
    background: none;
    text-align: left;
  }

  .item:hover {
    background: var(--active-line);
  }

  .item.selected {
    background: var(--selection);
  }

  .title,
  .folder {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title {
    font-weight: 600;
  }

  .sub {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.8em;
  }

  .sub span:last-child {
    flex: none;
  }
</style>
