<script lang="ts">
  import { tick } from "svelte";
  import type { NoteSummary } from "$lib/api";
  import { ensureNote, expanded, expandTo, key, loaded, loadFolder, loadMore } from "$lib/explorer.svelte";
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
    onerror: (e: unknown) => void;
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
    onerror,
  }: Props = $props();

  /** The visible note rows in display order; notes in collapsed folders are left out. */
  export function visibleNoteIds(): number[] {
    return rows.flatMap((r) => (r.kind === "note" ? [r.note.id] : []));
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

  const isSelectedFolder = (path: string) => selectedFolder !== null && key(selectedFolder) === key(path);
</script>

<div class="explorer">
  <div class="heading">
    <button
      class="small target"
      onclick={() => onselectfolder(null)}
      disabled={selectedFolder === null}
      title="New notes and folders go to the top level"
    >
      {selectedFolder ? `In ${selectedFolder}` : "Top level"}
    </button>
    <button class="small" onclick={() => (naming = true)} title="New folder inside the selected folder">+ Folder</button>
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
    role="tree"
    tabindex="-1"
    aria-label="Folders and notes"
    aria-multiselectable="true"
    bind:this={tree}
    {onkeydown}
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
          role="treeitem"
          aria-level={row.level}
          aria-expanded={row.open}
          aria-selected={isSelectedFolder(row.path)}
          tabindex={row.key === activeKey ? 0 : -1}
          data-key={row.key}
          title={row.path}
          style:padding-left="{(row.level - 1) * 14}px"
          onclick={() => clickFolder(row)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty" aria-hidden="true">{row.open ? "▾" : "▸"}</span>
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
          style:padding-left="{(row.level - 1) * 14}px"
          onclick={(e) => activate(row, e)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty" aria-hidden="true"></span>
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
          style:padding-left="{(row.level - 1) * 14}px"
          onclick={(e) => activate(row, e)}
          onkeydown={() => {}}
          onfocus={() => (focusKey = row.key)}
        >
          <span class="twisty" aria-hidden="true"></span>
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
    gap: 0.25rem;
    height: 100%;
    min-height: 0;
    padding: 0 0.75rem 0.75rem;
  }

  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.8em;
    color: var(--muted);
  }

  .small {
    padding: 0.1em 0.5em;
    font-size: 0.95em;
  }

  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .empty {
    margin: 0;
    padding: 0.25rem;
  }

  .row {
    display: flex;
    align-items: center;
    padding-block: 0.15em;
    border-radius: 4px;
    cursor: default;
    user-select: none;
  }

  .row:hover {
    background: var(--active-line);
  }

  .row.selected {
    background: var(--selection);
  }

  .row.picked {
    background: var(--selection);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .twisty {
    width: 1.4em;
    flex: none;
    color: var(--muted);
    text-align: center;
  }

  .name {
    flex: 1;
    min-width: 0;
    padding: 0 0.25em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder .name {
    font-weight: 600;
  }
</style>
