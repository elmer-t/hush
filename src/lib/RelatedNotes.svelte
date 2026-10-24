<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, errorMessage, RELATED_CHANGED_EVENT, type NoteSummary } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  // Notes shown, most similar first.
  const LIMIT = 10;

  type Props = {
    /** The open note. */
    id: number;
    /** Whether the section is expanded. */
    open: boolean;
    onopen: (id: number) => void;
  };
  let { id, open = $bindable(), onopen }: Props = $props();

  // `null`: the note has no vector yet.
  let notes = $state<NoteSummary[] | null>([]);
  let loaded = $state(false);
  let error = $state("");
  // Answers to an earlier note's request are dropped.
  let request = 0;

  async function load(noteId: number) {
    const mine = ++request;
    try {
      const result = await api.relatedNotes(noteId, LIMIT);
      if (mine !== request) return;
      notes = result;
      error = "";
    } catch (e) {
      if (mine !== request) return;
      error = errorMessage(e);
    }
    loaded = true;
  }

  // Another note opened.
  $effect(() => {
    loaded = false;
    void load(id);
  });

  // The index changed: a note was re-embedded, trashed, restored or deleted.
  onMount(() => {
    const unlisten = listen(RELATED_CHANGED_EVENT, () => void load(id));
    return () => void unlisten.then((stop) => stop());
  });
</script>

<section aria-label="Related notes">
  <button class="ghost head" aria-expanded={open} onclick={() => (open = !open)}>
    <Icon name={open ? "chevronDown" : "chevronRight"} size={14} strokeWidth={2} />
    <h2>Related</h2>
  </button>
  {#if open}
    {#if error}
      <p class="error">{error}</p>
    {:else if !loaded}
      <!-- Usually a few milliseconds: nothing, rather than a flash of text. -->
    {:else if notes === null}
      <p class="muted">Not indexed yet</p>
    {:else if notes.length === 0}
      <p class="muted">No related notes</p>
    {:else}
      <ul>
        {#each notes as note (note.id)}
          <li>
            <button class="ghost note" onclick={() => onopen(note.id)} title={note.folder ? `${note.folder}/${note.title}` : note.title}>
              <span class="title">{note.title}</span>
              {#if note.folder}<span class="folder">{note.folder}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
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

  p {
    margin: 0;
    font-size: 13px;
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .note {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    width: 100%;
    height: auto;
    min-height: 0;
    margin-left: -6px;
    padding: 4px 6px;
    text-align: left;
    font-weight: normal;
  }

  .title,
  .folder {
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .title {
    font-size: 13px;
    color: var(--text);
  }

  .folder {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
  }
</style>
