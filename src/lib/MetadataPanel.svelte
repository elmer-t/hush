<script lang="ts">
  import type { NoteMeta, NoteSummary } from "$lib/api";
  import { longDate } from "$lib/dates";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    meta: NoteMeta;
    summary: NoteSummary;
    /** Sets a frontmatter field; an empty list removes it. */
    onset: (key: string, values: string[]) => void;
  };
  let { meta, summary, onset }: Props = $props();

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

  .add {
    align-self: flex-start;
    padding: 0 8px;
    color: var(--accent);
  }

  .add:hover:not(:disabled) {
    color: var(--accent);
  }
</style>
