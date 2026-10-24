<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, CLOSE_EVENT, errorMessage, type NoteMeta, type NoteSummary, type OpenedNote, type VaultInfo } from "$lib/api";
  import Import from "$lib/Import.svelte";
  import MetadataPanel from "$lib/MetadataPanel.svelte";
  import NoteEditor from "$lib/NoteEditor.svelte";
  import Settings from "$lib/Settings.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import Trash from "$lib/Trash.svelte";

  let { vault }: { vault: VaultInfo } = $props();

  // Auto-lock is enforced by the backend; activity is reported at most this often.
  const TOUCH_INTERVAL_MS = 15_000;

  let view = $state<"note" | "settings" | "import" | "trash">("note");
  // The import view stays mounted once opened, so a running import keeps its progress.
  let importOpened = $state(false);
  let showLeft = $state(true);
  let showRight = $state(true);
  let opened = $state<OpenedNote | null>(null);
  let summary = $state<NoteSummary | null>(null);
  let meta = $state<NoteMeta | null>(null);
  let focusTitle = $state(false);
  // Kept for the unlocked session only: lock reloads the page, which resets it.
  let previewOnly = $state(false);
  let error = $state("");
  let editor = $state<ReturnType<typeof NoteEditor>>();
  let sidebar = $state<ReturnType<typeof Sidebar>>();
  let lastTouch = 0;

  onMount(() => {
    // The backend holds the window open briefly so pending edits can be saved.
    const unlisten = listen(CLOSE_EVENT, async () => {
      await editor?.flush();
      await api.exitApp();
    });
    return () => void unlisten.then((stop) => stop());
  });

  function activity() {
    const now = Date.now();
    if (now - lastTouch < TOUCH_INTERVAL_MS) return;
    lastTouch = now;
    void api.touch();
  }

  // Capture phase, so shortcuts win over the editor's own (CodeMirror binds Ctrl+L).
  function onkeydown(e: KeyboardEvent) {
    activity();
    if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
    const key = e.key.toLowerCase();
    if (key === "l") {
      e.preventDefault();
      e.stopPropagation();
      void lock();
    } else if (key === "n") {
      e.preventDefault();
      e.stopPropagation();
      void create(sidebar?.currentFolder() ?? "");
    } else if (key === "k") {
      e.preventDefault();
      e.stopPropagation();
      void focusSearch();
    }
  }

  async function focusSearch() {
    showLeft = true;
    await tick();
    sidebar?.focusSearch();
  }

  async function toggleView(next: "settings" | "import" | "trash") {
    // Leaving the note unmounts the editor, which would drop an unsaved edit.
    await editor?.flush();
    view = view === next ? "note" : next;
    if (next === "import") importOpened = true;
  }

  async function lock() {
    await editor?.flush();
    await api.lockVault();
  }

  function show(note: OpenedNote, isNew: boolean) {
    opened = note;
    summary = note.note;
    meta = note.meta;
    focusTitle = isNew;
    view = "note";
    error = "";
  }

  async function select(id: number) {
    if (opened?.note.id === id && view === "note") return;
    await editor?.flush();
    try {
      show(await api.openNote(id), false);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function create(folder: string) {
    await editor?.flush();
    try {
      const note = await api.createNote(folder);
      await sidebar?.refresh([note.note.folder]);
      show(note, true);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function saved(s: NoteSummary, m: NoteMeta) {
    if (summary?.id === s.id) {
      summary = s;
      meta = m;
    }
    sidebar?.noteSaved(s);
  }

  async function renamed(fresh: OpenedNote) {
    // A move changes both the old and the new folder.
    const changed = [fresh.note.folder];
    if (summary?.id === fresh.note.id) {
      changed.push(summary.folder);
      summary = fresh.note;
      meta = fresh.meta;
    }
    await sidebar?.refresh(changed);
  }

  async function restored(note: OpenedNote) {
    await sidebar?.refresh();
    show(note, false);
  }

  // Bumped to remount the editor on the same note, e.g. after a bulk move.
  let editorRevision = $state(0);

  /** Flushes the open note's pending autosave if it is in `ids`; returns its id. */
  async function flushIfIn(ids: number[]): Promise<number | null> {
    const id = opened?.note.id ?? null;
    if (id === null || !ids.includes(id)) return null;
    await editor?.flush();
    return id;
  }

  async function moveNotes(ids: number[], folder: string, sources: string[]): Promise<boolean> {
    const open = await flushIfIn(ids);
    try {
      await api.moveNotes(ids, folder);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    await sidebar?.refresh([...sources, folder]);
    // Reopen, so its folder field and any rewritten links are current.
    if (open !== null) {
      try {
        show(await api.openNote(open), false);
        editorRevision++;
      } catch (e) {
        error = errorMessage(e);
      }
    }
    return true;
  }

  async function deleteNotes(ids: number[], sources: string[]): Promise<boolean> {
    const open = await flushIfIn(ids);
    try {
      await api.trashNotes(ids);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    if (open !== null) opened = summary = meta = null;
    await sidebar?.refresh(sources);
    return true;
  }

  async function deleted(id: number) {
    const changed = summary?.id === id ? [summary.folder] : [];
    if (opened?.note.id === id) opened = summary = meta = null;
    await sidebar?.refresh(changed);
  }
</script>

<svelte:window onkeydowncapture={onkeydown} onpointerdown={activity} onpointermove={activity} onwheel={activity} />

<div class="workspace" class:no-left={!showLeft} class:no-right={!showRight}>
  <header>
    <div class="group">
      <button onclick={() => (showLeft = !showLeft)} aria-pressed={showLeft} title="Show or hide the note list">
        Notes
      </button>
      <strong title={vault.path}>{vault.name}</strong>
    </div>
    <div class="group">
      <button onclick={() => (showRight = !showRight)} aria-pressed={showRight} title="Show or hide note details">
        Details
      </button>
      <button onclick={() => toggleView("import")} aria-pressed={view === "import"}>Import</button>
      <button onclick={() => toggleView("trash")} aria-pressed={view === "trash"}>Trash</button>
      <button onclick={() => toggleView("settings")} aria-pressed={view === "settings"}>Settings</button>
      <button onclick={lock} title="Lock (Ctrl+L)">Lock</button>
    </div>
  </header>

  {#if showLeft}
    <div class="left">
      <Sidebar
        bind:this={sidebar}
        selected={opened ? summary : null}
        onselect={select}
        oncreate={create}
        onmove={moveNotes}
        ondelete={deleteNotes}
      />
    </div>
  {/if}

  <main>
    {#if error}<p class="error banner" role="alert">{error}</p>{/if}
    {#if importOpened}
      <div class="page" hidden={view !== "import"}><Import onimported={() => sidebar?.refresh("all")} /></div>
    {/if}
    {#if view === "settings"}
      <div class="page"><Settings /></div>
    {:else if view === "trash"}
      <div class="page"><Trash onrestored={restored} /></div>
    {:else if view === "note"}
      {#if opened}
        {#key `${opened.note.id}:${editorRevision}`}
          <NoteEditor
            bind:this={editor}
            {opened}
            {focusTitle}
            bind:previewOnly
            onsaved={saved}
            onrenamed={renamed}
            ondeleted={deleted}
          />
        {/key}
      {:else}
        <div class="empty">
          <p class="muted">Select a note, or create one.</p>
          <button class="primary" onclick={() => create(sidebar?.currentFolder() ?? "")}>New note</button>
        </div>
      {/if}
    {/if}
  </main>

  {#if showRight}
    <div class="right">
      {#if opened && summary && meta && view === "note"}
        <MetadataPanel {meta} {summary} onset={(key, values) => editor?.setField(key, values)} />
      {/if}
    </div>
  {/if}
</div>

<style>
  .workspace {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr) 280px;
    grid-template-rows: auto minmax(0, 1fr);
    grid-template-areas:
      "header header header"
      "left main right";
    height: 100vh;
  }

  .workspace.no-left {
    grid-template-columns: 0 minmax(0, 1fr) 280px;
  }

  .workspace.no-right {
    grid-template-columns: 260px minmax(0, 1fr) 0;
  }

  .workspace.no-left.no-right {
    grid-template-columns: 0 minmax(0, 1fr) 0;
  }

  header {
    grid-area: header;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.4rem 0.75rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .group {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .left {
    grid-area: left;
    min-height: 0;
    border-right: 1px solid var(--border);
  }

  main {
    grid-area: main;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
  }

  main > :global(*:last-child) {
    flex: 1;
    min-height: 0;
  }

  .right {
    grid-area: right;
    min-height: 0;
    overflow-y: auto;
    border-left: 1px solid var(--border);
  }

  .banner {
    padding: 0.5rem 1.25rem;
  }

  .page {
    overflow-y: auto;
    padding: 1.5rem 16px;
  }

  .page[hidden] {
    display: none;
  }

  .empty {
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 0.5rem;
  }
</style>
