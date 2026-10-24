<script lang="ts">
  import { tick } from "svelte";
  import type { NoteSummary } from "$lib/api";
  import { ensureNote, expanded, expandTo, findNote, key, loaded, loadFolder, loadMore } from "$lib/explorer.svelte";
  import Icon from "$lib/Icon.svelte";
  import { normalizeFolder } from "$lib/markdown";
  import { selectionKeys } from "$lib/selection";

  type Props = {
    folders: string[];
    /** The folder New note and + Folder use; `null` is the top level. */
    selectedFolder: string | null;
    /** The open note, highlighted and revealed. */
    selectedNote: NoteSummary | null;
    onselectfolder: (folder: string | null) => void;
    /** Plain click or Enter on a note. The event is passed on so modifier clicks can be told apart. */
    onopen: (id: number, event: MouseEvent | KeyboardEvent) => void;
    /** Notes picked for a bulk action; folders can't be picked. */
    picked: ReadonlySet<number>;
    onselectall: () => void;
    onclearselection: () => void;
    oncreatefolder: (folder: string) => void;
    /** Right-click, Shift+F10 or the menu key on a note; `x`/`y` are where the menu opens. */
    onnotemenu: (id: number, x: number, y: number) => void;
    /** Delete on a focused note. */
    ondeletenote: (id: number) => void;
    onerror: (e: unknown) => void;
    /** Notes dropped onto a folder (`""` is the top level). */
    ondropnotes: (ids: number[], folder: string) => void;
    /** Folder `from` dropped onto folder `to` (`""` is the top level). */
    ondropfolder: (from: string, to: string) => void;
  };
  let {
    folders,
    selectedFolder,
    selectedNote,
    onselectfolder,
    onopen,
    picked,
    onselectall,
    onclearselection,
    oncreatefolder,
    onnotemenu,
    ondeletenote,
    onerror,
    ondropnotes,
    ondropfolder,
  }: Props = $props();

  /** The visible note rows in display order; notes in collapsed folders are left out. */
  export function visibleNoteIds(): number[] {
    return rows.flatMap((r) => (r.kind === "note" ? [r.note.id] : []));
  }

  /** Puts keyboard focus on a note's row, if it is shown. */
  export function focusNote(id: number) {
    if (rows.some((r) => r.key === `n:${id}`)) void moveFocus(`n:${id}`);
  }

  type Row =
    | { kind: "folder"; key: string; level: number; path: string; name: string; open: boolean; parent: string }
    | { kind: "note"; key: string; level: number; note: NoteSummary; parent: string }
    | { kind: "more"; key: string; level: number; folder: string; parent: string };

  // Folder path (lowercased) -> its direct subfolders, sorted by name.
  const children = $derived.by(() => {
    const map = new Map<string, { path: string; name: string }[]>();
    const seen = new Set<string>();
    for (const folder of folders) {
      const parts = folder.split("/").filter(Boolean);
      for (let i = 0; i < parts.length; i++) {
        const path = parts.slice(0, i + 1).join("/");
        if (seen.has(key(path))) continue;
        seen.add(key(path));
        const parent = key(parts.slice(0, i).join("/"));
        if (!map.has(parent)) map.set(parent, []);
        map.get(parent)!.push({ path, name: parts[i] });
      }
    }
    for (const list of map.values()) list.sort((a, b) => a.name.localeCompare(b.name));
    return map;
  });

  // Only rows inside expanded folders, in display order: subfolders first, then the
  // folder's own notes by title, then a Show more row when it has more.
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const visit = (folder: string, level: number) => {
      const parent = folder ? `f:${key(folder)}` : "";
      for (const child of children.get(key(folder)) ?? []) {
        const open = expanded.has(key(child.path));
        out.push({ kind: "folder", key: `f:${key(child.path)}`, level, path: child.path, name: child.name, open, parent });
        if (open) visit(child.path, level + 1);
      }
      const notes = loaded.get(key(folder));
      if (!notes) return;
      const sorted = notes.notes.slice().sort((a, b) => a.title.localeCompare(b.title));
      for (const note of sorted) out.push({ kind: "note", key: `n:${note.id}`, level, note, parent });
      if (notes.hasMore) out.push({ kind: "more", key: `m:${key(folder)}`, level, folder, parent });
    };
    visit("", 1);
    return out;
  });

  let tree = $state<HTMLElement>();
  // The row that takes Tab focus (roving tabindex).
  let focusKey = $state<string | null>(null);
  const activeKey = $derived(
    focusKey && rows.some((r) => r.key === focusKey)
      ? focusKey
      : selectedNote && rows.some((r) => r.key === `n:${selectedNote.id}`)
        ? `n:${selectedNote.id}`
        : (rows[0]?.key ?? null),
  );
  let naming = $state(false);
  let newName = $state("");

  // The root's own notes show at the top level, so they are always loaded.
  $effect(() => {
    if (!loaded.has("")) loadFolder("").catch(onerror);
  });

  // Reveal the open note however it was opened (and again after a move): expand its
  // folders, then scroll to it.
  let revealed = "";
  $effect(() => {
    const note = selectedNote;
    const target = note ? `${note.id}:${key(note.folder)}` : "";
    if (!note || target === revealed) return;
    revealed = target;
    void reveal(note);
  });

  async function reveal(note: NoteSummary) {
    try {
      await expandTo(note.folder);
    } catch (e) {
      onerror(e);
      return;
    }
    ensureNote(note);
    await tick();
    rowElement(`n:${note.id}`)?.scrollIntoView({ block: "nearest" });
  }

  function rowElement(k: string): HTMLElement | null {
    return tree?.querySelector<HTMLElement>(`[data-key="${CSS.escape(k)}"]`) ?? null;
  }

  async function toggle(path: string) {
    const k = key(path);
    if (expanded.has(k)) {
      expanded.delete(k);
      return;
    }
    expanded.add(k);
    if (!loaded.has(k)) {
      try {
        await loadFolder(path);
      } catch (e) {
        onerror(e);
      }
    }
  }

  function clickFolder(row: Extract<Row, { kind: "folder" }>) {
    focusKey = row.key;
    onselectfolder(row.path);
    void toggle(row.path);
  }

  function showMore(folder: string) {
    loadMore(folder).catch(onerror);
  }

  function activate(row: Row, e: MouseEvent | KeyboardEvent) {
    focusKey = row.key;
    if (row.kind === "folder") clickFolder(row);
    else if (row.kind === "note") onopen(row.note.id, e);
    else showMore(row.folder);
  }

  function openMenu(row: Extract<Row, { kind: "note" }>, e?: MouseEvent) {
    focusKey = row.key;
    if (e) {
      e.preventDefault();
      onnotemenu(row.note.id, e.clientX, e.clientY);
      return;
    }
    const rect = rowElement(row.key)?.getBoundingClientRect();
    onnotemenu(row.note.id, rect ? rect.left + 24 : 0, rect ? rect.bottom : 0);
  }

  async function moveFocus(k: string | undefined) {
    if (!k) return;
    focusKey = k;
    await tick();
    rowElement(k)?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    const i = rows.findIndex((r) => r.key === activeKey);
    if (i < 0) return;
    const row = rows[i];
    let handled = true;
    switch (e.key) {
      case "ArrowDown":
        void moveFocus(rows[i + 1]?.key);
        break;
      case "ArrowUp":
        void moveFocus(rows[i - 1]?.key);
        break;
      case "Home":
        void moveFocus(rows[0]?.key);
        break;
      case "End":
        void moveFocus(rows[rows.length - 1]?.key);
        break;
      case "ArrowRight":
        if (row.kind === "folder" && !row.open) void toggle(row.path);
        else if (row.kind === "folder" && rows[i + 1]?.level > row.level) void moveFocus(rows[i + 1].key);
        break;
      case "ArrowLeft":
        if (row.kind === "folder" && row.open) void toggle(row.path);
        else if (row.parent) void moveFocus(row.parent);
        break;
      case "Enter":
        activate(row, e);
        break;
      case "ContextMenu":
        if (row.kind === "note") openMenu(row);
        break;
      case "F10":
        if (e.shiftKey && row.kind === "note") openMenu(row);
        else handled = false;
        break;
      case "Delete":
        if (row.kind === "note") ondeletenote(row.note.id);
        break;
      default:
        handled = false;
    }
    if (handled) e.preventDefault();
  }

  function create(e: SubmitEvent) {
    e.preventDefault();
    const name = normalizeFolder(newName);
    naming = false;
    newName = "";
    if (!name) return;
    const path = selectedFolder ? `${selectedFolder}/${name}` : name;
    if (selectedFolder) expanded.add(key(selectedFolder));
    oncreatefolder(path);
    onselectfolder(path);
  }

  function focusOnMount(el: HTMLInputElement) {
    el.focus();
  }

  // Drag and drop moves notes, or one folder with everything in it. What is dragged is
  // kept here, since a DataTransfer can't be read during dragover.
  const DRAG_TYPE = "application/x-hush-notes";
  const FOLDER_DRAG_TYPE = "application/x-hush-folder";
  const HOVER_EXPAND_MS = 700;
  let dragged: { kind: "notes"; ids: number[]; folders: string[] } | { kind: "folder"; path: string } | null = null;
  // The folder a drop would go to, by key ("" is the top level); null when none.
  let dropKey = $state<string | null>(null);
  let hoverKey: string | null = null;
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  function dragStart(note: NoteSummary, e: DragEvent) {
    // A picked note drags the whole selection, any other note just itself.
    const ids = picked.has(note.id) ? [...picked] : [note.id];
    dragged = { kind: "notes", ids, folders: ids.map((id) => (id === note.id ? note : findNote(id))?.folder ?? "") };
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData(DRAG_TYPE, ids.join(","));
  }

  function dragFolderStart(path: string, e: DragEvent) {
    dragged = { kind: "folder", path };
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData(FOLDER_DRAG_TYPE, path);
  }

  /** Whether dropping what is dragged onto `folder` would move anything. A folder can't
   * go into itself, one of its subfolders or the parent it is already in. */
  function canDrop(folder: string): boolean {
    if (!dragged) return false;
    if (dragged.kind === "notes") return dragged.folders.some((f) => key(f) !== key(folder));
    const from = key(dragged.path);
    const to = key(folder);
    return to !== from && !to.startsWith(`${from}/`) && to !== from.split("/").slice(0, -1).join("/");
  }

  function dragEnd() {
    dragged = null;
    dropKey = null;
    hoverFolder(null);
  }

  /** The folder a drop on `row` goes to: a folder row's own, else the folder the row is in. */
  function dropFolder(row: Row | undefined): string {
    if (!row) return "";
    if (row.kind === "folder") return row.path;
    if (row.kind === "note") return row.note.folder;
    return row.folder;
  }

  function rowAt(e: DragEvent): Row | undefined {
    const el = (e.target as Element | null)?.closest<HTMLElement>("[data-key]");
    return el ? rows.find((r) => r.key === el.dataset.key) : undefined;
  }

  // Expands a collapsed folder once it has been hovered for a moment during a drag.
  function hoverFolder(path: string | null) {
    const k = path === null ? null : key(path);
    if (k === hoverKey) return;
    clearTimeout(hoverTimer);
    hoverKey = k;
    if (path === null || k === null || expanded.has(k)) return;
    hoverTimer = setTimeout(() => {
      if (hoverKey === k && !expanded.has(k)) void toggle(path);
    }, HOVER_EXPAND_MS);
  }

  function dragOver(e: DragEvent) {
    if (!dragged) return;
    const row = rowAt(e);
    const folder = dropFolder(row);
    hoverFolder(row?.kind === "folder" ? row.path : null);
    // Dropping where it would do nothing, or can't be done, isn't offered.
    if (!canDrop(folder)) {
      dropKey = null;
      return;
    }
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dropKey = key(folder);
  }

  function dragLeave(e: DragEvent) {
    if (tree?.contains(e.relatedTarget as Node | null)) return;
    dropKey = null;
    hoverFolder(null);
  }

  function drop(e: DragEvent) {
    if (!dragged) return;
    e.preventDefault();
    const folder = dropFolder(rowAt(e));
    const what = dragged;
    const ok = canDrop(folder);
    dragEnd();
    if (!ok) return;
    if (what.kind === "notes") ondropnotes(what.ids, folder);
    else ondropfolder(what.path, folder);
  }

  const indent = (level: number) => 8 + (level - 1) * 16;

  const isSelectedFolder = (path: string) => selectedFolder !== null && key(selectedFolder) === key(path);
