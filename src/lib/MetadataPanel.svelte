<script lang="ts">
  import { api, errorMessage, type NoteMeta, type NoteSummary, type ReadStatus, type TagSuggestion } from "$lib/api";
  import { longDate } from "$lib/dates";
  import Icon from "$lib/Icon.svelte";
  import ModelNotice from "$lib/ModelNotice.svelte";
  import { markRead, READ_STATUSES, readStatusOf } from "$lib/readStatus.svelte";
  import StarRating from "$lib/StarRating.svelte";

  type Props = {
    meta: NoteMeta;
    summary: NoteSummary;
    /** Sets a frontmatter field; an empty list removes it. */
    onset: (key: string, values: string[]) => void;
    /** Accepts a suggested tag: the parent saves pending edits, adds it and reloads the note. */
    onaccept: (tag: string) => Promise<void>;
    /** Bumped when the note's suggestions changed elsewhere (a reject in the Suggestions view). */
    revision?: number;
  };
  let { meta, summary, onset, onaccept, revision = 0 }: Props = $props();

  let suggestions = $state<TagSuggestion[]>([]);
  let suggestionError = $state("");
  // An accept or reject in flight, so a double click doesn't send it twice.
  let deciding = $state(false);

  // Reloaded for each note and after each save (its tags may have changed).
  $effect(() => {
    void revision;
    void loadSuggestions(summary.id, summary.updatedAt);
  });

  async function loadSuggestions(id: number, _updatedAt: number) {
    try {
      const found = await api.tagSuggestions(id);
      if (summary.id === id) suggestions = found;
    } catch {
      if (summary.id === id) suggestions = [];
    }
  }

  function suggestionTitle(s: TagSuggestion): string {
    const score = s.score.toFixed(2);
    if (s.method === "classifier") return `Classifier trained on your notes: probability ${score}`;
    if (s.method === "descriptionAndExamples") return `Similarity to the tag's description and examples: ${score}`;
    return `Similarity to the tag's description: ${score}`;
  }

  async function accept(tag: string) {
    if (deciding) return;
    deciding = true;
    suggestionError = "";
    try {
      await onaccept(tag);
    } finally {
      deciding = false;
    }
  }

  async function reject(tag: string) {
    if (deciding) return;
    deciding = true;
    suggestionError = "";
    const id = summary.id;
    try {
      await api.rejectTagSuggestion(id, tag);
      await loadSuggestions(id, summary.updatedAt);
    } catch (e) {
      suggestionError = errorMessage(e);
    } finally {
      deciding = false;
    }
  }

  // Kept in the vault, not the frontmatter, so marking doesn't save the note.
  const readStatus = $derived(readStatusOf(summary));
  let readError = $state("");

  async function setReadStatus(status: ReadStatus) {
    readError = "";
    try {
      await markRead([summary.id], status);
    } catch (e) {
      readError = errorMessage(e);
    }
  }

  // Whether the form for a new attribute is shown.
  let adding = $state(false);
  let newKey = $state("");
  let newValue = $state("");
  let attributeError = $state("");

  const attributes = $derived.by(() => {
    const groups = new Map<string, string[]>();
    for (const [key, value] of meta.attributes) groups.set(key, [...(groups.get(key) ?? []), value]);
    return [...groups];
  });

  function setText(key: string, value: string) {
    const v = value.trim();
    onset(key, v ? [v] : []);
  }

  function addTag(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key !== "Enter") return;
    const tag = e.currentTarget.value.trim().replace(/^#/, "");
    e.currentTarget.value = "";
    if (tag && !meta.tags.includes(tag)) onset("tags", [...meta.frontmatterTags, tag]);
  }

  function removeTag(tag: string) {
    onset(
      "tags",
      meta.frontmatterTags.filter((t) => t !== tag),
    );
  }

  function addAttribute(e: SubmitEvent) {
    e.preventDefault();
    const key = newKey.trim().toLowerCase();
    const value = newValue.trim();
    if (!/^[\p{L}\p{N}_-]+$/u.test(key)) {
      attributeError = "Use letters, digits, - and _ in the name.";
      return;
    }
    if (!value) return;
    const existing = attributes.find(([k]) => k === key)?.[1] ?? [];
    onset(key, [...existing, value]);
    newKey = newValue = attributeError = "";
    adding = false;
  }
</script>

