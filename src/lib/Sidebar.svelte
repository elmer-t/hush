<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import { indexText, indexing } from "$lib/indexing";
  import { meaningCoverage, meaningOff, resultCount, resultOrder, scopeLabel, searchSummary } from "$lib/search";
  import {
    api,
    errorMessage,
    INDEX_STATUS_EVENT,
    type Facets,
    type IndexStatus,
    type MetadataEdit,
    type MetadataEditReport,
    type NoteSummary,
    type ReadStatus,
    type SearchHit,
    type SearchMode,
    type SearchScope,
    type SidebarView,
  } from "$lib/api";
  import {
    clearLoaded,
    findNote,
    folderDeleted as treeFolderDeleted,
    folderMoved as treeFolderMoved,
    rebase,
    refreshFolders,
    updateNote,
  } from "$lib/explorer.svelte";
  import { isToggle } from "$lib/selection";
  import ContextMenu, { type MenuItem } from "$lib/ContextMenu.svelte";
  import ExplorerTree from "$lib/ExplorerTree.svelte";
  import { inFolder } from "$lib/markdown";
  import { sameFolders, subtreeSizes, type FolderSize } from "$lib/folderTree";
  import { markRead, READ_STATUSES } from "$lib/readStatus.svelte";
  import { activeFilterCount, emptyFilters, searchFolder, searchKey, searchPlaceholder, toQuery } from "$lib/search";
  import { searchScheduler } from "$lib/searchTiming";
  import { currentSearch } from "$lib/currentSearch.svelte";
  import { MODEL_FEATURES, model, reducedText } from "$lib/modelStatus.svelte";
  import FolderTree from "$lib/FolderTree.svelte";
  import Icon from "$lib/Icon.svelte";
  import NoteList from "$lib/NoteList.svelte";
  import RecentlyOpened from "$lib/RecentlyOpened.svelte";
  import SearchFilters from "$lib/SearchFilters.svelte";
  import Suggestions from "$lib/Suggestions.svelte";

  type Props = {
    /** The open note. */
    selected: NoteSummary | null;
    trashCount: number;
    /** Whether the trash is the page shown. */
    trashOpen: boolean;
    ontrash: () => void;
    onselect: (id: number) => void;
    /** Bulk move; `sources` are the notes' current folders. Resolves to whether it worked. */
    onmove: (ids: number[], folder: string, sources: string[]) => Promise<boolean>;
    /** Moves folder `from` into folder `to`. Resolves to whether it worked. */
    onmovefolder: (from: string, to: string) => Promise<boolean>;
    /** Bulk delete (to the trash). Resolves to whether it worked. */
    ondelete: (ids: number[], sources: string[]) => Promise<boolean>;
    /** Moves a folder that holds notes, with everything in it, to the trash. Resolves to whether it worked. */
    ondeletefolder: (folder: string) => Promise<boolean>;
    /** Bulk metadata edit. Resolves to the report, or null when it failed. */
    onedit: (ids: number[], edit: MetadataEdit, sources: string[]) => Promise<MetadataEditReport | null>;
    /** Accepts a tag on notes in the Suggestions view. Resolves to the report, or null when it failed. */
    onaccepttag: (ids: number[], tag: string, sources: string[]) => Promise<MetadataEditReport | null>;
    /** Rejects a tag on notes in the Suggestions view. Resolves to whether it worked. */
    onrejecttag: (ids: number[], tag: string) => Promise<boolean>;
    /** Whether the Activity panel is open. */
    activityOpen: boolean;
    /** Toggles the Activity panel (the footer status line and its button). */
    onactivity: () => void;
  };
  let {
    selected,
    trashCount,
    trashOpen,
    ontrash,
    onselect,
    onmove,
    onmovefolder,
    ondelete,
    ondeletefolder,
    onedit,
    onaccepttag,
    onrejecttag,
    activityOpen,
    onactivity,
  }: Props = $props();
  const selectedId = $derived(selected?.id ?? null);

  const PAGE_SIZE = 500;
  // From this many notes left, the footer says indexing stops while locked.
  const PAUSE_NOTICE_MIN = 10_000;

  /** `~5 h left`, `~12 min left`, `under a minute left`. */
  function timeLeft(secs: number): string {
    if (secs < 60) return "under a minute left";
    if (secs < 3600) return `~${Math.round(secs / 60)} min left`;
    if (secs < 48 * 3600) return `~${Math.round(secs / 3600)} h left`;
    return `~${Math.round(secs / 86400)} days left`;
  }

  // Tree until the vault's stored choice has loaded.
  let view = $state<SidebarView>("tree");
  let folders = $state<string[]>([]);
  // Each folder's own bytes, loaded with the folder list but kept apart from it: a save
  // changes these numbers and not the paths, so the trees aren't rebuilt for it.
  let folderSizes = $state<FolderSize[]>([]);
  let sizesGeneration = 0;
  // Folders made in the tree that have no notes yet; they exist only until the vault locks.
  let emptyFolders = $state<string[]>([]);
  // Stored in the vault (a folder path is note data), read once after the folders load.
  let selectedFolder = $state<string | null>(null);
  let folderRestored = false;
  // Whether a search is limited to `selectedFolder` or covers the whole vault. Separate
  // from the folder, which also says where new notes go. Stored in the vault.
  let scope = $state<SearchScope>("folder");
  // Words (full-text) or meaning (vectors and words fused, #146). Stored in the vault.
  let mode = $state<SearchMode>("words");
  let text = $state("");
  let filters = $state(emptyFilters());
  let showFilters = $state(false);
  let facets = $state<Facets>({ tags: [], topics: [], authors: [], attributeKeys: [] });
  let hits = $state<SearchHit[]>([]);
  // Too many matches to rank: the list shows the newest ones.
  let newestFirst = $state(false);
  // How the last meaning search came out: whether it used the vectors, where its list
  // was cut, and why it searched by words only.
  let meaningResult = $state<{ byMeaning: boolean; top: number | null; wordsOnly: string | null }>({
    byMeaning: false,
    top: null,
    wordsOnly: null,
  });
  // A meaning search is waiting for the model to load for its query.
  let loadingModel = $state(false);
  let hasMore = $state(false);
  let error = $state("");
  // Bumped when the query changes, so the list starts again at the top.
  let queryVersion = $state(0);
  let searchInput = $state<HTMLInputElement>();
  let loading = false;
  let generation = 0;
  let explorer = $state<ReturnType<typeof ExplorerTree>>();
  // The tag the Suggestions view reviews; kept until the vault locks.
  let reviewedTag = $state<string | null>(null);

  // Notes picked for a bulk action, in the visible list only. Frontend state, never stored.
  const picked = new SvelteSet<number>();
  // The last note clicked, where a Shift+click range starts.
  let anchor: number | null = null;
  let moving = $state(false);
  let moveTarget = $state("");
  let editing = $state(false);
  // Read status isn't a frontmatter edit: it is stored in the vault, so it goes its own way.
  type EditKind = MetadataEdit["kind"] | "setReadStatus";
  let editKind = $state<EditKind>("addTag");
  let editValue = $state("");
  // What the last bulk edit did.
  let editNotice = $state<{ text: string; titles: string[] } | null>(null);
  let busy = $state(false);
  // The tree's open context menu: the note or folder it was opened on and where.
  let menu = $state<{ id: number; x: number; y: number } | { folder: string; x: number; y: number } | null>(null);

  // The "Recently opened" section: empty when the list is off or nothing was opened yet.
  let recentOpened = $state<NoteSummary[]>([]);
  let recentCollapsed = $state(false);

  const allFolders = $derived([...folders, ...emptyFolders]);
  // Folder (lowercased, "" for the vault) -> bytes in it and below it.
  const sizes = $derived(subtreeSizes(folderSizes));
  const filterCount = $derived(activeFilterCount(filters));
  const searching = $derived(text.trim() !== "" || filterCount > 0);
  // The folder the list is limited to now: null while a whole-vault search runs.
  const queryFolder = $derived(searchFolder(selectedFolder, scope, searching));
  // Why Meaning can't be picked now; the stored choice is kept and applies again later.
  const meaningReason = $derived(meaningOff(model.status));
  const searchMode = $derived<SearchMode>(mode === "meaning" && meaningReason === null ? "meaning" : "words");
  const coverage = $derived(searchMode === "meaning" && text.trim() ? meaningCoverage(model.status, model.index) : null);

  // The background indexing job (embeddings): a progress line while it works.
  let indexStatus = $state<IndexStatus>({ state: "idle" });
  const progress = $derived(indexing(indexStatus));
  // Why features that need the model are off, if they are: the footer says so until
  // they work again (no pop-up to dismiss).
  const modelReason = $derived(reducedText());
  const firstIndex = $derived(model.status?.reduced?.reason === "firstIndex");

  onMount(() => {
    void Promise.all([loadFolders(), loadFacets()]).then(restoreFolder);
    api.sidebarView().then((v) => (view = v), () => {});
    api.sidebarSearchScope().then((s) => (scope = s), () => {});
    api.sidebarSearchMode().then((m) => (mode = m), () => {});
    api.recentOpenedCollapsed().then((c) => (recentCollapsed = c), () => {});
    const unlisten = listen<IndexStatus>(INDEX_STATUS_EVENT, (e) => (indexStatus = e.payload));
    api.indexStatus().then((s) => (indexStatus = s), () => {});
    return () => void unlisten.then((stop) => stop());
  });

  // Opening a note moves it to the front of the list, so reload it then.
  $effect(() => {
    void selectedId;
    void refreshRecent();
  });

  /** Reloads the "Recently opened" list (after an open, a trash or restore, or a settings change). */
  export async function refreshRecent() {
    try {
      recentOpened = await api.recentOpenedNotes();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function toggleRecent() {
    recentCollapsed = !recentCollapsed;
    api.setRecentOpenedCollapsed(recentCollapsed).catch((e) => (error = errorMessage(e)));
  }

  function openRecent(id: number) {
    clearSelection();
    anchor = id;
    onselect(id);
  }

  function setView(next: SidebarView) {
    if (next !== view) clearSelection();
    view = next;
    api.setSidebarView(next).catch((e) => (error = errorMessage(e)));
  }

  function setScope(next: SearchScope) {
    scope = next;
    api.setSidebarSearchScope(next).catch((e) => (error = errorMessage(e)));
  }

  function setMode(next: SearchMode) {
    mode = next;
    api.setSidebarSearchMode(next).catch((e) => (error = errorMessage(e)));
  }

  /** Enter searches without waiting; Alt+F switches between this folder and the whole
   *  vault; Alt+M between words and meaning. */
  function searchKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") search.now();
    else if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyF" && selectedFolder !== null) {
      e.preventDefault();
      setScope(scope === "folder" ? "vault" : "folder");
    } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyM" && (mode === "meaning" || meaningReason === null)) {
      e.preventDefault();
      setMode(mode === "meaning" ? "words" : "meaning");
    }
  }

  // A new search, another search scope, or another folder in the Recent list, shows other
  // notes. In the tree the folder is only the target for new notes, so the selection stays.
  $effect(() => {
    JSON.stringify([text, filters, scope, searchMode, view === "recent" ? selectedFolder : null]);
    untrack(clearSelection);
  });

  // Search as you type: any change to the text, filters, folder or scope reruns the query.
  // Typing waits for a pause; the rest searches at once (see `searchTiming`).
  const search = searchScheduler(() => {
    queryVersion++;
    void loadNotes(true);
  });
  $effect(() => {
    search.change({ text, scope: searchKey(filters, selectedFolder, scope, searchMode) });
    return () => search.cancel();
  });

  /** Reloads the folder list, filters and the flat list. The tree's notes are only
   * reloaded for `changedFolders` (both folders of a move), or all with `"all"`. */
  export async function refresh(changedFolders: string[] | "all" = []) {
    if (changedFolders === "all") clearLoaded();
    await Promise.all([
      loadFolders(),
      loadFacets(),
      loadNotes(true),
      refreshRecent(),
      changedFolders === "all" ? Promise.resolve() : refreshFolders(changedFolders).catch((e) => (error = errorMessage(e))),
    ]);
  }

  /** After folder `from` moved to `to`: tree-only folders below it and the selected
   * folder follow it, the tree's expanded state too, and the lists reload. */
  export async function folderMoved(from: string, to: string) {
    const follow = (f: string) => rebase(f, from, to) ?? f;
    emptyFolders = emptyFolders
      .map(follow)
      .filter((f, i, all) => all.findIndex((g) => g.toLowerCase() === f.toLowerCase()) === i);
    if (selectedFolder !== null) selectedFolder = follow(selectedFolder);
    await Promise.all([treeFolderMoved(from, to).catch((e) => (error = errorMessage(e))), refresh()]);
  }

  /** Moves a folder and everything in it to the trash. A folder that only exists in the
   * tree (no notes in it or below it) is just removed, without asking. */
  async function deleteFolder(path: string) {
    if (busy) return;
    const inside = (f: string) => rebase(f, path, path) !== null;
    if (folders.some(inside)) {
      busy = true;
      const ok = await ondeletefolder(path);
      busy = false;
      if (!ok) return;
    }
    emptyFolders = emptyFolders.filter((f) => !inside(f));
    if (selectedFolder !== null && inside(selectedFolder)) {
      const parent = path.split("/").slice(0, -1).join("/");
      selectedFolder = parent || null;
    }
    treeFolderDeleted(path);
    // Notes in the folder may have been picked; they are in the trash now.
    for (const id of [...picked]) {
      const note = notesOf([id])[0];
      if (note && inside(note.folder)) picked.delete(id);
    }
    if (picked.size === 0) clearSelection();
    await refresh();
  }

  /** Shows the Suggestions view for `tag` (Settings → Tags → Review suggestions). */
  export function reviewTag(tag: string) {
    text = "";
    filters = emptyFilters();
    reviewedTag = tag;
    setView("suggestions");
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
    void loadSizes();
    if (searching) {
      // Keep the ranking stable while typing; the next search re-checks the match.
      hits = hits.map((h) => (h.note.id === summary.id ? { ...h, note: summary } : h));
      return;
    }
    // Unfiltered, the list is newest first: move the note to the top, or drop it if it left the folder.
    const others = hits.filter((h) => h.note.id !== summary.id);
    hits = inFolder(summary.folder, selectedFolder) ? [{ note: summary, snippet: null }, ...others] : others;
  }

  /** Selects the stored folder again, if it still exists and nothing was selected since. */
  async function restoreFolder() {
    const f = await api.sidebarFolder().catch(() => null);
    if (f !== null && selectedFolder === null && folders.some((x) => x === f || x.startsWith(`${f}/`))) {
      selectedFolder = f;
    }
    folderRestored = true;
  }

  $effect(() => {
    const folder = selectedFolder;
    if (folderRestored) void api.setSidebarFolder(folder).catch(() => {});
  });

  async function loadFolders() {
    const sized = loadSizes();
    try {
      const listed = await api.folders();
      // Most refreshes (a save, a note created in an existing folder) leave the list as
      // it was; keeping the same array spares rebuilding both trees.
      if (!sameFolders(listed, folders)) folders = listed;
      const known = new Set(folders.map((f) => f.toLowerCase()));
      emptyFolders = emptyFolders.filter((f) => !known.has(f.toLowerCase()));
    } catch (e) {
      error = errorMessage(e);
    }
    await sized;
  }

  /** Reloads the folder sizes; an answer overtaken by a newer load is dropped. */
  async function loadSizes() {
    const run = ++sizesGeneration;
    try {
      const listed = await api.folderSizes();
      if (run === sizesGeneration) folderSizes = listed;
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
      const query = toQuery(text, queryFolder, filters, reset ? 0 : hits.length, PAGE_SIZE, searchMode);
      if (reset) currentSearch.query = searching ? query : null;
      // The first meaning search after unlock loads the model for its query: say so.
      let waiting = true;
      if (reset && searchMode === "meaning" && text.trim().length > 1) {
        void api.meaningModelLoaded().then((loaded) => {
          if (run === generation && waiting && !loaded) loadingModel = true;
        }, () => {});
      }
      const result = await api.searchNotes(query).finally(() => (waiting = false));
      if (run === generation) loadingModel = false;
      if (run !== generation || result.cancelled) return;
      const page = result.hits;
      if (reset) {
        hits = page;
        newestFirst = result.newestFirst;
        meaningResult = { byMeaning: result.byMeaning, top: result.top, wordsOnly: result.wordsOnly };
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
    editing = false;
    editNotice = null;
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

  function notesOf(ids: Iterable<number>): NoteSummary[] {
    return [...ids].flatMap((id) => {
      const note = hits.find((h) => h.note.id === id)?.note ?? findNote(id);
      return note ? [note] : [];
    });
  }

  function pickedNotes(): NoteSummary[] {
    return notesOf(picked);
  }

  /** Notes dragged onto a folder in the tree; moved notes that were picked are unpicked. */
  async function dropNotes(ids: number[], folder: string) {
    if (busy) return;
    busy = true;
    const ok = await onmove(ids, folder, notesOf(ids).map((n) => n.folder));
    busy = false;
    if (!ok) return;
    for (const id of ids) picked.delete(id);
    if (picked.size === 0) clearSelection();
  }

  /** A folder dragged onto another folder in the tree. */
  async function dropFolder(from: string, to: string) {
    if (busy) return;
    busy = true;
    await onmovefolder(from, to);
    busy = false;
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

  function deleteSelection() {
    void trash([...picked]);
  }

  /** Moves notes to the trash; any of them that were picked are unpicked. */
  async function trash(ids: number[]) {
    if (busy) return;
    busy = true;
    const ok = await ondelete(ids, notesOf(ids).map((n) => n.folder));
    busy = false;
    if (!ok) return;
    for (const id of ids) picked.delete(id);
    if (picked.size === 0) clearSelection();
  }

  /** The notes an action on `id` covers: the whole selection when `id` is picked, else just `id`. */
  function targetsOf(id: number): number[] {
    return picked.has(id) ? [...picked] : [id];
  }

  /** Makes `id` the selection unless it is already part of it, for Move to… and Edit… on one note. */
  function pickFor(id: number) {
    if (picked.has(id)) return;
    clearSelection();
    picked.add(id);
    anchor = id;
  }

  function openNoteMenu(id: number, x: number, y: number) {
    menu = { id, x, y };
  }

  function openFolderMenu(folder: string, x: number, y: number) {
    menu = { folder, x, y };
  }

  function closeMenu(refocus: boolean) {
    const was = menu;
    menu = null;
    if (!refocus || !was) return;
    if ("id" in was) explorer?.focusNote(was.id);
    else explorer?.focusFolder(was.folder);
  }

  const menuItems = $derived.by((): MenuItem[] => {
    if (!menu) return [];
    if ("folder" in menu) {
      const folder = menu.folder;
      return [{ label: "Move folder to trash", icon: "trash", danger: true, action: () => void deleteFolder(folder) }];
    }
    const id = menu.id;
    const n = targetsOf(id).length;
    const items: MenuItem[] = [];
    if (n === 1) {
      items.push({
        label: "Open",
        icon: "note",
        action: () => {
          clearSelection();
          anchor = id;
          onselect(id);
        },
      });
    }
    const some = n === 1 ? "" : ` ${n} notes`;
    items.push(
      {
        label: `Move${some} to…`,
        icon: "folderMove",
        action: () => {
          pickFor(id);
          moving = true;
          editing = false;
        },
      },
      {
        label: `Edit${some}…`,
        icon: "tag",
        action: () => {
          pickFor(id);
          editing = true;
          moving = false;
          void loadFacets();
        },
      },
      { label: `Move${some} to trash`, icon: "trash", danger: true, action: () => void trash(targetsOf(id)) },
    );
    return items;
  });

  const EDITS: { kind: EditKind; label: string }[] = [
    { kind: "addTag", label: "Add tag" },
    { kind: "removeTag", label: "Remove tag" },
    { kind: "setTopic", label: "Set topic" },
    { kind: "setAuthor", label: "Set author" },
    { kind: "setRating", label: "Set rating" },
    { kind: "setReadStatus", label: "Set read status" },
  ];
  const editSuggestions = $derived(
    editKind === "setTopic" ? facets.topics : editKind === "setAuthor" ? facets.authors : facets.tags,
  );
  const tagEdit = $derived(editKind === "addTag" || editKind === "removeTag");

  function togglePanel(panel: "move" | "edit") {
    moving = panel === "move" && !moving;
    editing = panel === "edit" && !editing;
    if (editing) void loadFacets();
  }

  async function editSelection(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    const value = editValue.trim();
    if (tagEdit && value.replace(/^#+/, "") === "") return;
    busy = true;
    const ids = [...picked];
    if (editKind === "setReadStatus") {
      await markSelection(ids, value as ReadStatus);
      busy = false;
      return;
    }
    const edit: MetadataEdit =
      editKind === "setRating" ? { kind: editKind, value: value ? Number(value) : null } : { kind: editKind, value };
    const report = await onedit(ids, edit, pickedNotes().map((n) => n.folder));
    busy = false;
    if (!report) return;
    editNotice = describeEdit(editKind, value.replace(/^#+/, ""), report);
    editValue = "";
    // A filtered list may have dropped some of them; keep only what is still shown.
    if (view === "recent" || searching) {
      const shown = new Set(hits.map((h) => h.note.id));
      for (const id of ids) if (!shown.has(id)) picked.delete(id);
    }
  }

  /** Marks the selected notes. Nothing is saved to them, so the lists only reload when
   * they are filtered on read status. */
  async function markSelection(ids: number[], status: ReadStatus) {
    let changed: number[];
    try {
      changed = await markRead(ids, status);
    } catch (e) {
      error = errorMessage(e);
      return;
    }
    error = "";
    const notes = (n: number) => `${n} ${n === 1 ? "note" : "notes"}`;
    const label = (READ_STATUSES.find((s) => s.value === status)?.label ?? status).toLowerCase();
    const parts = [`Marked ${notes(changed.length)} as ${label}`];
    const already = ids.length - changed.length;
    if (already > 0) parts.push(`${notes(already)} already ${already === 1 ? "was" : "were"}`);
    editNotice = { text: parts.join("; ") + ".", titles: [] };
    if (filters.readStatus) {
      await loadNotes(true);
      const shown = new Set(hits.map((h) => h.note.id));
      for (const id of ids) if (!shown.has(id)) picked.delete(id);
    }
  }

  function describeEdit(kind: MetadataEdit["kind"], value: string, r: MetadataEditReport) {
    const notes = (n: number) => `${n} ${n === 1 ? "note" : "notes"}`;
    const n = notes(r.changed.length);
    const field = kind === "setTopic" ? "topic" : kind === "setRating" ? "rating" : "author";
    const shown = kind === "setRating" ? `★ ${value}` : `“${value}”`;
    const parts = [
      kind === "addTag"
        ? `Added “${value}” to ${n}`
        : kind === "removeTag"
          ? `Removed “${value}” from ${n}`
          : value === ""
            ? `Removed the ${field} from ${n}`
            : `Set the ${field} to ${shown} on ${n}`,
    ];
    if (r.unchanged > 0) {
      const why =
        kind === "addTag"
          ? "already had it"
          : kind === "removeTag"
            ? "didn't have it in the frontmatter"
            : value === ""
              ? `had no ${field}`
              : `already had that ${field}`;
      parts.push(`${notes(r.unchanged)} ${why}`);
    }
    if (r.stillInline.length > 0) {
      const have = r.stillInline.length === 1 ? "still has" : "still have";
      parts.push(`${r.stillInline.length} ${have} it as an inline #${value}`);
    }
    if (r.unreadable.length > 0) parts.push(`${notes(r.unreadable.length)} skipped: frontmatter can't be read`);
    const title = (t: { folder: string; title: string }) => (t.folder ? `${t.folder}/${t.title}` : t.title);
    return { text: parts.join("; ") + ".", titles: [...r.stillInline, ...r.unreadable].map(title) };
  }

  function addFolder(folder: string) {
    if (!allFolders.some((f) => f.toLowerCase() === folder.toLowerCase())) emptyFolders = [...emptyFolders, folder];
  }
</script>

<div class="sidebar">
  <div class="top">
    <label class="search" class:active={text !== ""}>
      <Icon name="search" />
      <input
        type="search"
        placeholder={searchPlaceholder(selectedFolder, scope)}
        aria-label={searchPlaceholder(selectedFolder, scope)}
        title={selectedFolder !== null && scope === "folder" ? `Search ${selectedFolder}` : undefined}
        spellcheck="false"
        bind:this={searchInput}
        bind:value={text}
        onkeydown={searchKeydown}
      />
      {#if text}
        <button class="ghost small icon" aria-label="Clear search" onclick={() => (text = "")}>
          <Icon name="close" size={14} strokeWidth={2} />
        </button>
      {:else}
        <span class="kbd key">Ctrl K</span>
      {/if}
    </label>
    <div class="bar">
      <div class="segmented" role="group" aria-label="Note list view">
        <button aria-pressed={view === "tree"} onclick={() => setView("tree")}>Tree</button>
        <button aria-pressed={view === "recent"} onclick={() => setView("recent")}>Recent</button>
        <button
          aria-pressed={view === "suggestions"}
          onclick={() => setView("suggestions")}
          title="Review one tag's suggestions across the vault"
        >
          Suggestions
        </button>
      </div>
      <div class="spacer"></div>
      <button
        class="small filters"
        onclick={toggleFilters}
        aria-expanded={showFilters}
        aria-pressed={filterCount > 0}
      >
        <Icon name="filter" size={14} />
        Filters{#if filterCount > 0}<span class="count">{filterCount}</span>{/if}
      </button>
    </div>
    {#if showFilters}
      <SearchFilters bind:filters {facets} />
    {/if}
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  {#if searching}
    <div class="summary" role="status" aria-label={searchSummary(hits.length, hasMore, text, newestFirst, queryFolder, meaningResult)}>
      <p>
        {resultCount(hits.length, hasMore)} in
        <span class="scope" title={queryFolder ?? undefined}>{scopeLabel(queryFolder)}</span>{resultOrder(text, newestFirst, meaningResult)}
        {#if loadingModel}<span class="note">· Loading model…</span>{/if}
        {#if searchMode === "meaning" && meaningResult.wordsOnly && text.trim()}
          <span class="note">· by words: {meaningResult.wordsOnly}</span>
        {/if}
        {#if coverage}<span class="note coverage">· {coverage}</span>{/if}
      </p>
      <div
        class="segmented mode-switch"
        role="group"
        aria-label="Search mode"
        title={meaningReason ?? "Alt+M switches while typing"}
      >
        <button aria-pressed={searchMode === "words"} onclick={() => setMode("words")}>Words</button>
        <button
          aria-pressed={searchMode === "meaning"}
          disabled={meaningReason !== null}
          title={meaningReason ?? "Find notes by what they are about, as well as by their words"}
          onclick={() => setMode("meaning")}
        >
          Meaning
        </button>
      </div>
      {#if selectedFolder !== null}
        <div class="segmented scope-switch" role="group" aria-label="Search scope" title="Alt+F switches while typing">
          <button aria-pressed={scope === "folder"} title="Search {selectedFolder}" onclick={() => setScope("folder")}>
            This folder
          </button>
          <button aria-pressed={scope === "vault"} onclick={() => setScope("vault")}>Whole vault</button>
        </div>
      {/if}
    </div>
  {/if}
  {#if !searching && recentOpened.length > 0}
    <RecentlyOpened
      notes={recentOpened}
      {selectedId}
      collapsed={recentCollapsed}
      ontoggle={toggleRecent}
      onopen={openRecent}
    />
  {/if}
  {#if view === "suggestions" && !searching}
    <Suggestions
      bind:tag={reviewedTag}
      {selectedId}
      onopen={onselect}
      onaccept={onaccepttag}
      onreject={onrejecttag}
    />
  {:else if view === "tree" && !searching}
    <div class="explorer">
      <ExplorerTree
        bind:this={explorer}
        folders={allFolders}
        {sizes}
        {selectedFolder}
        selectedNote={selected}
        onselectfolder={(f) => (selectedFolder = f)}
        onopen={pick}
        {picked}
        onselectall={selectAll}
        onclearselection={clearSelection}
        oncreatefolder={addFolder}
        onnotemenu={openNoteMenu}
        ondeletenote={(id) => void trash(targetsOf(id))}
        onfoldermenu={openFolderMenu}
        ondeletefolder={(path) => void deleteFolder(path)}
        onerror={(e) => (error = errorMessage(e))}
        ondropnotes={(ids, folder) => void dropNotes(ids, folder)}
        ondropfolder={(from, to) => void dropFolder(from, to)}
      />
    </div>
  {:else}
    {#if view === "recent"}
      <div class="folders" class:unscoped={searching && queryFolder === null && selectedFolder !== null}>
        <FolderTree
          folders={allFolders}
          {sizes}
          selected={selectedFolder}
          onselect={(f) => (selectedFolder = f)}
          oncreate={addFolder}
        />
      </div>
    {/if}
    {#if newestFirst && searching}
      <p class="broad muted" role="status">Many matches, newest first. Type more to sort by relevance.</p>
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
          emptyText={!searching ? "No notes here yet." : queryFolder !== null ? `No matching notes in ${scopeLabel(queryFolder)}.` : "No matching notes."}
          emptyAction={searching && queryFolder !== null ? { label: "Search the whole vault", run: () => setScope("vault") } : null}
        />
      {/key}
    </div>
  {/if}
  {#if picked.size > 0}
    <div class="selection">
      {#if editNotice}
        <div class="notice" role="status">
          <p>{editNotice.text}</p>
          {#if editNotice.titles.length > 0}
            <ul>
              {#each editNotice.titles as t, i (i)}<li>{t}</li>{/each}
            </ul>
          {/if}
        </div>
      {/if}
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
      {#if editing}
        <form class="edit" onsubmit={editSelection}>
          <select
            aria-label="Edit"
            bind:value={editKind}
            onchange={(e) =>
              (editValue =
                e.currentTarget.value === "setRating" ? "5" : e.currentTarget.value === "setReadStatus" ? "read" : "")}
            disabled={busy}
          >
            {#each EDITS as e (e.kind)}<option value={e.kind}>{e.label}</option>{/each}
          </select>
          {#if editKind === "setReadStatus"}
            <select aria-label="Read status" bind:value={editValue} disabled={busy}>
              {#each READ_STATUSES as r (r.value)}<option value={r.value}>{r.label}</option>{/each}
            </select>
          {:else if editKind === "setRating"}
            <select aria-label="Rating" bind:value={editValue} disabled={busy}>
              {#each [1, 2, 3, 4, 5] as n (n)}<option value={String(n)}>{"★".repeat(n)} {n}</option>{/each}
              <option value="">No rating</option>
            </select>
          {:else}
            <input
              list="edit-values"
              placeholder={tagEdit ? "Tag" : "Empty = remove"}
              aria-label={EDITS.find((e) => e.kind === editKind)?.label}
              spellcheck="false"
              bind:value={editValue}
              onkeydown={(e) => e.key === "Escape" && (editing = false)}
              disabled={busy}
              {@attach (el) => el.focus()}
            />
            <datalist id="edit-values">
              {#each editSuggestions as v (v)}<option value={v}></option>{/each}
            </datalist>
          {/if}
          <button type="submit" class="primary" disabled={busy || (tagEdit && editValue.trim() === "")}>Apply</button>
        </form>
      {/if}
      <div class="actions" role="toolbar" aria-label="Selected notes">
        <span class="picked">{picked.size} selected</span>
        <button onclick={() => togglePanel("move")} aria-expanded={moving} disabled={busy}>
          <Icon name="folderMove" size={14} />
          Move to…
        </button>
        <button onclick={() => togglePanel("edit")} aria-expanded={editing} disabled={busy}>
          <Icon name="tag" size={14} />
          Edit…
        </button>
        <button onclick={deleteSelection} disabled={busy} title="Move the selected notes to the trash">
          <Icon name="trash" size={14} />
          Delete
        </button>
        <button class="esc" onclick={clearSelection} disabled={busy} aria-label="Clear selection (Esc)">Esc</button>
      </div>
    </div>
  {/if}
  {#if menu}
    <ContextMenu
      x={menu.x}
      y={menu.y}
      label={"folder" in menu
        ? "Folder actions"
        : targetsOf(menu.id).length === 1
          ? "Note actions"
          : "Actions for the selected notes"}
      items={menuItems}
      onclose={closeMenu}
    />
  {/if}
  <div class="footer">
    <!-- The status line opens the Activity panel, which says what every job is doing. -->
    <div role="status">
      {#if modelReason && !firstIndex}
        <button class="ghost index failed wrap" onclick={() => model.open?.()} title="Off: {MODEL_FEATURES}. Open Settings → Model">
          Model off: {modelReason}. Settings → Model
        </button>
      {:else if progress}
        <button class="ghost index" onclick={onactivity} title="Show activity (Ctrl J)">
          Indexing: {indexText(progress)}{#if progress.etaSecs != null},
            {timeLeft(progress.etaSecs)}{/if}
          {#if progress.total - progress.complete >= PAUSE_NOTICE_MIN}
            <span class="pause">Indexing pauses when the vault locks.</span>
          {/if}
        </button>
        {#if firstIndex}
          <button class="ghost index wrap" onclick={() => model.open?.()} title="Off until then: {MODEL_FEATURES}">
            Related notes and suggestions start after the first index. Settings → Model
          </button>
        {/if}
      {:else if indexStatus.state === "paused"}
        <button class="ghost index" onclick={onactivity} title="Show activity (Ctrl J)">
          Indexing paused while importing or compacting.
        </button>
      {:else if modelReason}
        <button class="ghost index wrap" onclick={() => model.open?.()} title="Off: {MODEL_FEATURES}. Open Settings → Model">
          Model not ready: {modelReason}. Settings → Model
        </button>
      {:else if indexStatus.state === "failed"}
        <button class="ghost index failed" onclick={onactivity} title={indexStatus.message}>
          Indexing stopped: {indexStatus.message}
        </button>
      {/if}
    </div>
    <button class="trash" class:open={activityOpen} aria-pressed={activityOpen} onclick={onactivity}>
      <Icon name="activity" />
      <span>Activity</span>
      <span class="n kbd">Ctrl J</span>
    </button>
    <button class="trash" class:open={trashOpen} aria-current={trashOpen ? "page" : undefined} onclick={ontrash}>
      <Icon name="trash" />
      <span>Trash</span>
      {#if trashCount > 0}<span class="n">{trashCount.toLocaleString()}</span>{/if}
    </button>
  </div>
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .top {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    max-height: 60%;
    overflow-y: auto;
  }

  .search {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    min-height: 34px;
    padding: 0 6px 0 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--muted);
    font-size: 14px;
  }

  .search:focus-within,
  .search.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .search input {
    flex: 1;
    min-width: 0;
    min-height: 30px;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    font-size: 14px;
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .key {
    padding: 1px 5px;
    margin-right: 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .filters {
    background: transparent;
    color: var(--text-2);
  }

  .filters[aria-pressed="true"],
  .filters[aria-expanded="true"] {
    border-color: var(--accent);
    color: var(--accent);
  }

  .count {
    min-width: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 11px;
    line-height: 18px;
    text-align: center;
  }

  .error {
    padding: 0 12px 8px;
  }

  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    padding: 4px 16px 6px;
    font-size: 12px;
    color: var(--muted);
  }

  .summary p {
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
  }

  .summary .scope {
    color: var(--text);
  }

  .scope-switch button,
  .mode-switch button {
    min-height: 22px;
    padding: 0 8px;
    font-size: 11px;
  }

  /* A whole-vault search: the folder highlight is not a filter now. */
  .folders.unscoped :global(.row.selected) {
    opacity: 0.55;
  }

  .folders {
    flex: none;
    max-height: 30vh;
    overflow-y: auto;
    padding: 0 12px 12px;
    border-bottom: 1px solid var(--border);
  }

  /* Room for a few rows of tree or results when the sidebar is short; the
     Recently opened panel above gives way first. */
  .explorer {
    flex: 1;
    min-height: 120px;
  }

  .results {
    flex: 1;
    min-height: 120px;
    display: grid;
    grid-template-rows: minmax(0, 1fr);
  }

  .selection {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 8px;
  }

  .move,
  .edit {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px;
  }

  .edit {
    grid-template-columns: auto minmax(0, 1fr) auto;
  }

  .edit select {
    width: auto;
  }

  .notice {
    max-height: 20vh;
    overflow-y: auto;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: 12.5px;
  }

  .notice p,
  .notice ul {
    margin: 0;
  }

  .notice ul {
    padding-left: 1.2em;
  }

  /* Inverted, so it reads as a mode rather than part of the list. */
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
    background: var(--text);
    color: var(--bg);
  }

  .picked {
    flex: 1;
    font-weight: 500;
  }

  .actions button {
    gap: 6px;
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: var(--inverse-button);
    color: var(--bg);
    font-size: 13px;
  }

  .actions button:hover:not(:disabled) {
    filter: brightness(1.15);
  }

  .actions .esc {
    padding: 0 8px;
    background: transparent;
    font-family: var(--mono);
    font-size: 11px;
    opacity: 0.85;
  }

  .footer {
    flex: none;
    padding: 8px;
    border-top: 1px solid var(--border);
  }

  .trash {
    width: 100%;
    justify-content: flex-start;
    padding: 0 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
    font-size: 13.5px;
  }

  .trash:hover:not(:disabled) {
    background: var(--hover);
  }

  .trash span:first-of-type {
    flex: 1;
    text-align: left;
  }

  .trash.open {
    background: var(--selection);
    color: var(--selection-text);
    font-weight: 500;
  }

  .n {
    font-size: 12px;
    color: var(--muted);
  }

  .trash.open .n {
    color: inherit;
  }

  .index .pause {
    display: block;
  }

  .index {
    display: block;
    width: 100%;
    height: auto;
    text-align: left;
    font-weight: normal;
    padding: 2px 8px 6px;
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .index.failed {
    color: var(--warn);
  }

  /* The model lines must show their reason and the Settings link in full. */
  .index.wrap {
    white-space: normal;
  }
  .broad {
    margin: 0;
    padding: 4px 12px;
    font-size: 0.85em;
  }
</style>
