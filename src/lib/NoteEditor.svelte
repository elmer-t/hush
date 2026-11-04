<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, type NoteMeta, type NoteSummary, type OpenedNote } from "$lib/api";
  import { confirmTrash } from "$lib/confirmTrash";
  import FindBar from "$lib/FindBar.svelte";
  import { findPattern, formatCount, type FindSpec } from "$lib/find";
  import Icon from "$lib/Icon.svelte";
  import { normalizeFolder } from "$lib/markdown";
  import MarkdownEditor from "$lib/MarkdownEditor.svelte";
  import Preview from "$lib/Preview.svelte";

  type Props = {
    opened: OpenedNote;
    focusTitle: boolean;
    /** The remembered "Preview only" choice, shared across notes for the session. */
    previewOnly: boolean;
    onsaved: (summary: NoteSummary, meta: NoteMeta) => void;
    onrenamed: (opened: OpenedNote) => void;
    ondeleted: (id: number) => void;
    /** A `[[wikilink]]` in the preview was clicked. */
    onwikilink: (target: string, x: number, y: number) => void;
  };
  let {
    opened,
    focusTitle,
    previewOnly = $bindable(),
    onsaved,
    onrenamed,
    ondeleted,
    onwikilink,
  }: Props = $props();

  const SAVE_DELAY_MS = 500;

  // The parent remounts this component per note, so the initial note is all it needs.
  const initial = untrack(() => opened.note);
  const id = initial.id;

  let body = $state(initial.body);
  let savedBody = initial.body;
  let title = $state(initial.title);
  let folder = $state(initial.folder);
  let committed = { title: initial.title, folder: initial.folder };
  // A new note opens in split view so it can be typed in, without changing the remembered choice.
  let forceSplit = $state(untrack(() => focusTitle));
  const showPreviewOnly = $derived(previewOnly && !forceSplit);
  let status = $state<"saved" | "pending" | "saving" | "failed">("saved");
  let error = $state("");
  let editor = $state<ReturnType<typeof MarkdownEditor>>();
  let titleInput = $state<HTMLInputElement>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let saving: Promise<void> = Promise.resolve();

  // Find in note. The bar drives the editor's search in split view, and the preview's in
  // "Preview only"; in split view the preview highlights the same query too.
  let findOpen = $state(false);
  let replacing = $state(false);
  let find = $state<FindSpec>({ query: "", caseSensitive: false, wholeWord: false, regexp: false, replace: "" });
  let findResult = $state({ current: -1, total: 0 });
  let findBar = $state<ReturnType<typeof FindBar>>();
  let preview = $state<ReturnType<typeof Preview>>();
  let returnFocus: HTMLElement | null = null;

  /** The query while there is one to show; a fresh object, so the panes see every change. */
  const activeFind = $derived(findOpen && find.query ? { ...find } : null);
  const findCount = $derived.by(() => {
    if (!activeFind) return "";
    if (activeFind.regexp && !findPattern(activeFind.query, activeFind)) return "Invalid pattern";
    return formatCount(findResult.current, findResult.total);
  });

  $effect(() => {
    const spec = showPreviewOnly ? null : activeFind;
    untrack(() => editor?.setFind(spec));
    if (!spec) untrack(() => (findResult = { current: -1, total: 0 }));
  });

  /** Opens the find bar, prefilled with the selected text; `replace` opens the replace row too. */
  export async function openFind(replace = false) {
    const selected = editor?.selectedText() || preview?.selectedText() || "";
    if (!findOpen) returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (selected && !selected.includes("\n")) find.query = selected;
    findOpen = true;
    if (replace && !showPreviewOnly) replacing = true;
    await tick();
    await findBar?.focus(replace && !showPreviewOnly ? "replace" : "find");
  }

  /** Closes the find bar, clears the highlights and gives focus back to where it was. */
  function closeFind() {
    findOpen = false;
    replacing = false;
    const target = returnFocus;
    returnFocus = null;
    if (target?.isConnected && target.offsetParent !== null) target.focus();
    else if (!showPreviewOnly) editor?.focus();
  }

  function findStep(dir: 1 | -1) {
    if (!activeFind) return;
    if (showPreviewOnly) preview?.findStep(dir);
    else editor?.findStep(dir);
  }

  function editorFindResult(current: number, total: number) {
    if (!showPreviewOnly) findResult = { current, total };
  }

  function previewFindResult(current: number, total: number) {
    if (showPreviewOnly) findResult = { current, total };
  }

  // F3 anywhere in the note, and Esc in its panes, while the bar is open (the bar's own fields
  // handle Esc themselves; the title and folder fields keep theirs).
  function findKeys(e: KeyboardEvent & { currentTarget: HTMLElement }) {
    if (!findOpen) return;
    if (e.key === "F3") {
      e.preventDefault();
      e.stopPropagation();
      findStep(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape" && (e.target as Element).closest(".panes")) {
      e.preventDefault();
      e.stopPropagation();
      closeFind();
    }
  }

  const statusText = $derived(
    { saved: "Saved", pending: "Edited", saving: "Saving…", failed: "Not saved" }[status],
  );

  onMount(() => {
    if (untrack(() => focusTitle)) titleInput?.select();
    else editor?.focus();
    return () => clearTimeout(timer);
  });

  function onchange(doc: string) {
    body = doc;
    if (body === savedBody) return;
    status = "pending";
    clearTimeout(timer);
    timer = setTimeout(() => void flush(), SAVE_DELAY_MS);
  }

  /** Saves pending edits now. Never throws; a failure is shown in the editor. */
  export async function flush(): Promise<void> {
    clearTimeout(timer);
    await saving;
    if (body === savedBody) return;
    const toSave = body;
    status = "saving";
    saving = api.saveNote(id, toSave).then(
      (saved) => {
        savedBody = toSave;
        status = body === toSave ? "saved" : "pending";
        error = "";
        onsaved(saved.summary, saved.meta);
      },
      (e) => {
        status = "failed";
        error = errorMessage(e);
      },
    );
    await saving;
  }

  /** Whether there are edits that aren't saved, e.g. after `flush` failed. */
  export function unsaved(): boolean {
    return body !== savedBody;
  }

  /** Sets a frontmatter field through the editor, so the change is undoable and autosaved. */
  export async function setField(key: string, values: string[]) {
    try {
      editor?.replace(await api.editFrontmatter(body, key, values));
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function commitName() {
    if (title.trim() === committed.title && normalizeFolder(folder) === committed.folder) {
      ({ title, folder } = committed);
      return;
    }
    await flush();
    try {
      const fresh = await api.renameNote(id, folder, title);
      committed = { title: fresh.note.title, folder: fresh.note.folder };
      ({ title, folder } = committed);
      // Renaming may rewrite this note's own links.
      if (fresh.note.body !== body) {
        savedBody = fresh.note.body;
        editor?.replace(fresh.note.body);
      }
      error = "";
      onrenamed(fresh);
    } catch (e) {
      error = errorMessage(e);
      ({ title, folder } = committed);
    }
  }

  function setPreviewOnly(only: boolean) {
    previewOnly = only;
    forceSplit = false;
  }

  let menuOpen = $state(false);

  function closeMenuOnLeave(e: FocusEvent & { currentTarget: HTMLElement }) {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) menuOpen = false;
  }

  function nameKeydown(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") {
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      ({ title, folder } = committed);
      e.currentTarget.blur();
    }
  }

  /** Moves the note to the trash, asking first unless that is turned off in Settings. */
  async function remove() {
    menuOpen = false;
    if (!(await confirmTrash(1, committed.title))) return;
    clearTimeout(timer);
    await saving;
    try {
      await api.trashNote(id);
      ondeleted(id);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  /** Deletes the note for good, skipping the trash. */
  async function removeForever() {
    menuOpen = false;
    const confirmed = await ask(`Delete “${committed.title}” forever? It won't go to the trash. This cannot be undone.`, {
      title: "Delete forever",
      kind: "warning",
      okLabel: "Delete forever",
      cancelLabel: "Cancel",
    });
    if (!confirmed) return;
    clearTimeout(timer);
    await saving;
    try {
      await api.deleteNote(id);
      ondeleted(id);
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<section class="note" onkeydowncapture={findKeys}>
  <header>
    <div class="toolbar">
      <label class="folder">
        <Icon name="folder" size={14} />
        <span class="visually-hidden">Folder</span>
        <input
          placeholder="No folder"
          spellcheck="false"
          bind:value={folder}
          onblur={commitName}
          onkeydown={nameKeydown}
        />
      </label>
      <div class="spacer"></div>
      <span class="status" class:failed={status === "failed"} role="status">
        {#if status === "saved"}<Icon name="check" size={14} strokeWidth={2} />{/if}
        {statusText}
      </span>
      <div class="segmented" role="group" aria-label="Editor view">
        <button aria-pressed={!showPreviewOnly} onclick={() => setPreviewOnly(false)}>
          <Icon name="split" size={14} />
          Split
        </button>
        <button aria-pressed={showPreviewOnly} onclick={() => setPreviewOnly(true)}>
          <Icon name="eye" size={14} />
          Preview
        </button>
      </div>
      <div class="menu" onfocusout={closeMenuOnLeave}>
        <button
          class="ghost icon"
          aria-label="More actions"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onclick={() => (menuOpen = !menuOpen)}
        >
          <Icon name="more" strokeWidth={2.5} />
        </button>
        {#if menuOpen}
          <div class="popover" role="menu" tabindex="-1" onkeydown={(e) => e.key === "Escape" && (menuOpen = false)}>
            <button role="menuitem" class="ghost" onclick={remove}>
              <Icon name="trash" />
              Move to trash
            </button>
            <button role="menuitem" class="ghost danger" onclick={removeForever}>
              <Icon name="close" />
              Delete forever
            </button>
          </div>
        {/if}
      </div>
    </div>
    <input
      class="title"
      aria-label="Title"
      bind:this={titleInput}
      bind:value={title}
      onblur={commitName}
      onkeydown={nameKeydown}
    />
  </header>

  {#if findOpen}
    <FindBar
      bind:this={findBar}
      bind:spec={find}
      bind:replacing
      canReplace={!showPreviewOnly}
      count={findCount}
      onstep={findStep}
      onreplace={(all) => editor?.replaceMatch(all)}
      onclose={closeFind}
    />
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="panes" class:preview-only={showPreviewOnly}>
    <div class="source">
      <MarkdownEditor bind:this={editor} doc={initial.body} {onchange} onfindresult={editorFindResult} />
    </div>
    <div class="rendered">
      <Preview
        bind:this={preview}
        {body}
        large={showPreviewOnly}
        {onwikilink}
        find={activeFind}
        findLead={showPreviewOnly}
        onfindresult={previewFindResult}
      />
    </div>
  </div>
</section>

<style>
  .note {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 24px 12px;
    border-bottom: 1px solid var(--border);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
  }

  .folder {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    background: var(--panel);
    color: var(--muted);
  }

  .folder input {
    width: 220px;
    min-height: 0;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text-2);
    font-family: var(--mono);
    font-size: 12px;
  }

  .folder:focus-within {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .spacer {
    flex: 1;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--muted);
  }

  .status.failed {
    color: var(--danger);
  }

  .menu {
    position: relative;
  }

  .popover {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--raised);
    box-shadow: var(--shadow);
  }

  .popover button {
    justify-content: flex-start;
    color: var(--text);
    font-size: 13px;
  }

  .popover button.danger {
    color: var(--danger);
  }

  .title {
    min-height: 0;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .error {
    padding: 8px 24px;
  }

  .panes {
    flex: 1;
    display: grid;
    grid-template-columns: 1fr 1fr;
    min-height: 0;
  }

  .source,
  .rendered {
    min-height: 0;
    overflow: auto;
  }

  .source {
    background: var(--code);
    border-right: 1px solid var(--border);
  }

  /* The editor stays mounted so its undo history survives toggling. */
  .preview-only {
    grid-template-columns: 1fr;
  }

  .preview-only .source {
    display: none;
  }
</style>
