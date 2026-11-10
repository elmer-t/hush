<script lang="ts">
  import { api, errorMessage, type DiscoverGroup, type DiscoverScope, type Discovery, type TagInfo } from "$lib/api";
  import { currentSearch } from "$lib/currentSearch.svelte";
  import ModelNotice from "$lib/ModelNotice.svelte";
  import { reducedText } from "$lib/modelStatus.svelte";

  // Settings → Tags → Discover tags: groups notes by meaning so the user can see which
  // tags the archive needs, and turns a group into a tag. Results live only in this
  // dialog; closing it (or locking) drops them.
  let { tags, oncreated }: { tags: TagInfo[]; oncreated: () => void } = $props();

  const SAMPLE = 8;
  // Folder suggestions shown while typing a folder.
  const FOLDER_SUGGESTIONS = 50;

  let dialog = $state<HTMLDialogElement>();
  let groups = $state<number | null>(null);
  let scopeKind = $state<DiscoverScope["kind"]>("all");
  let folder = $state("");
  let folders = $state<string[]>([]);
  // The scope of the shown result, for its empty message.
  let resultScope = $state<DiscoverScope["kind"]>("all");
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
  // Every note in the group, not only the listed ones.
  let wholeGroup = $state(false);
  let saving = $state(false);
  let formError = $state("");
  // Tags created from groups in this run, by group.
  let created = $state(new Map<DiscoverGroup, string>());

  let existing = $derived(tags.find((t) => t.tag === tagName.trim().replace(/^#/, "")));
  let folderSuggestions = $derived.by(() => {
    const typed = folder.trim().toLowerCase();
    return folders.filter((f) => f.toLowerCase().includes(typed)).slice(0, FOLDER_SUGGESTIONS);
  });
  // The search the sidebar shows now, described for the scope picker.
  let searchLabel = $derived.by(() => {
    const q = currentSearch.query;
    if (!q) return "";
    // Discover groups the words search's results, also while the sidebar searches by meaning.
    const words = q.mode === "meaning" ? " (words)" : "";
    return q.text.trim() ? `“${q.text.trim()}”${words}` : "the filtered list";
  });

  function open() {
    dialog?.showModal();
    if (scopeKind === "search" && !currentSearch.query) scopeKind = "all";
    api.folders().then((f) => (folders = f), () => {});
    void group();
  }

  function scope(): DiscoverScope | null {
    switch (scopeKind) {
      case "folder":
        return { kind: "folder", folder: folder.trim() };
      case "search":
        return currentSearch.query ? { kind: "search", query: { ...currentSearch.query } } : null;
      default:
        return { kind: scopeKind };
    }
  }

  // Closing (button or Escape) cancels a running grouping and drops the results, so
  // nothing about the groups outlives the dialog.
  function closed() {
    // Also drops the backend's copy of the groups' members.
    void api.cancelDiscoverTags();
    run++;
    running = false;
    result = creating = null;
    created = new Map();
    error = "";
  }

  async function group(e?: SubmitEvent) {
    e?.preventDefault();
    const chosenScope = scope();
    if (!chosenScope) {
      error = "There is no search to group; search in the sidebar first.";
      return;
    }
    const mine = ++run;
    running = true;
    error = "";
    creating = null;
    try {
      const found = await api.discoverTags(groups || null, chosenScope);
      // `null`: a newer run or closing the dialog cancelled this one. A run that
      // finished just before it was cancelled is dropped here.
      if (found && mine === run) {
        result = found;
        resultScope = chosenScope.kind;
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
    wholeGroup = g.size > g.notes.length;
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
    const all = wholeGroup && g.size > g.notes.length;
    if (!description.trim() && !all && chosen.size === 0) {
      formError = "Describe the tag or choose notes to add it to.";
      return;
    }
    saving = true;
    formError = "";
    try {
      // Nearest first, like the list.
      const report =
        all && result
          ? await api.createTagFromGroup(tag, description, [], { run: result.run, group: result.groups.indexOf(g) })
          : await api.createTagFromGroup(
              tag,
              description,
              g.notes.filter((n) => chosen.has(n.id)).map((n) => n.id),
            );
      const added = report.changed.length;
      const skipped = report.unreadable.length;
      let summary = `#${tag} added to ${count(added)}`;
      if (report.unchanged) summary += `, ${report.unchanged.toLocaleString()} already had it`;
      if (skipped) summary += `; ${skipped.toLocaleString()} skipped (frontmatter can't be read)`;
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
    <label>
      Notes
      <select bind:value={scopeKind}>
        <option value="all">All notes</option>
        <option value="untagged">Notes without tags</option>
        <option value="folder">A folder and its subfolders</option>
        <option value="search" disabled={!currentSearch.query}>
          {currentSearch.query ? `Search results for ${searchLabel}` : "Search results (search in the sidebar first)"}
        </option>
      </select>
    </label>
    {#if scopeKind === "folder"}
      <label>
        Folder
        <input list="discover-folders" bind:value={folder} placeholder="All folders" />
        <datalist id="discover-folders">
          {#each folderSuggestions as f (f)}<option value={f}></option>{/each}
        </datalist>
      </label>
    {/if}
    <button type="submit" disabled={running}>{running ? "Grouping…" : "Group notes"}</button>
  </form>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="body" aria-busy={running}>
    {#if running && !result}
      <p class="muted">Grouping notes…</p>
    {:else if result && result.model === null && reducedText()}
      <ModelNotice what="Discover tags" />
    {:else if result && result.model === null}
      <p class="muted">Notes are still being indexed. Try again once indexing has finished.</p>
    {:else if !result && !running && reducedText()}
      <ModelNotice what="Discover tags" />
    {:else if result && result.notes === 0}
      <p class="muted">
        {resultScope === "untagged"
          ? "Every indexed note has a tag."
          : resultScope === "all"
            ? "No indexed notes to group yet."
            : "No indexed notes here to group."}
      </p>
    {:else if result}
      <p class="muted summary">
        {count(result.notes)} in {result.groups.length} groups, largest first.
        {#if result.sampled < result.notes}
          The groups were formed from a sample of {count(result.sampled)}; every other note joined the nearest group.
        {/if}
        Groups are recomputed each time and not saved.
      </p>
      <ol class="groups">
        {#each result.groups as g, i (g)}
          <li class="group">
            <div class="head">
              <h3>Group {i + 1}</h3>
              <span class="muted">{count(g.size)}</span>
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
                {#if g.size > g.notes.length}
                  <label class="check whole">
                    <input type="checkbox" bind:checked={wholeGroup} />
                    Select all {count(g.size)} in this group
                  </label>
                {/if}
                <div class="pick">
                  {#if wholeGroup && g.size > g.notes.length}
                    <span class="muted">Adds it to every note in the group. The {g.notes.length} nearest the centre:</span>
                  {:else}
                    <span class="muted">
                      Add it to {chosen.size} of {g.size > g.notes.length
                        ? `the ${g.notes.length} nearest the centre`
                        : count(g.notes.length)}, nearest first:
                    </span>
                    <button type="button" class="ghost" onclick={() => (chosen = new Set(g.notes.map((n) => n.id)))}>All</button>
                    <button type="button" class="ghost" onclick={() => (chosen = new Set())}>None</button>
                  {/if}
                </div>
                <ul class="notes">
                  {#each g.notes as n (n.id)}
                    <li>
                      <label class="check">
                        <input
                          type="checkbox"
                          disabled={wholeGroup && g.size > g.notes.length}
                          checked={(wholeGroup && g.size > g.notes.length) || chosen.has(n.id)}
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
                    {wholeGroup && g.size > g.notes.length
                      ? `Create tag and add to ${count(g.size)}`
                      : chosen.size
                        ? `Create tag and add to ${count(chosen.size)}`
                        : "Save description"}
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
                {#if g.size > SAMPLE}<li class="muted">and {count(g.size - SAMPLE)} more</li>{/if}
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
    flex-wrap: wrap;
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

  label.whole {
    display: flex;
    font-size: 13px;
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
