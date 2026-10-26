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
    type DetailsSection,
    type OpenedNote,
    type VaultInfo,
  } from "$lib/api";
  import { confirmFolderTrash, confirmTrash } from "$lib/confirmTrash";
  import { findNote, rebase } from "$lib/explorer.svelte";
  import { model, watchModel } from "$lib/modelStatus.svelte";
  import Activity from "$lib/Activity.svelte";
  import Icon from "$lib/Icon.svelte";
  import GraphView from "$lib/GraphView.svelte";
  import Import from "$lib/Import.svelte";
  import LinkPanels from "$lib/LinkPanels.svelte";
  import MetadataPanel from "$lib/MetadataPanel.svelte";
  import NoteEditor from "$lib/NoteEditor.svelte";
  import NoteInternals from "$lib/NoteInternals.svelte";
  import {
    EDITOR_MIN,
    fitWidths,
    isBoolean,
    KEYS,
    LEFT,
    loadStored,
    loadWidth,
    oneOf,
    RIGHT,
    saveStored,
    saveWidth,
    type Column,
  } from "$lib/layout";
  import { flushGraphSettings, persistGraphSettings, restoreGraphScope } from "$lib/graph/settings.svelte";
  import RelatedNotes from "$lib/RelatedNotes.svelte";
  import ContextMenu, { type MenuItem } from "$lib/ContextMenu.svelte";
  import ResizeHandle from "$lib/ResizeHandle.svelte";
  import Settings from "$lib/Settings.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import Trash from "$lib/Trash.svelte";
  // Same shipped Tauri icon as the unlock screen.
  import appIcon from "../../src-tauri/icons/128x128@2x.png";

  let { vault }: { vault: VaultInfo } = $props();

  // Auto-lock is enforced by the backend; activity is reported at most this often.
  const TOUCH_INTERVAL_MS = 15_000;
  // How often the "Locks in" countdown is redrawn.
  const COUNTDOWN_TICK_MS = 15_000;

  // The layout is read from device storage here, so the first render already has it.
  // Settings, Import and Trash are never reopened: only the note or graph view is stored.
  let view = $state<"note" | "graph" | "settings" | "import" | "trash">(
    loadStored(KEYS.view, "note", oneOf("note", "graph")),
  );
  // The views with the details panel beside them.
  const details = $derived(view === "note" || view === "graph");
  // The import view stays mounted once opened, so a running import keeps its progress.
  let importOpened = $state(false);
  let showLeft = $state(loadStored(KEYS.showLeft, true, isBoolean));
  let showRight = $state(loadStored(KEYS.showRight, true, isBoolean));
  // Details panel sections the user collapsed; stored in the vault.
  let collapsed = $state<DetailsSection[]>([]);
  // Bumped when the open note was saved or other notes changed, so the links panels and
  // the related notes' reasons are read again.
  let linksRevision = $state(0);
  // The chooser a clicked ambiguous or unresolved link opens.
  let linkMenu = $state<{ x: number; y: number; label: string; items: MenuItem[] } | null>(null);
  let showInternals = $state(loadStored(KEYS.showInternals, false, isBoolean));
  // The Activity panel, toggled from the sidebar footer and with Ctrl+J.
  let showActivity = $state(loadStored(KEYS.showActivity, false, isBoolean));
  let opened = $state<OpenedNote | null>(null);
  let summary = $state<NoteSummary | null>(null);
  let meta = $state<NoteMeta | null>(null);
  let focusTitle = $state(false);
  // The editor mode, shared across notes.
  let previewOnly = $state(loadStored(KEYS.previewOnly, false, isBoolean));
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
  const rightShown = $derived(details && showRight);
  const fitted = $derived(
    fitWidths(windowWidth, leftShown ? leftWidth : null, rightShown ? rightWidth : null),
  );
  const leftColumn = $derived(leftShown ? fitted.left : 0);
  const rightColumn = $derived(rightShown ? fitted.right : 0);
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

  // Each layout change is stored when it happens, so auto-lock or a crash loses nothing.
  $effect(() => saveStored(KEYS.previewOnly, previewOnly));
  $effect(() => saveStored(KEYS.showLeft, showLeft));
  $effect(() => saveStored(KEYS.showRight, showRight));
  $effect(() => saveStored(KEYS.showInternals, showInternals));
  $effect(() => saveStored(KEYS.showActivity, showActivity));
  $effect(() => {
    if (view === "note" || view === "graph") saveStored(KEYS.view, view);
  });
  persistGraphSettings();

  onMount(() => {
    void restoreLastOpened();
    void restoreGraphScope();
    // The backend holds the window open briefly so pending edits can be saved.
    const unlisten = listen(CLOSE_EVENT, async () => {
      await editor?.flush();
      await api.exitApp();
    });
    const ticker = setInterval(() => (now = Date.now()), COUNTDOWN_TICK_MS);
    watchModel();
    model.open = () => void openModelSettings();
    void loadAutoLock();
    void loadTrashCount();
    void api
      .collapsedSections()
      .then((sections) => (collapsed = sections))
      .catch(() => {});
    return () => {
      clearInterval(ticker);
      void unlisten.then((stop) => stop());
    };
  });

  /** Reopens the note that was open when the app was last left, unless one was opened since. */
  async function restoreLastOpened() {
    const note = await api.lastOpenedNote().catch(() => null);
    if (note && !opened) show(note, false, true);
  }

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
    } else if (key === "j") {
      e.preventDefault();
      e.stopPropagation();
      showActivity = !showActivity;
    }
  }

  async function focusSearch() {
    showLeft = true;
    await tick();
    sidebar?.focusSearch();
  }

  async function toggleView(next: "graph" | "settings" | "import" | "trash") {
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

  /** Settings, scrolled to the Model section (from a "why is this off" link). */
  async function openModelSettings() {
    if (view !== "settings") await toggleView("settings");
    await tick();
    document.getElementById("model-settings")?.scrollIntoView({ block: "start" });
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

  const isOpen = (section: DetailsSection) => !collapsed.includes(section);

  function toggleSection(section: DetailsSection, open: boolean) {
    collapsed = open ? collapsed.filter((s) => s !== section) : [...collapsed.filter((s) => s !== section), section];
    api.setSectionCollapsed(section, !open).catch((e) => (error = errorMessage(e)));
  }

  /**
   * Follows a `[[link]]`: opens the note it names; for an ambiguous link, a menu at the
   * click to pick one; for an unresolved one, a menu offering to create it.
   */
  async function followLink(target: string, x: number, y: number) {
    let link;
    try {
      link = await api.resolveLink(target);
    } catch (e) {
      error = errorMessage(e);
      return;
    }
    if (link.resolution.kind === "unique") {
      await select(link.resolution.ids);
    } else if (link.resolution.kind === "ambiguous") {
      linkMenu = {
        x,
        y,
        label: `Notes matching ${target}`,
        items: link.notes.map((n) => ({
          label: n.folder ? `${n.folder}/${n.title}` : n.title,
          icon: "note",
          action: () => void select(n.id),
        })),
      };
    } else {
      linkMenu = {
        x,
        y,
        label: `No note ${target}`,
        items: [{ label: `Create “${link.target}”`, icon: "plus", action: () => void createLinked(link.target) }],
      };
    }
  }

  async function createLinked(target: string) {
    await editor?.flush();
    try {
      const note = await api.createLinkedNote(target);
      await sidebar?.refresh([note.note.folder]);
      show(note, false);
      linksRevision++;
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function lock() {
    await editor?.flush();
    await flushGraphSettings();
    await api.lockVault();
  }

  /** Makes `note` the open note; `stay` keeps the graph view instead of showing the editor. */
  function show(note: OpenedNote, isNew: boolean, stay = false) {
    opened = note;
    summary = note.note;
    meta = note.meta;
    focusTitle = isNew;
    if (!stay) view = "note";
    error = "";
  }

  /** A note clicked in the graph: it becomes the open note and the graph follows it. */
  async function openFromGraph(id: number) {
    if (opened?.note.id === id) return;
    try {
      show(await api.openNote(id), false, true);
    } catch (e) {
      error = errorMessage(e);
    }
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
      linksRevision++;
    }
    sidebar?.noteSaved(s);
  }

  async function renamed(fresh: OpenedNote) {
    // A move changes both the old and the new folder.
    const changed = [fresh.note.folder];
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
    // Links may have been rewritten, or their notes trashed.
    linksRevision++;
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
  class:no-details={!details}
  class:with-activity={showActivity}
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
      <img class="mark" src={appIcon} alt="" width="28" height="28" />
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
      <button class="toggle" onclick={() => toggleView("graph")} aria-pressed={view === "graph"}>
        <Icon name="graph" />
        Graph
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
    <!-- Disabled, not removed, where there is no details panel, so the header doesn't shift. -->
    <button
      class="ghost icon"
      onclick={() => (showRight = !showRight)}
      disabled={!details}
      aria-pressed={showRight}
      aria-label={showRight ? "Hide details panel" : "Show details panel"}
      title={showRight ? "Hide details panel" : "Show details panel"}
    >
      <Icon name="panelRight" />
    </button>
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
        activityOpen={showActivity}
        onactivity={() => (showActivity = !showActivity)}
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
    {:else if view === "graph"}
      <GraphView
        centre={summary ? { id: summary.id, folder: summary.folder } : null}
        onopen={openFromGraph}
        onedit={select}
      />
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
            onsaved={saved}
            onrenamed={renamed}
            ondeleted={deleted}
            onwikilink={followLink}
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

  {#if details}
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
          <LinkPanels
            id={summary.id}
            revision={linksRevision}
            {isOpen}
            ontoggle={toggleSection}
            onopen={view === "graph" ? openFromGraph : select}
            onfollow={followLink}
          />
          <RelatedNotes
            id={summary.id}
            revision={linksRevision}
            open={isOpen("related")}
            ontoggle={(open) => toggleSection("related", open)}
            onopen={view === "graph" ? openFromGraph : select}
          />
          <NoteInternals id={summary.id} updatedAt={summary.updatedAt} bind:open={showInternals} />
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
    {/if}
  {/if}

  {#if showActivity}
    <div class="activity-panel">
      <Activity onclose={() => (showActivity = false)} />
    </div>
  {/if}
</div>

{#if linkMenu}
  <ContextMenu {...linkMenu} onclose={() => (linkMenu = null)} />
{/if}

<style>
  /* --left and --right are set from the script (stored widths, hidden columns). */
  .workspace {
    display: grid;
    grid-template-columns: var(--left) minmax(0, 1fr) var(--right);
    grid-template-rows: auto minmax(0, 1fr);
    grid-template-areas:
      "header header header"
      "left main right";
    height: 100vh;
  }

  .workspace.with-activity {
    grid-template-rows: auto minmax(0, 1fr) min(300px, 40vh);
    grid-template-areas:
      "header header header"
      "left main right"
      "activity activity activity";
  }

  .activity-panel {
    grid-area: activity;
    min-height: 0;
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
    display: block;
    width: 28px;
    height: 28px;
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
