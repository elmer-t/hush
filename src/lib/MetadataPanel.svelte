<script lang="ts">
  import type { NoteMeta, NoteSummary } from "$lib/api";
  import { longDate } from "$lib/dates";

  type Props = {
    meta: NoteMeta;
    summary: NoteSummary;
    /** Sets a frontmatter field; an empty list removes it. */
    onset: (key: string, values: string[]) => void;
  };
  let { meta, summary, onset }: Props = $props();

  let newKey = $state("");
  let newValue = $state("");
  let attributeError = $state("");

  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

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
    if (tag && !meta.tags.some((t) => same(t, tag))) onset("tags", [...meta.frontmatterTags, tag]);
  }

  function removeTag(tag: string) {
    onset(
      "tags",
      meta.frontmatterTags.filter((t) => !same(t, tag)),
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
  }
</script>

<aside>
  <h2>Details</h2>

  <label>
    Author
    <input value={meta.author ?? ""} onchange={(e) => setText("author", e.currentTarget.value)} />
  </label>

  <label>
    Topic
    <input value={meta.topic ?? ""} onchange={(e) => setText("topic", e.currentTarget.value)} />
  </label>

  <div class="field">
    <span class="label">Tags</span>
    {#if meta.tags.length > 0}
      <ul class="tags">
        {#each meta.tags as tag (tag)}
          {#if meta.frontmatterTags.some((t) => same(t, tag))}
            <li class="tag">
              #{tag}
              <button aria-label="Remove tag {tag}" onclick={() => removeTag(tag)}>×</button>
            </li>
          {:else}
            <li class="tag inline" title="Written in the text; remove it there.">#{tag}</li>
          {/if}
        {/each}
      </ul>
    {/if}
    <input placeholder="Add tag and press Enter" aria-label="Add tag" onkeydown={addTag} />
  </div>

  <div class="field">
    <span class="label">Attributes</span>
    {#if attributes.length > 0}
      <dl>
        {#each attributes as [key, values] (key)}
          <dt>{key}</dt>
          <dd>
            <span>{values.join(", ")}</span>
            <button aria-label="Remove {key}" onclick={() => onset(key, [])}>×</button>
          </dd>
        {/each}
      </dl>
    {/if}
    <form class="add-attribute" onsubmit={addAttribute}>
      <input placeholder="Name" aria-label="Attribute name" bind:value={newKey} />
      <input placeholder="Value" aria-label="Attribute value" bind:value={newValue} />
      <button type="submit">Add</button>
    </form>
    {#if attributeError}<p class="error">{attributeError}</p>{/if}
  </div>

  <dl class="dates">
    <dt>Created</dt>
    <dd>{longDate(summary.createdAt)}</dd>
    <dt>Updated</dt>
    <dd>{longDate(summary.updatedAt)}</dd>
  </dl>
</aside>

<style>
  aside {
    display: grid;
    gap: 1rem;
    align-content: start;
    padding: 1rem;
  }

  h2 {
    margin: 0;
    font-size: 0.95rem;
  }

  .field {
    display: grid;
    gap: 0.35rem;
  }

  .label {
    font-size: 0.9em;
    color: var(--muted);
  }

  .tags {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    padding: 0.1em 0.5em;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 0.85em;
  }

  .tag.inline {
    border-style: dashed;
    color: var(--muted);
  }

  .tag button,
  dd button {
    border: none;
    background: none;
    padding: 0 0.2em;
    color: var(--muted);
  }

  dl {
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.25rem 0.75rem;
    font-size: 0.9em;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    overflow-wrap: anywhere;
  }

  .add-attribute {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 0.3rem;
  }

  .dates {
    border-top: 1px solid var(--border);
    padding-top: 0.75rem;
  }
</style>
