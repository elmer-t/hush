<script lang="ts">
  import type { NoteSummary } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    notes: NoteSummary[];
    selectedId: number | null;
    collapsed: boolean;
    ontoggle: () => void;
    /** Click or Enter on a note; the event is passed on like a click in the tree. */
    onopen: (id: number, event: MouseEvent | KeyboardEvent) => void;
  };
  let { notes, selectedId, collapsed, ontoggle, onopen }: Props = $props();
</script>

<section class="recent" aria-label="Recently opened">
  <button class="heading" aria-expanded={!collapsed} onclick={ontoggle}>
    <Icon name={collapsed ? "chevronRight" : "chevronDown"} size={14} strokeWidth={2} />
    <span class="label">Recently opened</span>
    <span class="count">{notes.length}</span>
  </button>
  {#if !collapsed}
    <ul>
      {#each notes as note (note.id)}
        <li>
          <button
            class="row"
            class:selected={note.id === selectedId}
            aria-current={note.id === selectedId}
            title={note.folder ? `${note.folder}/${note.title}` : note.title}
            onclick={(e) => onopen(note.id, e)}
          >
            <span class="glyph"><Icon name="note" size={14} /></span>
            <span class="title">{note.title}</span>
            {#if note.folder}<span class="folder">{note.folder}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .recent {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 8px 6px;
    border-bottom: 1px solid var(--border);
  }

  .heading {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    min-height: 26px;
    padding: 0 8px;
    border: none;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }

  .label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .count {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  ul {
    max-height: 30vh;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: 6px;
    background: none;
    color: inherit;
    font-size: 13.5px;
    text-align: left;
    cursor: default;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.selected {
    background: var(--selection);
    color: var(--selection-text);
    font-weight: 500;
  }

  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .glyph {
    flex: none;
    display: flex;
    opacity: 0.7;
  }

  .title {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
  }

  .row.selected .folder {
    color: inherit;
    opacity: 0.8;
  }
</style>
