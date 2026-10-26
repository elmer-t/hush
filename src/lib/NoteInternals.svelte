<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, errorMessage, RELATED_CHANGED_EVENT, type NoteInternals } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    /** The open note. */
    id: number;
    /** When it was last saved, so a save reloads the section. */
    updatedAt: number;
    /** Whether the section is expanded. */
    open: boolean;
  };
  let { id, updatedAt, open = $bindable() }: Props = $props();

  let internals = $state<NoteInternals | null>(null);
  let error = $state("");
  // Answers to an earlier request are dropped.
  let request = 0;

  async function load(noteId: number) {
    const mine = ++request;
    try {
      const result = await api.noteInternals(noteId);
      if (mine !== request) return;
      internals = result;
      error = "";
    } catch (e) {
      if (mine !== request) return;
      error = errorMessage(e);
    }
  }

  // Another note opened, or this one was saved; only while expanded.
  $effect(() => {
    void updatedAt;
    if (open) void load(id);
  });

  // A note was (re-)embedded: the vector and queue state may have changed.
  onMount(() => {
    const unlisten = listen(RELATED_CHANGED_EVENT, () => {
      if (open) void load(id);
    });
    return () => void unlisten.then((stop) => stop());
  });

  const date = (ms: number) => new Date(ms).toLocaleString();
  const short = (hash: string) => `${hash.slice(0, 12)}…`;
</script>

<section aria-label="Note internals">
  <button class="ghost head" aria-expanded={open} onclick={() => (open = !open)}>
    <Icon name={open ? "chevronDown" : "chevronRight"} size={14} strokeWidth={2} />
    <h2>Internals</h2>
  </button>
  {#if open}
    {#if error}
      <p class="error">{error}</p>
    {:else if internals && internals.id === id}
      {@const current = internals.vectors.find((v) => v.model === internals?.model)}
      <dl>
        <dt>Id</dt>
        <dd>{internals.id}</dd>
        <dt>Folder</dt>
        <dd>{internals.folder || "(root)"}</dd>
        <dt>Created</dt>
        <dd>{date(internals.createdAt)}</dd>
        <dt>Updated</dt>
        <dd>{date(internals.updatedAt)}</dd>
        <dt>Size</dt>
        <dd>{internals.words.toLocaleString()} words, {internals.chars.toLocaleString()} chars</dd>
        <dt>Hash</dt>
        <dd title={internals.contentHash ?? undefined}>{internals.contentHash ? short(internals.contentHash) : "nothing to embed"}</dd>
        <dt>Vector</dt>
        <dd>
          {#if current}
            {current.model}{current.current ? "" : ", out of date"}
          {:else}
            none for {internals.model}
          {/if}
          {#each internals.vectors.filter((v) => v.model !== internals?.model) as other (other.model)}
            <span class="muted">also {other.model}</span>
          {/each}
        </dd>
        <dt>Queued</dt>
        <dd>{internals.queued ? "yes, waiting for the embedding job" : "no"}</dd>
        <dt>Links</dt>
        <dd>{internals.backlinks.toLocaleString()} in, {internals.outgoing.toLocaleString()} out</dd>
        <dt>Tags</dt>
        <dd>
          {#if internals.tags.length === 0}
            none
          {:else}
            {#each internals.tags as t (t.tag)}
              <span class="tag">{t.tag} <span class="muted">{t.source === "suggestion" ? "accepted suggestion" : "manual"}</span></span>
            {/each}
          {/if}
        </dd>
      </dl>
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

  dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 10px;
    margin: 0;
    font-size: 12px;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
    font-family: var(--mono);
    font-size: 11.5px;
  }

  dd > span {
    display: block;
  }
</style>
