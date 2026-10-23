<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage, type NoteSummary } from "$lib/api";
  import { inFolder } from "$lib/markdown";
  import FolderTree from "$lib/FolderTree.svelte";
  import NoteList from "$lib/NoteList.svelte";

  type Props = {
    selectedId: number | null;
    onselect: (id: number) => void;
    oncreate: (folder: string) => void;
  };
  let { selectedId, onselect, oncreate }: Props = $props();

  const PAGE_SIZE = 500;

  let folders = $state<string[]>([]);
  // Folders made in the tree that have no notes yet; they exist only until the vault locks.
  let emptyFolders = $state<string[]>([]);
  let selectedFolder = $state<string | null>(null);
  let notes = $state<NoteSummary[]>([]);
  let hasMore = $state(false);
  let error = $state("");
  let loading = false;
  let generation = 0;

  const allFolders = $derived([...folders, ...emptyFolders]);

  onMount(() => void refresh());

  export async function refresh() {
    await Promise.all([loadFolders(), loadNotes(true)]);
  }

  /** The folder new notes go into. */
  export function currentFolder(): string {
    return selectedFolder ?? "";
  }

  /** Moves a just-saved note to the top, or drops it if it left the selected folder. */
  export function noteSaved(summary: NoteSummary) {
    const others = notes.filter((n) => n.id !== summary.id);
    notes = inFolder(summary.folder, selectedFolder) ? [summary, ...others] : others;
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

  async function loadNotes(reset: boolean) {
    if (!reset && (loading || !hasMore)) return;
    const run = reset ? ++generation : generation;
    loading = true;
    try {
      const page = await api.listNotes(selectedFolder, reset ? 0 : notes.length, PAGE_SIZE);
      if (run !== generation) return;
      if (reset) {
        notes = page;
      } else {
        const seen = new Set(notes.map((n) => n.id));
        notes = [...notes, ...page.filter((n) => !seen.has(n.id))];
      }
      hasMore = page.length === PAGE_SIZE;
      error = "";
    } catch (e) {
      error = errorMessage(e);
    } finally {
      if (run === generation) loading = false;
    }
  }

  function selectFolder(folder: string | null) {
    selectedFolder = folder;
    void loadNotes(true);
  }

  function addFolder(folder: string) {
    if (!allFolders.some((f) => f.toLowerCase() === folder.toLowerCase())) emptyFolders = [...emptyFolders, folder];
  }
</script>

<div class="sidebar">
  <div class="top">
    <button class="primary" onclick={() => oncreate(currentFolder())} title="New note (Ctrl+N)">New note</button>
  </div>
  <div class="folders">
    <FolderTree folders={allFolders} selected={selectedFolder} onselect={selectFolder} oncreate={addFolder} />
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  <NoteList {notes} {selectedId} {hasMore} {onselect} onmore={() => void loadNotes(false)} />
</div>

<style>
  .sidebar {
    display: grid;
    grid-template-rows: auto auto auto 1fr;
    height: 100%;
    min-height: 0;
  }

  .top {
    padding: 0.75rem;
  }

  .top button {
    width: 100%;
  }

  .folders {
    max-height: 40vh;
    overflow-y: auto;
    padding: 0 0.75rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  .error {
    padding: 0.5rem 0.75rem;
  }
</style>
