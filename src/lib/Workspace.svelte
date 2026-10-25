<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    api,
    CLOSE_EVENT,
    errorMessage,
    type MetadataEdit,
    type MetadataEditReport,
    type NoteMeta,
    type NoteSummary,
    type OpenedNote,
    type VaultInfo,
  } from "$lib/api";
  import { confirmFolderTrash, confirmTrash } from "$lib/confirmTrash";
  import { findNote, rebase } from "$lib/explorer.svelte";
  import Icon from "$lib/Icon.svelte";
  import Import from "$lib/Import.svelte";
  import MetadataPanel from "$lib/MetadataPanel.svelte";
  import NoteEditor from "$lib/NoteEditor.svelte";
  import { EDITOR_MIN, fitWidths, LEFT, loadWidth, RAIL, RIGHT, saveWidth, type Column } from "$lib/layout";
  import RelatedNotes from "$lib/RelatedNotes.svelte";
  import ResizeHandle from "$lib/ResizeHandle.svelte";
  import Settings from "$lib/Settings.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import Trash from "$lib/Trash.svelte";

  let { vault }: { vault: VaultInfo } = $props();

  // Auto-lock is enforced by the backend; activity is reported at most this often.
  const TOUCH_INTERVAL_MS = 15_000;
  // How often the "Locks in" countdown is redrawn.
  const COUNTDOWN_TICK_MS = 15_000;

  let view = $state<"note" | "settings" | "import" | "trash">("note");
  // The import view stays mounted once opened, so a running import keeps its progress.
  let importOpened = $state(false);
  let showLeft = $state(true);
  let showRight = $state(true);
  let showRelated = $state(true);
  let opened = $state<OpenedNote | null>(null);
  let summary = $state<NoteSummary | null>(null);
  let meta = $state<NoteMeta | null>(null);
  let focusTitle = $state(false);
  // Kept for the unlocked session only: lock reloads the page, which resets it.
  let previewOnly = $state(false);
  let error = $state("");
  let editor = $state<ReturnType<typeof NoteEditor>>();
  let sidebar = $state<ReturnType<typeof Sidebar>>();
  let trashCount = $state(0);
  // When activity was last reported; the backend's idle timer restarts then. Unlocking counts.
  let lastTouch = $state(Date.now());
  let now = $state(Date.now());
  let autoLockMinutes = $state<number | null>(null);

  // Side column widths as stored, and as shown: a window too narrow for both and the
  // editor shrinks them for display without changing what is stored.
  let leftWidth = $state(loadWidth(LEFT));
  let rightWidth = $state(loadWidth(RIGHT));
  let windowWidth = $state(window.innerWidth);
  const leftShown = $derived(showLeft);
  const rightShown = $derived(view === "note" && showRight);
  const fitted = $derived(
    fitWidths(
      windowWidth,
      leftShown ? leftWidth : null,
      rightShown ? rightWidth : null,
      view === "note" && !showRight ? RAIL : 0,
    ),
  );
  const leftColumn = $derived(leftShown ? fitted.left : 0);
  const rightColumn = $derived(view !== "note" ? 0 : showRight ? fitted.right : RAIL);
  /** How wide a column may be dragged: its maximum, but never into the editor's minimum. */
  const maxWidth = (column: Column, other: number) =>
    Math.max(column.min, Math.min(column.max, windowWidth - EDITOR_MIN - other));

  function resetWidth(column: Column) {
    if (column === LEFT) leftWidth = LEFT.default;
    else rightWidth = RIGHT.default;
    saveWidth(column, column.default);
  }

  const locksIn = $derived(
    autoLockMinutes === null
      ? null
      : Math.max(1, Math.ceil((autoLockMinutes * 60_000 - (now - lastTouch)) / 60_000)),
  );

  onMount(() => {
    // The backend holds the window open briefly so pending edits can be saved.
    const unlisten = listen(CLOSE_EVENT, async () => {
      await editor?.flush();
      await api.exitApp();
    });
    const ticker = setInterval(() => (now = Date.now()), COUNTDOWN_TICK_MS);
    void loadAutoLock();
    void loadTrashCount();
    return () => {
      clearInterval(ticker);
      void unlisten.then((stop) => stop());
    };
  });

  async function loadAutoLock() {
    autoLockMinutes = await api.autoLockMinutes().catch(() => null);
  }

  async function loadTrashCount() {
    trashCount = await api.trashCount().catch(() => trashCount);
  }

  function activity() {
    const at = Date.now();
    if (at - lastTouch < TOUCH_INTERVAL_MS) return;
    lastTouch = now = at;
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
    // The auto-lock time and the recently opened count may have been changed there.
    const leavingSettings = view === "settings";
    if (leavingSettings) void loadAutoLock();
    if (view === next) await showNote();
    else view = next;
    // After showNote, whose reopen may have recorded the note.
    if (leavingSettings) void sidebar?.refreshRecent();
    if (next === "import") importOpened = true;
  }

  /**
   * Back to the note view. The editor remounts from `opened`, which autosaves don't
   * update, so the open note is read again first: otherwise the editor would show the
   * text from when it was opened, and its next autosave would write that back.
   */
  async function showNote() {
    if (opened) {
      try {
        const fresh = await api.reloadNote(opened.note.id);
        opened = fresh;
        summary = fresh.note;
        meta = fresh.meta;
      } catch (e) {
        opened = summary = meta = null;
        error = errorMessage(e);
      }
    }
    view = "note";
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
        show(await api.reloadNote(open), false);
        editorRevision++;
      } catch (e) {
        error = errorMessage(e);
      }
    }
    return true;
  }

  async function moveFolder(from: string, to: string): Promise<boolean> {
    // The open note is flushed and reopened if it is in the folder or below it.
    const inside = summary && rebase(summary.folder, from, from) !== null ? summary.id : null;
    const open = inside === null ? null : await flushIfIn([inside]);
    let moved: string;
    try {
      moved = await api.moveFolder(from, to);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    await sidebar?.folderMoved(from, moved);
    if (open !== null) {
      try {
        show(await api.reloadNote(open), false);
        editorRevision++;
      } catch (e) {
        error = errorMessage(e);
      }
    }
    return true;
  }

  async function editNotes(ids: number[], edit: MetadataEdit, sources: string[]): Promise<MetadataEditReport | null> {
    // Flush first, and reload after: the editor's next autosave would write the old
    // frontmatter back otherwise.
    const open = await flushIfIn(ids);
    let report: MetadataEditReport;
    try {
      report = await api.editNotesMetadata(ids, edit);
    } catch (e) {
      error = errorMessage(e);
      return null;
    }
    error = "";
    if (report.changed.length === 0) return report;
    await sidebar?.refresh(sources);
    if (open !== null && report.changed.includes(open)) {
      try {
        show(await api.reloadNote(open), false);
        editorRevision++;
      } catch (e) {
        error = errorMessage(e);
      }
    }
    return report;
  }

  /** Adds a suggested tag to the open note. Like a bulk edit: flush first, reload after. */
  async function acceptSuggestion(tag: string) {
    const id = opened?.note.id;
    if (id === undefined) return;
    await editor?.flush();
    // Reloading would drop edits that couldn't be saved; the editor shows why.
    if (editor?.unsaved()) return;
    try {
      const fresh = await api.acceptTagSuggestion(id, tag);
      sidebar?.noteSaved(fresh.note);
      show(fresh, false);
      editorRevision++;
    } catch (e) {
      error = errorMessage(e);
    }
  }

  // Bumped to reload the open note's suggestions after a reject elsewhere.
  let suggestionRevision = $state(0);

  /** Accepts a tag on notes from the Suggestions view. Like a bulk edit: flush first, reload after. */
  async function acceptTag(ids: number[], tag: string, sources: string[]): Promise<MetadataEditReport | null> {
    const open = await flushIfIn(ids);
    // Reloading would drop edits that couldn't be saved; the editor shows why.
    if (open !== null && editor?.unsaved()) return null;
    let report: MetadataEditReport;
    try {
      report = await api.acceptTagSuggestions(tag, ids);
    } catch (e) {
      error = errorMessage(e);
      return null;
    }
    error = "";
    if (report.changed.length === 0) return report;
    await sidebar?.refresh(sources);
    if (open !== null && report.changed.includes(open)) {
      try {
        show(await api.reloadNote(open), false);
        editorRevision++;
      } catch (e) {
        error = errorMessage(e);
      }
    }
    return report;
  }

  async function rejectTag(ids: number[], tag: string): Promise<boolean> {
    try {
      await api.rejectTagSuggestions(tag, ids);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    if (opened && ids.includes(opened.note.id)) suggestionRevision++;
    return true;
  }

  /** Settings → Tags → Review suggestions: back to the note, with the Suggestions view for `tag`. */
  async function reviewTag(tag: string) {
    await showNote();
    showLeft = true;
    await tick();
    sidebar?.reviewTag(tag);
  }

  async function deleteNotes(ids: number[], sources: string[]): Promise<boolean> {
    const title = ids.length === 1 ? (summary?.id === ids[0] ? summary.title : findNote(ids[0])?.title) : undefined;
    try {
      if (!(await confirmTrash(ids.length, title))) return false;
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    const open = await flushIfIn(ids);
    try {
      await api.trashNotes(ids);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    if (open !== null) opened = summary = meta = null;
    void loadTrashCount();
    await sidebar?.refresh(sources);
    return true;
  }

  /** Moves a folder with everything in it to the trash, after asking with its counts. */
  async function deleteFolder(folder: string): Promise<boolean> {
    try {
      const contents = await api.folderContents(folder);
      if (contents.notes > 0 && !(await confirmFolderTrash(folder, contents))) return false;
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    // The open note is closed if it is in the folder or below it, as when it is trashed.
    const inside = summary && rebase(summary.folder, folder, folder) !== null ? summary.id : null;
    const open = inside === null ? null : await flushIfIn([inside]);
    try {
      await api.trashFolder(folder);
    } catch (e) {
      error = errorMessage(e);
      return false;
    }
    error = "";
    if (open !== null) opened = summary = meta = null;
    void loadTrashCount();
    return true;
  }

  async function restoredFolder() {
    await sidebar?.refresh();
  }

  async function deleted(id: number) {
    const changed = summary?.id === id ? [summary.folder] : [];
    if (opened?.note.id === id) opened = summary = meta = null;
    void loadTrashCount();
    await sidebar?.refresh(changed);
  }
</script>

<svelte:window
  bind:innerWidth={windowWidth}
  onkeydowncapture={onkeydown}
  onpointerdown={activity}
  onpointermove={activity}
  onwheel={activity}
/>

<div
  class="workspace"
  class:no-left={!showLeft}
  class:no-right={!showRight}
  class:no-details={view !== "note"}
  style:--left="{leftColumn}px"
  style:--right="{rightColumn}px"
>
  <header>
    <button
      class="ghost icon"
      onclick={() => (showLeft = !showLeft)}
      aria-pressed={showLeft}
      aria-label={showLeft ? "Hide note list" : "Show note list"}
      title={showLeft ? "Hide note list" : "Show note list"}
    >
      <Icon name="panelLeft" />
    </button>
    <div class="vault" title={vault.path}>
      <span class="mark" aria-hidden="true">h</span>
      <span class="names">
        <strong>{vault.name}</strong>
        <span class="path">{vault.path}</span>
      </span>
    </div>
    <div class="spacer"></div>
    <div class="group">
      <button class="primary" onclick={() => create(sidebar?.currentFolder() ?? "")}>
        <Icon name="plus" strokeWidth={2} />
        New note
        <span class="kbd">Ctrl N</span>
      </button>
      <button class="toggle" onclick={() => toggleView("import")} aria-pressed={view === "import"}>
        <Icon name="import" />
        Import
      </button>
      <button
        class="icon toggle"
        onclick={() => toggleView("settings")}
        aria-pressed={view === "settings"}
        aria-label="Settings"
        title="Settings"
      >
        <Icon name="sliders" />
      </button>
      <span class="divider" aria-hidden="true"></span>
      {#if locksIn !== null}
        <span class="countdown" title="Locks after {autoLockMinutes} idle minutes">
          <Icon name="clock" size={14} />
          Locks in {locksIn} min
        </span>
      {/if}
      <button onclick={lock}>
        <Icon name="lock" />
        Lock
        <span class="kbd">Ctrl L</span>
      </button>
    </div>
  </header>

  {#if showLeft}
    <div class="left">
      <Sidebar
        bind:this={sidebar}
        selected={opened ? summary : null}
        {trashCount}
        trashOpen={view === "trash"}
        ontrash={() => toggleView("trash")}
        onselect={select}
        onmove={moveNotes}
        onmovefolder={moveFolder}
        ondelete={deleteNotes}
        ondeletefolder={deleteFolder}
        onedit={editNotes}
        onaccepttag={acceptTag}
        onrejecttag={rejectTag}
      />
    </div>
    <ResizeHandle
      edge="right"
      label="Resize sidebar"
      width={leftColumn}
      min={LEFT.min}
      max={maxWidth(LEFT, rightColumn)}
      onresize={(w) => (leftWidth = w)}
      oncommit={(w) => saveWidth(LEFT, w)}
      onreset={() => resetWidth(LEFT)}
    />
  {/if}

  <main>
    {#if error}<p class="callout danger banner" role="alert"><Icon name="alert" />{error}</p>{/if}
    {#if importOpened}
      <div class="page" hidden={view !== "import"}><Import onimported={() => sidebar?.refresh("all")} /></div>
    {/if}
    {#if view === "settings"}
      <div class="page"><Settings onreviewtag={reviewTag} /></div>
    {:else if view === "trash"}
      <Trash onrestored={restored} onrestoredfolder={restoredFolder} oncount={(n) => (trashCount = n)} />
    {:else if view === "note"}
      {#if opened}
        {#key `${opened.note.id}:${editorRevision}`}
          <NoteEditor
            bind:this={editor}
            {opened}
            {focusTitle}
            bind:previewOnly
            detailsShown={showRight}
            ontoggledetails={() => (showRight = !showRight)}
            onsaved={saved}
            onrenamed={renamed}
            ondeleted={deleted}
          />
        {/key}
      {:else}
        <div class="empty">
          <p class="muted">Select a note, or create one.</p>
          <button class="primary" onclick={() => create(sidebar?.currentFolder() ?? "")}>
            <Icon name="plus" strokeWidth={2} />
            New note
          </button>
        </div>
      {/if}
    {/if}
  </main>

  {#if view === "note"}
    {#if showRight}
      <div class="right">
        {#if opened && summary && meta}
          <MetadataPanel
            {meta}
            {summary}
            onset={(key, values) => editor?.setField(key, values)}
            onaccept={acceptSuggestion}
            revision={suggestionRevision}
          />
          <RelatedNotes id={summary.id} bind:open={showRelated} onopen={select} />
        {/if}
      </div>
      <ResizeHandle
        edge="left"
        label="Resize details panel"
        width={rightColumn}
        min={RIGHT.min}
        max={maxWidth(RIGHT, leftColumn)}
        onresize={(w) => (rightWidth = w)}
        oncommit={(w) => saveWidth(RIGHT, w)}
        onreset={() => resetWidth(RIGHT)}
      />
    {:else}
      <div class="right rail">
        <button class="ghost icon" onclick={() => (showRight = true)} aria-label="Show details panel" title="Show details panel">
          <Icon name="panelRight" />
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  /* --left and --right are set from the script (stored widths, hidden columns, the rail). */
  .workspace {
    display: grid;
    grid-template-columns: var(--left) minmax(0, 1fr) var(--right);
    grid-template-rows: auto minmax(0, 1fr);
    grid-template-areas:
      "header header header"
      "left main right";
    height: 100vh;
  }

  header {
    grid-area: header;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
  }

  .vault {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .mark {
    flex: none;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-text);
    font-family: var(--serif);
    font-style: italic;
    font-size: 18px;
    line-height: 1;
  }

  .names {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }

  .names strong {
    font-weight: 600;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--muted);
  }

  .spacer {
    flex: 1 1 0;
  }

  .group {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .group .icon {
    color: var(--text-2);
  }

  .toggle[aria-pressed="true"] {
    border-color: var(--accent);
    color: var(--accent);
  }

  .group .icon.toggle[aria-pressed="true"] {
    color: var(--accent);
  }

  .divider {
    width: 1px;
    height: 20px;
    background: var(--border);
  }

  .countdown {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--muted);
  }

  .workspace > :global(.handle.right) {
    grid-area: left;
  }

  .workspace > :global(.handle.left) {
    grid-area: right;
  }

  .left {
    grid-area: left;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--panel);
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
    background: var(--panel);
  }

  .rail {
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 10px;
  }

  .banner {
    border-radius: 0;
  }

  .page {
    overflow-y: auto;
    padding: 32px 16px 48px;
  }

  .page[hidden] {
    display: none;
  }

  .empty {
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
  }

  .empty p {
    margin: 0;
  }
</style>
