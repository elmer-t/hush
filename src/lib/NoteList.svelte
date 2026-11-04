<script lang="ts">
  import type { SearchHit } from "$lib/api";
  import { shortDate } from "$lib/dates";
  import Icon from "$lib/Icon.svelte";
  import { readStatusOf } from "$lib/readStatus.svelte";
  import ReadMark from "$lib/ReadMark.svelte";
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
  const rowHeight = $derived(hits.some((h) => h.snippet) ? 100 : 64);

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
        aria-pressed={picked.size > 0 ? picked.has(note.id) : undefined}
        style:top="{(first + i) * rowHeight}px"
        style:height="{rowHeight - 2}px"
        onclick={(e) => onselect(note.id, e)}
      >
        {#if picked.size > 0}
          <span class="check" class:on={picked.has(note.id)} aria-hidden="true">
            {#if picked.has(note.id)}<Icon name="check" size={12} strokeWidth={3} />{/if}
          </span>
        {/if}
        <span class="text">
          <span class="head">
            <span class="title">{note.title}</span>
            {#if note.rating}<span class="rating" aria-label="{note.rating} of 5 stars">★{note.rating}</span>{/if}
            <ReadMark status={readStatusOf(note)} />
            <span class="date">{shortDate(note.updatedAt)}</span>
          </span>
          <span class="folder">{note.folder || "vault root"}</span>
          {#if snippet}
            <span class="snippet">
              {#each highlightParts(snippet) as part, j (j)}
                {#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}
              {/each}
            </span>
          {/if}
        </span>
      </button>
    {/each}
  </div>
</div>

<style>
  .list {
    overflow-y: auto;
    min-height: 0;
    padding: 0 8px 8px;
  }

  .spacer {
    position: relative;
  }

  .empty {
    padding: 4px 8px;
    margin: 0;
  }

  .item {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
    gap: 10px;
    padding: 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: none;
    color: var(--text);
    text-align: left;
    white-space: normal;
    overflow: hidden;
  }

  .item:hover:not(:disabled) {
    border-color: transparent;
    background: var(--hover);
  }

  .item.selected,
  .item.selected:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .item.picked {
    background: var(--selection);
  }

  .check {
    flex: none;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
  }

  .check.on {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--accent-text);
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .title,
  .folder {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title {
    flex: 1;
    min-width: 0;
    font-weight: 500;
  }

  .rating {
    flex: none;
    font-size: 12px;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .date {
    flex: none;
    font-size: 12px;
    color: var(--muted);
  }

  .folder {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--muted);
  }

  .snippet {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    font-size: 13px;
    line-height: 1.35;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
</style>
