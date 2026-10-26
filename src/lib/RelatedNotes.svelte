<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, errorMessage, RELATED_CHANGED_EVENT, type RelatedNote, type TitledNote } from "$lib/api";
  import DetailsSection from "$lib/DetailsSection.svelte";
  import Icon from "$lib/Icon.svelte";

  // Notes shown, most similar first.
  const LIMIT = 10;
  // Tags shown as chips before "+n".
  const TAG_CHIPS = 2;

  type Props = {
    /** The open note. */
    id: number;
    /** Bumped when the open note was saved or links changed elsewhere: the reasons may differ. */
    revision?: number;
    /** Whether the section is expanded. */
    open: boolean;
    ontoggle: (open: boolean) => void;
    onopen: (id: number) => void;
  };
  let { id, revision = 0, open, ontoggle, onopen }: Props = $props();

  // `null`: the note has no vector yet.
  let notes = $state<RelatedNote[] | null>([]);
  let loaded = $state(false);
  let error = $state("");
  // The note whose full breakdown is shown.
  let expanded = $state<number | null>(null);
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

  // Another note opened (shown empty until loaded), or the open note was saved or a link
  // to it changed (keeping what is shown).
  let shownId = 0;
  $effect(() => {
    void revision;
    if (id !== shownId) {
      shownId = id;
      loaded = false;
      expanded = null;
    }
    void load(id);
  });

  // The index changed: a note was re-embedded, trashed, restored or deleted.
  onMount(() => {
    const unlisten = listen(RELATED_CHANGED_EVENT, () => void load(id));
    return () => void unlisten.then((stop) => stop());
  });

  type Chip = { label: string; kind: "link" | "tag" | "same" };

  function linked(r: RelatedNote["reasons"]): string | null {
    if (r.linksTo && r.linkedFrom) return "Linked ↔";
    if (r.linksTo) return "Linked →";
    if (r.linkedFrom) return "Linked ←";
    return null;
  }

  function chips(n: RelatedNote): Chip[] {
    const r = n.reasons;
    const out: Chip[] = [];
    const direct = linked(r);
    if (direct) out.push({ label: direct, kind: "link" });
    const hops = new Set([...r.bothLinkTo, ...r.bothLinkedFrom, ...r.via].map((x) => x.id)).size;
    if (hops) out.push({ label: `2-hop ×${hops}`, kind: "link" });
    for (const tag of r.sharedTags.slice(0, TAG_CHIPS)) out.push({ label: `#${tag}`, kind: "tag" });
    if (r.sharedTags.length > TAG_CHIPS) out.push({ label: `+${r.sharedTags.length - TAG_CHIPS}`, kind: "tag" });
    if (r.sameFolder) out.push({ label: "Folder", kind: "same" });
    if (r.sameAuthor) out.push({ label: "Author", kind: "same" });
    if (r.sameTopic) out.push({ label: "Topic", kind: "same" });
    return out;
  }

  const names = (notes: TitledNote[]) => notes.map((n) => n.title).join(", ");

  /** Every reason as a line, for the hover text and the expanded view. */
  function breakdown(n: RelatedNote): [string, string][] {
    const r = n.reasons;
    const lines: [string, string][] = [["Meaning", `${n.score.toFixed(3)} cosine similarity`]];
    if (r.linksTo) lines.push(["Linked", "this note links to it"]);
    if (r.linkedFrom) lines.push(["Linked", "it links to this note"]);
    if (r.bothLinkTo.length) lines.push(["Both link to", names(r.bothLinkTo)]);
    if (r.bothLinkedFrom.length) lines.push(["Both linked from", names(r.bothLinkedFrom)]);
    if (r.via.length) lines.push(["Linked via", names(r.via)]);
    if (r.sharedTags.length) lines.push(["Shared tags", r.sharedTags.map((t) => `#${t}`).join(" ")]);
    if (r.sameFolder) lines.push(["Same folder", n.note.folder || "(root)"]);
    if (r.sameAuthor) lines.push(["Same author", r.sameAuthor]);
    if (r.sameTopic) lines.push(["Same topic", r.sameTopic]);
    return lines;
  }

  const hover = (n: RelatedNote) =>
    [n.note.folder ? `${n.note.folder}/${n.note.title}` : n.note.title, ...breakdown(n).map(([k, v]) => `${k}: ${v}`)].join("\n");
</script>

<DetailsSection title="Related" {open} {ontoggle}>
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
      {#each notes as n (n.note.id)}
        <li>
          <div class="row">
            <button class="ghost note" onclick={() => onopen(n.note.id)} title={hover(n)}>
              <span class="title">{n.note.title}</span>
              {#if n.note.folder}<span class="folder">{n.note.folder}</span>{/if}
              <span class="score">
                <span class="bar" aria-hidden="true"><span style:width="{Math.max(0, n.score) * 100}%"></span></span>
                <span class="num">{n.score.toFixed(2)}</span>
                {#each chips(n) as chip (chip.label)}
                  <span class="chip {chip.kind}">{chip.label}</span>
                {/each}
              </span>
            </button>
            <button
              class="ghost icon more"
              aria-expanded={expanded === n.note.id}
              aria-label="Why related: {n.note.title}"
              title="Why related"
              onclick={() => (expanded = expanded === n.note.id ? null : n.note.id)}
            >
              <Icon name={expanded === n.note.id ? "chevronDown" : "chevronRight"} size={12} strokeWidth={2} />
            </button>
          </div>
          {#if expanded === n.note.id}
            <dl class="why">
              {#each breakdown(n) as [key, value], i (i)}
                <dt>{key}</dt>
                <dd>{value}</dd>
              {/each}
            </dl>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</DetailsSection>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: 2px;
  }

  .note {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    min-width: 0;
    height: auto;
    min-height: 0;
    margin-left: -6px;
    padding: 4px 6px;
    text-align: left;
    font-weight: normal;
  }

  .more {
    flex: none;
    width: 22px;
    height: 22px;
    margin-top: 4px;
    color: var(--muted);
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

  .score {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 3px 4px;
    margin-top: 2px;
  }

  .bar {
    display: inline-block;
    width: 36px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .num {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-2);
  }

  .chip {
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: 3px;
    font-family: var(--mono);
    font-size: 10px;
    line-height: 15px;
    color: var(--text-2);
  }

  .chip.link {
    border-color: var(--accent);
    color: var(--accent);
  }

  .why {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 8px;
    margin: 2px 0 6px;
    padding: 6px 8px;
    border-left: 2px solid var(--border-strong);
    font-size: 12px;
  }

  .why dt {
    color: var(--muted);
  }

  .why dd {
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--text);
  }
</style>
