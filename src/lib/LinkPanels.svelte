<script lang="ts">
  import { api, errorMessage, type Backlink, type DetailsSection as Section, type OutgoingLink } from "$lib/api";
  import DetailsSection from "$lib/DetailsSection.svelte";

  type Props = {
    /** The open note. */
    id: number;
    /** Bumped when the open note was saved or links may have changed elsewhere. */
    revision?: number;
    isOpen: (section: Section) => boolean;
    ontoggle: (section: Section, open: boolean) => void;
    onopen: (id: number) => void;
    /** Follows an outgoing link's target, as a click on it in the preview does. */
    onfollow: (target: string, x: number, y: number) => void;
  };
  let { id, revision = 0, isOpen, ontoggle, onopen, onfollow }: Props = $props();

  let backlinks = $state<Backlink[]>([]);
  let outgoing = $state<OutgoingLink[]>([]);
  let loaded = $state(false);
  let error = $state("");
  // Answers to an earlier request are dropped.
  let request = 0;

  async function load(noteId: number) {
    const mine = ++request;
    try {
      const [back, out] = await Promise.all([api.backlinks(noteId), api.outgoingLinks(noteId)]);
      if (mine !== request) return;
      backlinks = back;
      outgoing = out;
      error = "";
    } catch (e) {
      if (mine !== request) return;
      error = errorMessage(e);
    }
    loaded = true;
  }

  // Read again when another note opens (shown empty until loaded) and on each revision
  // (keeping what is shown).
  let shownId = 0;
  $effect(() => {
    void revision;
    if (id !== shownId) {
      shownId = id;
      loaded = false;
    }
    void load(id);
  });

  const path = (folder: string, title: string) => (folder ? `${folder}/${title}` : title);

  function follow(link: OutgoingLink, e: MouseEvent) {
    if (link.resolution.kind === "unique") onopen(link.resolution.ids);
    else onfollow(link.target, e.clientX, e.clientY);
  }

  function status(link: OutgoingLink): string {
    switch (link.resolution.kind) {
      case "unique":
        return link.notes[0] ? path(link.notes[0].folder, link.notes[0].title) : "";
      case "ambiguous":
        return `ambiguous: ${link.resolution.ids.length} notes`;
      case "missing":
        return "unresolved";
    }
  }
</script>

<DetailsSection
  title="Backlinks"
  badge={loaded ? String(backlinks.length) : ""}
  open={isOpen("backlinks")}
  ontoggle={(open) => ontoggle("backlinks", open)}
>
  {#if error}
    <p class="error">{error}</p>
  {:else if !loaded}
    <!-- Usually a few milliseconds. -->
  {:else if backlinks.length === 0}
    <p class="muted">No notes link here</p>
  {:else}
    <ul>
      {#each backlinks as b (b.note.id)}
        <li>
          <button class="ghost link" onclick={() => onopen(b.note.id)} title={path(b.note.folder, b.note.title)}>
            <span class="title">{b.note.title}</span>
            {#if b.note.folder}<span class="folder">{b.note.folder}</span>{/if}
            {#if b.snippet}<span class="snippet">{b.snippet}</span>{/if}
            {#if b.ambiguous}<span class="flag">ambiguous link</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</DetailsSection>

<DetailsSection
  title="Outgoing links"
  badge={loaded ? String(outgoing.length) : ""}
  open={isOpen("outgoing")}
  ontoggle={(open) => ontoggle("outgoing", open)}
>
  {#if error}
    <p class="error">{error}</p>
  {:else if !loaded}
    <!-- Usually a few milliseconds. -->
  {:else if outgoing.length === 0}
    <p class="muted">No links in this note</p>
  {:else}
    <ul>
      {#each outgoing as link (link.target)}
        <li>
          <button
            class="ghost link"
            class:unresolved={link.resolution.kind !== "unique"}
            onclick={(e) => follow(link, e)}
            title={status(link)}
          >
            <span class="title">{link.target}</span>
            <span class="folder">{status(link)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</DetailsSection>

<style>
  .link {
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

  .folder,
  .flag {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
  }

  .flag {
    color: var(--warn);
  }

  .snippet {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-size: 12px;
    color: var(--text-2);
  }

  .unresolved .title {
    color: var(--muted);
    font-style: italic;
  }
</style>
