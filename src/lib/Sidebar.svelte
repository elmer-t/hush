<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage, type Facets, type NoteSummary, type SearchHit, type SidebarView } from "$lib/api";
  import { clearLoaded, refreshFolders, updateNote } from "$lib/explorer.svelte";
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
  };
  let { selected, onselect, oncreate }: Props = $props();
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

  const allFolders = $derived([...folders, ...emptyFolders]);
  const filterCount = $derived(activeFilterCount(filters));
  const searching = $derived(text.trim() !== "" || filterCount > 0);

  onMount(() => {
    void Promise.all([loadFolders(), loadFacets()]);
    api.sidebarView().then((v) => (view = v), () => {});
  });

  function setView(next: SidebarView) {
    view = next;
    api.setSidebarView(next).catch((e) => (error = errorMessage(e)));
  }

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
  {#if view === "tree" && !searching}
    <div class="explorer">
      <ExplorerTree
        folders={allFolders}
        {selectedFolder}
        selectedNote={selected}
        onselectfolder={(f) => (selectedFolder = f)}
        onopen={(id) => onselect(id)}
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
          {hasMore}
          {onselect}
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
