<script lang="ts">
  import { api, errorMessage, type DiscoverGroup, type Discovery, type TagInfo } from "$lib/api";

  // Settings → Tags → Discover tags: groups notes by meaning so the user can see which
  // tags the archive needs, and turns a group into a tag. Results live only in this
  // dialog; closing it (or locking) drops them.
  let { tags, oncreated }: { tags: TagInfo[]; oncreated: () => void } = $props();

  const SAMPLE = 8;

  let dialog = $state<HTMLDialogElement>();
  let groups = $state<number | null>(null);
  let untaggedOnly = $state(false);
  let running = $state(false);
  // Counts runs, so only the latest one's result is shown.
  let run = 0;
  let result = $state<Discovery | null>(null);
  let error = $state("");

  // The group whose "Create tag" form is open, and that form.
  let creating = $state<DiscoverGroup | null>(null);
  let tagName = $state("");
  let description = $state("");
  let chosen = $state<Set<number>>(new Set());
  let saving = $state(false);
  let formError = $state("");
  // Tags created from groups in this run, by group.
  let created = $state(new Map<DiscoverGroup, string>());

  let existing = $derived(tags.find((t) => t.tag === tagName.trim().replace(/^#/, "")));

  function open() {
    dialog?.showModal();
    void group();
  }

  // Closing (button or Escape) cancels a running grouping and drops the results, so
  // nothing about the groups outlives the dialog.
  function closed() {
    if (running) void api.cancelDiscoverTags();
    run++;
    running = false;
    result = creating = null;
    created = new Map();
    error = "";
  }

  async function group(e?: SubmitEvent) {
    e?.preventDefault();
    const mine = ++run;
    running = true;
    error = "";
    creating = null;
    try {
      const found = await api.discoverTags(groups || null, untaggedOnly);
      // `null`: a newer run or closing the dialog cancelled this one. A run that
      // finished just before it was cancelled is dropped here.
      if (found && mine === run) {
        result = found;
        created = new Map();
        if (found.k) groups = found.k;
      }
    } catch (err) {
      if (mine === run) error = errorMessage(err);
    } finally {
      if (mine === run) running = false;
    }
  }

  function startCreate(g: DiscoverGroup) {
    creating = g;
    tagName = g.words[0] ?? "";
    description = g.words.length ? `Notes about ${g.words.slice(0, 5).join(", ")}` : "";
    chosen = new Set(g.notes.map((n) => n.id));
    formError = "";
  }

  function toggle(id: number, on: boolean) {
    const next = new Set(chosen);
    if (on) next.add(id);
    else next.delete(id);
    chosen = next;
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!creating) return;
    const g = creating;
    const tag = tagName.trim().replace(/^#/, "");
    if (!description.trim() && chosen.size === 0) {
      formError = "Describe the tag or choose notes to add it to.";
      return;
    }
    saving = true;
    formError = "";
    try {
      // Nearest first, like the list.
      const ids = g.notes.filter((n) => chosen.has(n.id)).map((n) => n.id);
      const report = await api.createTagFromGroup(tag, description, ids);
      const added = report.changed.length;
      const skipped = report.unreadable.length;
      let summary = added === 1 ? `#${tag} added to 1 note` : `#${tag} added to ${added} notes`;
      if (report.unchanged) summary += `, ${report.unchanged} already had it`;
      if (skipped) summary += `; ${skipped} skipped (frontmatter can't be read)`;
      created = new Map(created).set(g, summary);
      creating = null;
      oncreated();
    } catch (err) {
      formError = errorMessage(err);
    } finally {
      saving = false;
    }
  }

  const count = (n: number) => (n === 1 ? "1 note" : `${n.toLocaleString()} notes`);
</script>

<div class="entry">
  <span class="muted">Not sure which tags you need? Group your notes by meaning and make tags from the groups.</span>
  <button type="button" onclick={open}>Discover tags…</button>
</div>

<dialog bind:this={dialog} aria-labelledby="discover-title" onclose={closed}>
  <header>
    <h2 id="discover-title">Discover tags</h2>
    <button type="button" class="ghost" onclick={() => dialog?.close()}>Close</button>
  </header>

  <form class="controls" onsubmit={group}>
    <label>
      Groups
      <input type="number" min="1" max="200" step="1" bind:value={groups} placeholder="auto" />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={untaggedOnly} />
      Only notes without tags
    </label>
    <button type="submit" disabled={running}>{running ? "Grouping…" : "Group notes"}</button>
  </form>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="body" aria-busy={running}>
    {#if running && !result}
      <p class="muted">Grouping notes…</p>
    {:else if result && result.model === null}
      <p class="muted">Notes are still being indexed. Try again once indexing has finished.</p>
    {:else if result && result.notes === 0}
      <p class="muted">{untaggedOnly ? "Every indexed note has a tag." : "No indexed notes to group yet."}</p>
    {:else if result}
      <p class="muted summary">
        {count(result.notes)} in {result.groups.length} groups, largest first. Groups are recomputed each time and not
        saved.
      </p>
      <ol class="groups">
        {#each result.groups as g, i (g)}
          <li class="group">
            <div class="head">
              <h3>Group {i + 1}</h3>
              <span class="muted">{count(g.notes.length)}</span>
              {#if creating !== g}
                <button type="button" class="create" onclick={() => startCreate(g)} disabled={saving}>
                  Create tag…
                </button>
              {/if}
            </div>
            {#if g.words.length}
              <ul class="chips" aria-label="Distinctive words">
                {#each g.words as w (w)}<li class="chip outline">{w}</li>{/each}
              </ul>
            {/if}
            {#if created.has(g)}<p class="done" role="status">{created.get(g)}.</p>{/if}

            {#if creating === g}
              <form class="create-form" onsubmit={create}>
                <div class="row">
                  <label>
                    Tag
                    <input class="mono" bind:value={tagName} required aria-label="Tag name" />
                  </label>
                  <label>
                    Description
                    <input bind:value={description} aria-label="Tag description" />
                  </label>
                </div>
                {#if existing}
                  <p class="muted hint">
                    #{existing.tag} already exists ({count(existing.notes)}){#if existing.description && description.trim()};
                      its description will be replaced{/if}.
                  </p>
                {/if}
                <div class="pick">
                  <span class="muted">Add it to {chosen.size} of {count(g.notes.length)}, nearest the centre first:</span>
                  <button type="button" class="ghost" onclick={() => (chosen = new Set(g.notes.map((n) => n.id)))}>All</button>
                  <button type="button" class="ghost" onclick={() => (chosen = new Set())}>None</button>
                </div>
                <ul class="notes">
                  {#each g.notes as n (n.id)}
                    <li>
                      <label class="check">
                        <input
                          type="checkbox"
                          checked={chosen.has(n.id)}
                          onchange={(e) => toggle(n.id, e.currentTarget.checked)}
                        />
                        <span class="title">{n.title}</span>
                        {#if n.folder}<span class="muted folder">{n.folder}</span>{/if}
                      </label>
                    </li>
                  {/each}
                </ul>
                {#if formError}<p class="error" role="alert">{formError}</p>{/if}
                <div class="actions">
                  <button type="button" class="ghost" onclick={() => (creating = null)} disabled={saving}>Cancel</button>
                  <button type="submit" class="primary" disabled={saving}>
                    {chosen.size ? `Create tag and add to ${count(chosen.size)}` : "Save description"}
                  </button>
                </div>
              </form>
            {:else}
              <ul class="sample" aria-label="Notes nearest the centre">
                {#each g.notes.slice(0, SAMPLE) as n (n.id)}
                  <li>
                    <span class="title">{n.title}</span>
                    {#if n.folder}<span class="muted folder">{n.folder}</span>{/if}
                  </li>
                {/each}
                {#if g.notes.length > SAMPLE}<li class="muted">and {count(g.notes.length - SAMPLE)} more</li>{/if}
              </ul>
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  </div>
</dialog>

<style>
  .entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }

  .entry button {
    flex: none;
  }

  dialog {
    width: calc(100% - 32px);
    max-width: 760px;
    max-height: calc(100vh - 48px);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--raised);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog[open] {
    display: flex;
    flex-direction: column;
  }

  dialog::backdrop {
    background: var(--scrim);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 24px 0;
  }

  h2 {
    margin: 0;
    font-size: 17px;
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

  .controls {
    display: flex;
    align-items: flex-end;
    gap: 16px;
    padding: 16px 24px;
    border-bottom: 1px solid var(--border);
  }

  .controls label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }

  .controls input[type="number"] {
    width: 90px;
  }

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .controls label.check {
    min-height: 32px;
  }

  .controls button {
    margin-left: auto;
  }

  .error {
    padding: 12px 24px 0;
  }

  .body {
    overflow: auto;
    padding: 16px 24px 24px;
  }

  .summary {
    margin-bottom: 12px;
  }

  .groups {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .head .muted {
    font-size: 12px;
  }

  .create {
    margin-left: auto;
  }

  .sample,
  .notes {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }

  .notes {
    max-height: 280px;
    overflow: auto;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .title {
    overflow-wrap: anywhere;
  }

  .folder {
    font-size: 12px;
  }

  .sample li,
  .notes label {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .done {
    color: var(--text-2);
  }

  .create-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .create-form .row {
    display: grid;
    grid-template-columns: 160px minmax(0, 1fr);
    gap: 8px;
  }

  .create-form .row label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }

  .create-form input:not([type="checkbox"]) {
    min-height: 32px;
    border-radius: 8px;
  }

  .hint {
    font-size: 12px;
  }

  .pick {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }

  .pick span {
    margin-right: auto;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
