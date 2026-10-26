<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { api, errorMessage, TAGS_TRAINED_EVENT, type TagInfo, type TagTraining } from "$lib/api";
  import ModelNotice from "$lib/ModelNotice.svelte";

  type Props = {
    /** The Tags settings' list; reloaded whenever it changes (a description was saved). */
    tags: TagInfo[];
  };
  let { tags }: Props = $props();

  let rows = $state<TagTraining[]>([]);
  let error = $state("");

  $effect(() => {
    void tags;
    void load();
  });

  onMount(() => {
    const unlisten = listen(TAGS_TRAINED_EVENT, () => void load());
    return () => void unlisten.then((stop) => stop());
  });

  async function load() {
    try {
      rows = await api.tagTraining();
      error = "";
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function status(row: TagTraining): string {
    const more = (n: number) => (n === 1 ? "1 more example" : `${n} more examples`);
    switch (row.status) {
      case "trained":
        return "Trained";
      case "waiting":
        return "Training soon";
      case "descriptionOnly":
        return `Description only · needs ${more(row.needed)}`;
      case "needsExamples":
        return `Needs ${more(row.needed)}`;
    }
  }

  function percent(value: number | null): string {
    return value === null ? "–" : `${Math.round(value * 100)}%`;
  }
</script>

<section class="training" aria-labelledby="training-heading">
  <h3 id="training-heading">Classifiers</h3>
  <p class="muted">
    Once a tag is on 10 notes, Hush trains a small classifier for it on your own notes, on this computer, and
    suggests the tag from that instead, with or without a description. Accepting and rejecting suggestions
    teaches it. Precision is how often a suggestion was right, recall how many of the tagged notes it found
    (tested on notes it wasn't trained on).
  </p>
  <ModelNotice what="Tag classifiers" />
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if rows.length > 0}
    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th scope="col">Tag</th>
            <th scope="col" class="num" title="Notes with the tag / notes without it">Examples</th>
            <th scope="col">Status</th>
            <th scope="col" class="num">Precision</th>
            <th scope="col" class="num">Recall</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as row (row.tag)}
            <tr>
              <td class="tag">#{row.tag}</td>
              <td
                class="num"
                title="{row.positives} with the tag, {row.negatives} without{row.rejected
                  ? ` (${row.rejected} rejected)`
                  : ''}"
              >
                {row.positives} / {row.negatives}
              </td>
              <td class="status" class:muted={row.status !== "trained"}>{status(row)}</td>
              <td class="num">{percent(row.precision)}</td>
              <td class="num">{percent(row.recall)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .training {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
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

  .scroll {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th {
    font-weight: 600;
    text-align: left;
    color: var(--muted);
  }

  th,
  td {
    padding: 4px 8px 4px 0;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .tag {
    min-width: 7em;
    font-weight: 600;
    white-space: normal;
    overflow-wrap: anywhere;
  }

  .status {
    white-space: normal;
  }
</style>