<aside aria-label="Details">
  <div class="head">
    <h2>Details</h2>
    <span class="muted">writes frontmatter</span>
  </div>

  <div class="grid">
    <label>
      Author
      <input value={meta.author ?? ""} onchange={(e) => setText("author", e.currentTarget.value)} />
    </label>
    <label>
      Topic
      <input value={meta.topic ?? ""} onchange={(e) => setText("topic", e.currentTarget.value)} />
    </label>
    <div class="rating">
      <span>Rating</span>
      <StarRating value={meta.rating} onchange={(r) => onset("rating", r === null ? [] : [String(r)])} />
    </div>
    <div class="read">
      <span title="Kept in the vault, not in the note's frontmatter">Read status</span>
      <div class="segmented" role="group" aria-label="Read status">
        {#each READ_STATUSES as s (s.value)}
          <button aria-pressed={readStatus === s.value} onclick={() => void setReadStatus(s.value)}>{s.label}</button>
        {/each}
      </div>
      {#if readError}<span class="error" role="alert">{readError}</span>{/if}
    </div>
    <div class="date">
      Created
      <span>{longDate(summary.createdAt)}</span>
    </div>
    <div class="date">
      Updated
      <span>{longDate(summary.updatedAt)}</span>
    </div>
  </div>

  <div class="field">
    <span class="label">Tags</span>
    <ul class="chips">
      {#each meta.tags as tag (tag)}
        {#if meta.frontmatterTags.includes(tag)}
          <li class="chip">
            {tag}
            <button aria-label="Remove tag {tag}" onclick={() => removeTag(tag)}>
              <Icon name="close" size={12} strokeWidth={2} />
            </button>
          </li>
        {:else}
          <li class="chip outline" title="From #{tag} in the text; remove it there.">#{tag}</li>
        {/if}
      {/each}
      <li><input class="add-chip" placeholder="Add tag" aria-label="Add tag" onkeydown={addTag} /></li>
    </ul>
    {#if suggestions.length > 0}
      <ul class="suggestions" aria-label="Suggested tags">
        {#each suggestions as s (s.tag)}
          <li title={suggestionTitle(s)}>
            <span class="muted">Suggested:</span>
            <span class="suggested">#{s.tag}</span>
            <button class="ghost small" disabled={deciding} aria-label="Accept tag {s.tag}" onclick={() => accept(s.tag)}>
              <Icon name="check" size={12} strokeWidth={2} />
              Accept
            </button>
            <button class="ghost small" disabled={deciding} aria-label="Reject tag {s.tag}" onclick={() => reject(s.tag)}>
              <Icon name="close" size={12} strokeWidth={2} />
              Reject
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <ModelNotice what="Tag suggestions" />
    {/if}
    {#if suggestionError}<p class="error">{suggestionError}</p>{/if}
  </div>

  <div class="field">
    <span class="label">Attributes</span>
    {#each attributes as [key, values] (key)}
      <div class="attribute">
        <span class="key">{key}</span>
        <span class="value">{values.join(", ")}</span>
        <button class="ghost small icon" aria-label="Remove {key}" onclick={() => onset(key, [])}>
          <Icon name="close" size={12} strokeWidth={2} />
        </button>
      </div>
    {/each}
    {#if adding}
      <form class="add-attribute" onsubmit={addAttribute}>
        <input
          class="mono"
          placeholder="name"
          aria-label="Attribute name"
          bind:value={newKey}
          onkeydown={(e) => e.key === "Escape" && (adding = false)}
          {@attach (el) => el.focus()}
        />
        <input placeholder="value" aria-label="Attribute value" bind:value={newValue} />
        <button type="submit" class="small">Add</button>
      </form>
      {#if attributeError}<p class="error">{attributeError}</p>{/if}
    {:else}
      <button class="ghost small add" onclick={() => (adding = true)}>
        <Icon name="plus" size={14} strokeWidth={2} />
        Add attribute
      </button>
    {/if}
  </div>
</aside>

<style>
  aside {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
  }

  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    font-size: 12px;
  }

  h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .grid input {
    min-height: 30px;
  }

  .rating,
  .read {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12px;
    color: var(--muted);
  }

  .date {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .date span {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 12px;
    color: var(--muted);
  }

  .attribute {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
    font-size: 13px;
  }

  .key {
    flex: 0 0 72px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .add-attribute {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr) auto;
    gap: 6px;
  }

  .add-attribute input {
    min-height: 30px;
    font-size: 13px;
  }

  .add-attribute .mono {
    font-size: 12px;
  }

  .suggestions {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .suggestions li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }

  .suggested {
    color: var(--accent);
  }

  .suggestions button {
    padding: 0 6px;
  }

  .add {
    align-self: flex-start;
    padding: 0 8px;
    color: var(--accent);
  }

  .add:hover:not(:disabled) {
    color: var(--accent);
  }
</style>
