<script lang="ts">
  import { api, type Facets } from "$lib/api";
  import { emptyFilters, type Filters } from "$lib/search";

  type Props = { filters: Filters; facets: Facets };
  let { filters = $bindable(), facets }: Props = $props();

  let attrKey = $state("");
  let attrValue = $state("");
  let attrValues = $state<string[]>([]);

  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

  $effect(() => {
    const key = attrKey;
    attrValues = [];
    if (key) void api.attributeValues(key).then((v) => key === attrKey && (attrValues = v), () => {});
  });

  function addTag(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key !== "Enter") return;
    const tag = e.currentTarget.value.trim().replace(/^#/, "");
    e.currentTarget.value = "";
    if (tag && !filters.tags.includes(tag)) filters.tags = [...filters.tags, tag];
  }

  function addAttribute(e: SubmitEvent) {
    e.preventDefault();
    const value = attrValue.trim();
    if (!attrKey || !value) return;
    if (!filters.attributes.some(([k, v]) => same(k, attrKey) && same(v, value)))
      filters.attributes = [...filters.attributes, [attrKey, value]];
    attrValue = "";
  }
</script>

<div class="filters">
  <div class="field">
    <span class="label">Tags (all must match)</span>
    {#if filters.tags.length > 0}
      <ul class="chips">
        {#each filters.tags as tag (tag)}
          <li class="chip">
            #{tag}
            <button
              aria-label="Remove tag filter {tag}"
              onclick={() => (filters.tags = filters.tags.filter((t) => t !== tag))}>×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
    <input list="filter-tags" placeholder="Add tag and press Enter" aria-label="Add tag filter" onkeydown={addTag} />
    <datalist id="filter-tags">
      {#each facets.tags as tag (tag)}<option value={tag}></option>{/each}
    </datalist>
  </div>

  <label>
    Topic
    <select bind:value={filters.topic}>
      <option value="">Any topic</option>
      {#each facets.topics as topic (topic)}<option value={topic}>{topic}</option>{/each}
    </select>
  </label>

  <label>
    Author
    <select bind:value={filters.author}>
      <option value="">Any author</option>
      {#each facets.authors as author (author)}<option value={author}>{author}</option>{/each}
    </select>
  </label>

  <fieldset class="field">
    <legend class="label">
      <select bind:value={filters.dateField} aria-label="Date to filter on">
        <option value="created">Created</option>
        <option value="updated">Updated</option>
      </select>
    </legend>
    <div class="dates">
      <input type="date" aria-label="From" max={filters.to || undefined} bind:value={filters.from} />
      <span class="muted">–</span>
      <input type="date" aria-label="To" min={filters.from || undefined} bind:value={filters.to} />
    </div>
  </fieldset>

  <div class="field">
    <span class="label">Attributes</span>
    {#if filters.attributes.length > 0}
      <ul class="chips">
        {#each filters.attributes as [key, value] (`${key}=${value}`)}
          <li class="chip">
            {key} = {value}
            <button
              aria-label="Remove attribute filter {key}"
              onclick={() => (filters.attributes = filters.attributes.filter(([k, v]) => k !== key || v !== value))}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
    <form class="add-attribute" onsubmit={addAttribute}>
      <select bind:value={attrKey} aria-label="Attribute name" disabled={facets.attributeKeys.length === 0}>
        <option value="">Name</option>
        {#each facets.attributeKeys as key (key)}<option value={key}>{key}</option>{/each}
      </select>
      <input list="filter-attr-values" placeholder="Value" aria-label="Attribute value" bind:value={attrValue} />
      <datalist id="filter-attr-values">
        {#each attrValues as value (value)}<option {value}></option>{/each}
      </datalist>
      <button type="submit" disabled={!attrKey}>Add</button>
    </form>
  </div>

  <button class="clear" onclick={() => (filters = emptyFilters())}>Clear filters</button>
</div>

<style>
  .filters {
    display: grid;
    gap: 0.75rem;
    padding: 0 0.75rem 0.75rem;
  }

  .field {
    display: grid;
    gap: 0.35rem;
    margin: 0;
    padding: 0;
    border: none;
    min-width: 0;
  }

  .label {
    padding: 0;
    font-size: 0.9em;
    color: var(--muted);
  }

  legend select {
    width: auto;
    padding: 0.1em 0.3em;
  }

  .dates {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 0.3rem;
  }

  .dates input {
    padding: 0.4em;
    min-width: 0;
  }

  .chips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    padding: 0.1em 0.5em;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 0.85em;
    overflow-wrap: anywhere;
  }

  .chip button {
    border: none;
    background: none;
    padding: 0 0.2em;
    color: var(--muted);
  }

  .add-attribute {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 0.3rem;
  }

  .add-attribute > * {
    min-width: 0;
  }

  .clear {
    justify-self: start;
  }
</style>
