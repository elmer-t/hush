<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage, type SuggestionThreshold, type TagInfo } from "$lib/api";
  import TagTraining from "$lib/TagTraining.svelte";

  let tags = $state<TagInfo[]>([]);
  let loaded = $state(false);
  let error = $state("");
  // The tag whose description was saved last, for a short confirmation next to it.
  let savedTag = $state("");

  let newTag = $state("");
  let newDescription = $state("");
  let newError = $state("");

  let threshold = $state<SuggestionThreshold | null>(null);
  let thresholdValue = $state<number | null>(null);
  let thresholdStatus = $state("");
  let thresholdError = $state("");

  onMount(() => {
    void load();
  });

  async function load() {
    try {
      [tags, threshold] = await Promise.all([api.tagList(), api.suggestionThreshold()]);
      thresholdValue = threshold.threshold;
      error = "";
    } catch (e) {
      error = errorMessage(e);
    }
    loaded = true;
  }

  // Saved on change, like the other inline fields.
  async function saveDescription(info: TagInfo, value: string) {
    const description = value.trim();
    if (description === info.description) return;
    savedTag = error = "";
    try {
      await api.setTagDescription(info.tag, description);
      tags = await api.tagList();
      savedTag = info.tag;
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function addTag(e: SubmitEvent) {
    e.preventDefault();
    newError = "";
    const tag = newTag.trim().replace(/^#/, "");
    if (!newDescription.trim()) {
      newError = "Describe the tag in a sentence.";
      return;
    }
    try {
      await api.setTagDescription(tag, newDescription);
      tags = await api.tagList();
      savedTag = tag;
      newTag = newDescription = "";
    } catch (err) {
      newError = errorMessage(err);
    }
  }

  async function saveThreshold(e: SubmitEvent) {
    e.preventDefault();
    thresholdStatus = thresholdError = "";
    try {
      await api.setSuggestionThreshold(thresholdValue);
      threshold = await api.suggestionThreshold();
      thresholdValue = threshold.threshold;
      thresholdStatus = "Saved.";
    } catch (err) {
      thresholdError = errorMessage(err);
    }
  }

  async function resetThreshold() {
    thresholdStatus = thresholdError = "";
    try {
      await api.setSuggestionThreshold(null);
      threshold = await api.suggestionThreshold();
      thresholdValue = threshold.threshold;
      thresholdStatus = "Back to the default.";
    } catch (err) {
      thresholdError = errorMessage(err);
    }
  }
</script>

<section class="card tags" aria-labelledby="tags-heading">
  <h2 id="tags-heading">Tags</h2>
  <p class="muted">
    Describe a tag in a sentence and Hush suggests it on notes whose meaning is close to it. Until a tag has a
    classifier (below), it is only suggested if it has a description. Tags are case-sensitive.
  </p>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if loaded && tags.length === 0}
    <p class="muted">No tags yet.</p>
  {:else}
    <ul class="list">
      {#each tags as info (info.tag)}
        <li>
          <div class="name">
            <span class="tag">#{info.tag}</span>
            <span class="muted count">{info.notes === 1 ? "1 note" : `${info.notes} notes`}</span>
            {#if savedTag === info.tag}<span class="muted count">Saved.</span>{/if}
          </div>
          <input
            value={info.description}
            placeholder="No description: not suggested"
            aria-label="Description of {info.tag}"
            onchange={(e) => saveDescription(info, e.currentTarget.value)}
          />
        </li>
      {/each}
    </ul>
  {/if}

  <form class="add" onsubmit={addTag}>
    <h3>Describe a new tag</h3>
    <div class="row">
      <input class="mono" placeholder="tag" aria-label="New tag" bind:value={newTag} required />
      <input placeholder="dreams, nightmares, sleep" aria-label="New tag description" bind:value={newDescription} />
      <button type="submit">Add</button>
    </div>
    {#if newError}<p class="error" role="alert">{newError}</p>{/if}
  </form>

  <TagTraining {tags} />

  {#if threshold}
    <form class="threshold" onsubmit={saveThreshold}>
      <h3>Suggestion threshold</h3>
      <label>
        Suggest a tag when a note's similarity to its description is at least
        <input type="number" min="-1" max="1" step="0.01" bind:value={thresholdValue} required />
      </label>
      <p class="muted">
        For model <span class="mono">{threshold.model}</span>{#if threshold.default !== null}; default
          {threshold.default}{/if}. Higher means fewer, surer suggestions.
      </p>
      {#if thresholdError}<p class="error" role="alert">{thresholdError}</p>{/if}
      <div class="actions">
        {#if thresholdStatus}<span class="muted">{thresholdStatus}</span>{/if}
        {#if threshold.default !== null}
          <button type="button" class="ghost" onclick={resetThreshold}>Use default</button>
        {/if}
        <button type="submit">Save</button>
      </div>
    </form>
  {/if}
</section>

<style>
  .tags {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }

  p {
    margin: 0;
    font-size: 13px;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list li {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .name {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .tag {
    font-size: 13px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .count {
    font-size: 12px;
  }

  input {
    min-height: 32px;
    border-radius: 8px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }

  .row {
    display: grid;
    grid-template-columns: 120px minmax(0, 1fr) auto;
    gap: 6px;
  }

  .threshold input {
    max-width: 120px;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
  }
</style>
