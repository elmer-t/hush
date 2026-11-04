<script lang="ts">
  import { api, type Facets } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import { READ_FILTERS } from "$lib/readStatus.svelte";
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
  <div class="head">
    <span class="title">Filters</span>
    <button class="link small-link" onclick={() => (filters = emptyFilters())}>Clear all</button>
  </div>

  <div class="field">
    <span class="label">Tags (all of)</span>
    <ul class="chips">
      {#each filters.tags as tag (tag)}
        <li class="chip">
          {tag}
          <button
            aria-label="Remove tag filter {tag}"
            onclick={() => (filters.tags = filters.tags.filter((t) => t !== tag))}
          >
            <Icon name="close" size={12} strokeWidth={2} />
          </button>
        </li>
      {/each}
      <li>
        <input class="add-chip" list="filter-tags" placeholder="Add tag" aria-label="Add tag filter" onkeydown={addTag} />
      </li>
    </ul>
    <datalist id="filter-tags">
      {#each facets.tags as tag (tag)}<option value={tag}></option>{/each}
    </datalist>
  </div>

  <div class="pair">
    <label class:set={filters.topic}>
      Topic
      <select bind:value={filters.topic}>
        <option value="">Any</option>
        {#each facets.topics as topic (topic)}<option value={topic}>{topic}</option>{/each}
      </select>
    </label>
    <label class:set={filters.author}>
      Author
      <select bind:value={filters.author}>
        <option value="">Any</option>
        {#each facets.authors as author (author)}<option value={author}>{author}</option>{/each}
      </select>
    </label>
    <label class:set={filters.minRating > 0}>
      Rating
      <select bind:value={filters.minRating}>
        <option value={0}>Any</option>
        {#each [1, 2, 3, 4] as n (n)}<option value={n}>★ {n}+</option>{/each}
        <option value={5}>★ 5</option>
      </select>
    </label>
    <label class:set={filters.readStatus}>
      Read status
      <select bind:value={filters.readStatus}>
        <option value="">Any</option>
        {#each READ_FILTERS as f (f.value)}<option value={f.value}>{f.label}</option>{/each}
      </select>
    </label>
  </div>

  <div class="dates">
    <div class="segmented" role="group" aria-label="Date to filter on">
      <button aria-pressed={filters.dateField === "created"} onclick={() => (filters.dateField = "created")}>Created</button>
      <button aria-pressed={filters.dateField === "updated"} onclick={() => (filters.dateField = "updated")}>Updated</button>
    </div>
    <input type="date" aria-label="From" max={filters.to || undefined} bind:value={filters.from} />
    <span class="muted">–</span>
    <input type="date" aria-label="To" min={filters.from || undefined} bind:value={filters.to} />
  </div>

  <div class="field">
    {#if filters.attributes.length > 0}
      <ul class="chips">
        {#each filters.attributes as [key, value] (`${key}=${value}`)}
          <li class="chip">
            {key} = {value}
            <button
              aria-label="Remove attribute filter {key}"
              onclick={() => (filters.attributes = filters.attributes.filter(([k, v]) => k !== key || v !== value))}
            >
              <Icon name="close" size={12} strokeWidth={2} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <form class="add-attribute" onsubmit={addAttribute}>
      <select bind:value={attrKey} aria-label="Attribute name" disabled={facets.attributeKeys.length === 0}>
        <option value="">attribute</option>
        {#each facets.attributeKeys as key (key)}<option value={key}>{key}</option>{/each}
      </select>
      <span class="muted">=</span>
      <input list="filter-attr-values" placeholder="value" aria-label="Attribute value" bind:value={attrValue} />
      <datalist id="filter-attr-values">
        {#each attrValues as value (value)}<option {value}></option>{/each}
      </datalist>
      <button type="submit" class="small" disabled={!attrKey}>Add</button>
    </form>
  </div>
</div>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }

  .small-link {
    font-size: 12px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .label {
    font-size: 12px;
    color: var(--muted);
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }

  .pair select {
    min-height: 30px;
  }

  /* A filter that is in use. */
  .set select {
    border-color: var(--accent);
  }

  .dates {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .dates .segmented {
    border-radius: 6px;
  }

  .dates .segmented button {
    min-height: 24px;
    padding: 0 8px;
    border-radius: 4px;
    font-size: 12px;
  }

  .dates input {
    flex: 1 1 110px;
    width: auto;
    min-width: 0;
    min-height: 28px;
    padding: 0 6px;
    font-family: var(--mono);
    font-size: 12px;
  }

  .add-attribute {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .add-attribute select,
  .add-attribute input {
    flex: 1;
    min-width: 0;
    min-height: 28px;
    padding: 0 6px;
    font-family: var(--mono);
    font-size: 12px;
  }
</style>
