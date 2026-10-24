<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { api, errorMessage, type Facets, type NoteSummary, type SearchHit, type SidebarView } from "$lib/api";
  import { clearLoaded, findNote, refreshFolders, updateNote } from "$lib/explorer.svelte";
  import { isToggle } from "$lib/selection";
  import ExplorerTree from "$lib/ExplorerTree.svelte";
  import { inFolder } from "$lib/markdown";
  import { activeFilterCount, emptyFilters, toQuery } from "$lib/search";
  import FolderTree from "$lib/FolderTree.svelte";
  import NoteList from "$lib/NoteList.svelte";
  import SearchFilters from "$lib/SearchFilters.svelte";

  type Props = {
    /** The open note. */
    selected: NoteSummary | null;
    onselect: (id: number) => void;
    oncreate: (folder: string) => void;
    /** Bulk move; `sources` are the notes' current folders. Resolves to whether it worked. */
    onmove: (ids: number[], folder: string, sources: string[]) => Promise<boolean>;
    /** Bulk delete (to the trash). Resolves to whether it worked. */
    ondelete: (ids: number[], sources: string[]) => Promise<boolean>;
  };
  let { selected, onselect, oncreate, onmove, ondelete }: Props = $props();
  const selectedId = $derived(selected?.id ?? null);

  const PAGE_SIZE = 500;
  const SEARCH_DEBOUNCE_MS = 150;

  // Tree until the vault's stored choice has loaded.
  let view = $state<SidebarView>("tree");
  let folders = $state<string[]>([]);
  // Folders made in the tree that have no notes yet; they exist only until the vault locks.
  let emptyFolders = $state<string[]>([]);
  let selectedFolder = $state<string | null>(null);
  let text = $state("");
  let filters = $state(emptyFilters());
  let showFilters = $state(false);
  let facets = $state<Facets>({ tags: [], topics: [], authors: [], attributeKeys: [] });
  let hits = $state<SearchHit[]>([]);
  let hasMore = $state(false);
  let error = $state("");
  // Bumped when the query changes, so the list starts again at the top.
  let queryVersion = $state(0);
  let searchInput = $state<HTMLInputElement>();
  let loading = false;
  let generation = 0;
  let explorer = $state<ReturnType<typeof ExplorerTree>>();

  // Notes picked for a bulk action, in the visible list only. Frontend state, never stored.
  const picked = new SvelteSet<number>();
  // The last note clicked, where a Shift+click range starts.
  let anchor: number | null = null;
  let moving = $state(false);
  let moveTarget = $state("");
  let busy = $state(false);

  const allFolders = $derived([...folders, ...emptyFolders]);
  const filterCount = $derived(activeFilterCount(filters));
  const searching = $derived(text.trim() !== "" || filterCount > 0);

  onMount(() => {
    void Promise.all([loadFolders(), loadFacets()]);
    api.sidebarView().then((v) => (view = v), () => {});
  });

  function setView(next: SidebarView) {
    if (next !== view) clearSelection();
    view = next;
    api.setSidebarView(next).catch((e) => (error = errorMessage(e)));
  }

  // A new search, or another folder in the Recent list, shows other notes. In the tree
  // the folder is only the target for new notes, so the selection stays.
  $effect(() => {
    JSON.stringify([text, filters, view === "recent" ? selectedFolder : null]);
    untrack(clearSelection);
  });

  // Search as you type: any change to the text, filters or folder reruns the query.
  let timer: ReturnType<typeof setTimeout> | undefined;
  let firstRun = true;
  $effect(() => {
    JSON.stringify([text, filters, selectedFolder]);
    clearTimeout(timer);
    timer = setTimeout(
      () => {
        queryVersion++;
        void loadNotes(true);
      },
      firstRun ? 0 : SEARCH_DEBOUNCE_MS,
    );
    firstRun = false;
    return () => clearTimeout(timer);
  });

  /** Reloads the folder list, filters and the flat list. The tree's notes are only
   * reloaded for `changedFolders` (both folders of a move), or all with `"all"`. */
  export async function refresh(changedFolders: string[] | "all" = []) {
    if (changedFolders === "all") clearLoaded();
    await Promise.all([
      loadFolders(),
      loadFacets(),
      loadNotes(true),
      changedFolders === "all" ? Promise.resolve() : refreshFolders(changedFolders).catch((e) => (error = errorMessage(e))),
    ]);
  }

  /** The folder new notes go into. */
  export function currentFolder(): string {
    return selectedFolder ?? "";
  }

  export function focusSearch() {
    searchInput?.focus();
    searchInput?.select();
  }

  /** Keeps the list in step with a just-saved note. */
  export function noteSaved(summary: NoteSummary) {
    updateNote(summary);
    if (searching) {
      // Keep the ranking stable while typing; the next search re-checks the match.
      hits = hits.map((h) => (h.note.id === summary.id ? { ...h, note: summary } : h));
      return;
    }
    // Unfiltered, the list is newest first: move the note to the top, or drop it if it left the folder.
    const others = hits.filter((h) => h.note.id !== summary.id);
    hits = inFolder(summary.folder, selectedFolder) ? [{ note: summary, snippet: null }, ...others] : others;
  }

  async function loadFolders() {
    try {
      folders = await api.folders();
      const known = new Set(folders.map((f) => f.toLowerCase()));
      emptyFolders = emptyFolders.filter((f) => !known.has(f.toLowerCase()));
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function loadFacets() {
    try {
      facets = await api.searchFacets();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function loadNotes(reset: boolean) {
    if (!reset && (loading || !hasMore)) return;
    const run = reset ? ++generation : generation;
    loading = true;
    try {
      const query = toQuery(text, selectedFolder, filters, reset ? 0 : hits.length, PAGE_SIZE);
      const page = await api.searchNotes(query);
      if (run !== generation) return;
      if (reset) {
        hits = page;
      } else {
        const seen = new Set(hits.map((h) => h.note.id));
        hits = [...hits, ...page.filter((h) => !seen.has(h.note.id))];
      }
      hasMore = page.length === PAGE_SIZE;
      error = "";
    } catch (e) {
      error = errorMessage(e);
    } finally {
      if (run === generation) loading = false;
    }
  }

  function toggleFilters() {
    showFilters = !showFilters;
    // Tags and attributes change as notes are edited.
    if (showFilters) void loadFacets();
  }

  function clearSelection() {
    picked.clear();
    anchor = null;
    moving = false;
  }

  /** The notes shown, in order, that a Shift+click range or Ctrl/Cmd+A can cover. */
  function visibleIds(): number[] {
    if (view === "tree" && !searching) return explorer?.visibleNoteIds() ?? [];
    return hits.map((h) => h.note.id);
  }

  /** Plain click opens; Ctrl/Cmd+click toggles; Shift+click picks a range from the last click. */
  function pick(id: number, e: MouseEvent | KeyboardEvent) {
    if (e.shiftKey && anchor !== null) {
      const ids = visibleIds();
      const from = ids.indexOf(anchor);
      const to = ids.indexOf(id);
      if (from >= 0 && to >= 0) {
        if (!isToggle(e)) picked.clear();
        for (const i of ids.slice(Math.min(from, to), Math.max(from, to) + 1)) picked.add(i);
        return;
      }
    }
    if (isToggle(e)) {
      if (picked.has(id)) picked.delete(id);
      else picked.add(id);
      anchor = id;
      return;
    }
    clearSelection();
    anchor = id;
    onselect(id);
  }

  function selectAll() {
    for (const id of visibleIds()) picked.add(id);
  }

  function pickedNotes(): NoteSummary[] {
    return [...picked].flatMap((id) => {
      const note = hits.find((h) => h.note.id === id)?.note ?? findNote(id);
      return note ? [note] : [];
    });
  }

  async function moveSelection(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    const notes = pickedNotes();
    const ok = await onmove([...picked], moveTarget, notes.map((n) => n.folder));
    busy = false;
    if (ok) {
      clearSelection();
      moveTarget = "";
    }
  }

  async function deleteSelection() {
    if (busy) return;
    busy = true;
    const notes = pickedNotes();
    const ok = await ondelete([...picked], notes.map((n) => n.folder));
    busy = false;
    if (ok) clearSelection();
  }

  function addFolder(folder: string) {
    if (!allFolders.some((f) => f.toLowerCase() === folder.toLowerCase())) emptyFolders = [...emptyFolders, folder];
  }
</script>

<div class="sidebar">
  <div class="top">
    <button class="primary" onclick={() => oncreate(currentFolder())} title="New note (Ctrl+N)">New note</button>
    <div class="search">
      <input
        type="search"
        placeholder="Search (Ctrl+K)"
        aria-label="Search notes"
        spellcheck="false"
        bind:this={searchInput}
        bind:value={text}
      />
      <button onclick={toggleFilters} aria-expanded={showFilters} aria-pressed={filterCount > 0} title="Filters">
        Filters{#if filterCount > 0}&nbsp;<span class="count">{filterCount}</span>{/if}
      </button>
    </div>
  </div>
  {#if showFilters}
    <div class="panel"><SearchFilters bind:filters {facets} /></div>
  {/if}
  <div class="views" role="group" aria-label="Note list view">
    <button aria-pressed={view === "tree"} onclick={() => setView("tree")}>Tree</button>
    <button aria-pressed={view === "recent"} onclick={() => setView("recent")}>Recent</button>
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  {#if picked.size > 0}
    <div class="actions" role="toolbar" aria-label="Selected notes">
      <span><strong>{picked.size}</strong> selected</span>
      <button onclick={() => (moving = !moving)} aria-expanded={moving} disabled={busy}>Move to…</button>
      <button class="danger" onclick={deleteSelection} disabled={busy} title="Move the selected notes to the trash">
        Delete
      </button>
      <button onclick={clearSelection} disabled={busy}>Clear</button>
    </div>
    {#if moving}
      <form class="move" onsubmit={moveSelection}>
        <input
          list="move-folders"
          placeholder="Folder (empty = top level)"
          aria-label="Move to folder"
          spellcheck="false"
          bind:value={moveTarget}
          onkeydown={(e) => e.key === "Escape" && (moving = false)}
          disabled={busy}
          {@attach (el) => el.focus()}
        />
        <datalist id="move-folders">
          {#each allFolders as f (f)}<option value={f}></option>{/each}
        </datalist>
        <button type="submit" class="primary" disabled={busy}>Move</button>
      </form>
    {/if}
  {/if}
  {#if view === "tree" && !searching}
    <div class="explorer">
      <ExplorerTree
        bind:this={explorer}
        folders={allFolders}
        {selectedFolder}
        selectedNote={selected}
        onselectfolder={(f) => (selectedFolder = f)}
        onopen={pick}
        {picked}
        onselectall={selectAll}
        onclearselection={clearSelection}
        oncreatefolder={addFolder}
        onerror={(e) => (error = errorMessage(e))}
      />
    </div>
  {:else}
    {#if view === "recent"}
      <div class="folders">
        <FolderTree
          folders={allFolders}
          selected={selectedFolder}
          onselect={(f) => (selectedFolder = f)}
          oncreate={addFolder}
        />
      </div>
    {/if}
    <div class="results">
      {#key queryVersion}
        <NoteList
          {hits}
          {selectedId}
          {picked}
          {hasMore}
          onselect={pick}
          onselectall={selectAll}
          onclearselection={clearSelection}
          onmore={() => void loadNotes(false)}
          emptyText={searching ? "No matching notes." : "No notes here yet."}
        />
      {/key}
    </div>
  {/if}
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .top {
    display: grid;
    gap: 0.5rem;
    padding: 0.75rem;
  }

  .search {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.3rem;
  }

  .search button {
    padding-inline: 0.6em;
  }

  .search button[aria-pressed="true"] {
    border-color: var(--accent);
  }

  .count {
    display: inline-block;
    min-width: 1.4em;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 0.8em;
    text-align: center;
  }

  .panel {
    flex: none;
    margin-bottom: 0.75rem;
    max-height: 45vh;
    overflow-y: auto;
    border-bottom: 1px solid var(--border);
  }

  .folders {
    flex: none;
    max-height: 30vh;
    overflow-y: auto;
    padding: 0 0.75rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  .error {
    padding: 0.5rem 0.75rem;
  }

  .actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0.75rem 0.5rem;
    font-size: 0.85em;
  }

  .actions span {
    flex: 1;
  }

  .actions button {
    padding: 0.1em 0.5em;
  }

  .danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .move {
    flex: none;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.3rem;
    margin: 0 0.75rem 0.5rem;
  }

  .views {
    flex: none;
    display: grid;
    grid-template-columns: 1fr 1fr;
    margin: 0 0.75rem 0.5rem;
  }

  .views button {
    padding-block: 0.15em;
    font-size: 0.85em;
  }

  .views button:first-child {
    border-radius: 6px 0 0 6px;
  }

  .views button:last-child {
    border-radius: 0 6px 6px 0;
    border-left: none;
  }

  .views button[aria-pressed="true"] {
    background: var(--selection);
    font-weight: 600;
  }

  .explorer {
    flex: 1;
    min-height: 0;
  }

  .results {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr);
  }
</style>
