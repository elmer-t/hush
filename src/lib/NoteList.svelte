<script lang="ts">
  import type { SearchHit } from "$lib/api";
  import { shortDate } from "$lib/dates";
  import { highlightParts } from "$lib/search";
  import { selectionKeys } from "$lib/selection";

  type Props = {
    hits: SearchHit[];
    selectedId: number | null;
    /** Notes picked for a bulk action. */
    picked: ReadonlySet<number>;
    hasMore: boolean;
    /** Every click; the event tells a plain click from Ctrl/Cmd or Shift. */
    onselect: (id: number, e: MouseEvent) => void;
    onselectall: () => void;
    onclearselection: () => void;
    /** Asks for the next page when the user nears the end of the list. */
    onmore: () => void;
    emptyText: string;
  };
  let { hits, selectedId, picked, hasMore, onselect, onselectall, onclearselection, onmore, emptyText }: Props = $props();

  // Rows have a fixed height, so only the visible slice (plus a margin) is rendered.
  // Rows grow to fit two lines of snippet whenever any hit has one.
  const OVERSCAN = 8;
  const rowHeight = $derived(hits.some((h) => h.snippet) ? 96 : 52);

  let scrollTop = $state(0);
  let height = $state(0);

  const first = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN));
  const last = $derived(Math.min(hits.length, Math.ceil((scrollTop + height) / rowHeight) + OVERSCAN));

  $effect(() => {
    if (hasMore && last >= hits.length - OVERSCAN) onmore();
  });
</script>

<div
  class="list"
  bind:clientHeight={height}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  {@attach selectionKeys(onselectall, onclearselection)}
>
  {#if hits.length === 0}
    <p class="muted empty">{emptyText}</p>
  {/if}
  <div class="spacer" style:height="{hits.length * rowHeight}px">
    {#each hits.slice(first, last) as { note, snippet }, i (note.id)}
      <button
        class="item"
        class:selected={note.id === selectedId}
        class:picked={picked.has(note.id)}
        aria-current={note.id === selectedId}
        style:top="{(first + i) * rowHeight}px"
        style:height="{rowHeight}px"
        onclick={(e) => onselect(note.id, e)}
      >
        <span class="title">{note.title}</span>
        <span class="sub muted">
          <span class="folder">{note.folder}</span>
          <span>{shortDate(note.updatedAt)}</span>
        </span>
        {#if snippet}
          <span class="snippet muted">
            {#each highlightParts(snippet) as part, j (j)}
              {#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}
            {/each}
          </span>
        {/if}
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

  .item.picked {
    background: var(--selection);
    box-shadow: inset 3px 0 0 var(--accent);
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

  .snippet {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    font-size: 0.8em;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  mark {
    background: var(--highlight);
    color: inherit;
    border-radius: 2px;
  }
</style>