</script>

<div class="explorer">
  <div class="heading">
    <span class="label">Folders</span>
    {#if selectedFolder}
      <button
        class="link target"
        onclick={() => onselectfolder(null)}
        title="New notes and folders go to {selectedFolder}. Click to use the top level."
      >
        in {selectedFolder}
      </button>
    {/if}
    <span class="spacer"></span>
    <button
      class="ghost small icon"
      onclick={() => (naming = true)}
      aria-label="New folder"
      title={selectedFolder ? `New folder in ${selectedFolder}` : "New folder"}
    >
      <Icon name="folderPlus" />
    </button>
  </div>

  {#if naming}
    <form onsubmit={create}>
      <input
        placeholder={selectedFolder ? `New folder in ${selectedFolder}` : "New folder"}
        aria-label="New folder name"
        bind:value={newName}
        onblur={() => (naming = false)}
        onkeydown={(e) => e.key === "Escape" && (naming = false)}
        {@attach focusOnMount}
      />
    </form>
  {/if}

  <div
    class="rows"
    class:drop-target={dropKey === ""}
    role="tree"
    tabindex="-1"
    aria-label="Folders and notes"
    aria-multiselectable="true"
    bind:this={tree}
    {onkeydown}
    ondragover={dragOver}
    ondragleave={dragLeave}
    ondrop={drop}
    {@attach selectionKeys(onselectall, onclearselection)}
  >
    {#if rows.length === 0}
      <p class="muted empty">No notes yet.</p>
    {/if}
    {#each rows as row (row.key)}
      {#if row.kind === "folder"}
        <div
          class="row folder"
          class:selected={isSelectedFolder(row.path)}
          class:drop-target={dropKey === key(row.path)}
          role="treeitem"
          aria-level={row.level}
          aria-expanded={row.open}
          aria-selected={isSelectedFolder(row.path)}
          tabindex={row.key === activeKey ? 0 : -1}
          data-key={row.key}
          title={row.path}
          style:padding-left="{indent(row.level)}px"
          draggable="true"
          ondragstart={(e) => dragFolderStart(row.path, e)}
          ondragend={dragEnd}
          onclick={() => clickFolder(row)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty"><Icon name={row.open ? "chevronDown" : "chevronRight"} size={14} strokeWidth={2} /></span>
          <span class="glyph"><Icon name="folder" /></span>
          <span class="name">{row.name}</span>
        </div>
      {:else if row.kind === "note"}
        <div
          class="row note"
          class:selected={row.note.id === selectedNote?.id}
          class:picked={picked.has(row.note.id)}
          role="treeitem"
          aria-level={row.level}
          aria-selected={picked.size > 0 ? picked.has(row.note.id) : row.note.id === selectedNote?.id}
          aria-current={row.note.id === selectedNote?.id}
          tabindex={row.key === activeKey ? 0 : -1}
          data-key={row.key}
          title={row.note.folder ? `${row.note.folder}/${row.note.title}` : row.note.title}
          style:padding-left="{indent(row.level)}px"
          draggable="true"
          ondragstart={(e) => dragStart(row.note, e)}
          ondragend={dragEnd}
          onclick={(e) => activate(row, e)}
          oncontextmenu={(e) => openMenu(row, e)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty"></span>
          <span class="glyph note-glyph"><Icon name="note" /></span>
          <span class="name">{row.note.title}</span>
        </div>
      {:else}
        <div
          class="row more"
          role="treeitem"
          aria-level={row.level}
          aria-selected="false"
          tabindex={row.key === activeKey ? 0 : -1}
          data-key={row.key}
          style:padding-left="{indent(row.level)}px"
          onclick={(e) => activate(row, e)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty"></span>
          <span class="name muted">Show more…</span>
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .explorer {
    display: flex;
    flex-direction: column;
    gap: 2px;
    height: 100%;
    min-height: 0;
    padding: 0 8px 8px;
  }

  .heading {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 0 4px 0 8px;
  }

  .label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 11.5px;
  }

  .spacer {
    flex: 1;
  }

  form {
    padding: 0 4px 4px;
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .empty {
    margin: 0;
    padding: 4px 8px;
  }

  .row {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding-right: 8px;
    border-radius: 6px;
    font-size: 13.5px;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.selected {
    background: var(--selection);
    color: var(--selection-text);
    font-weight: 500;
  }

  .row.picked {
    background: var(--selection);
    color: var(--selection-text);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .row.drop-target,
  .rows.drop-target {
    border-radius: 6px;
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .twisty {
    width: 14px;
    flex: none;
    display: flex;
    color: var(--muted);
  }

  .glyph {
    flex: none;
    display: flex;
    color: var(--muted);
  }

  .note-glyph {
    color: inherit;
    opacity: 0.7;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
