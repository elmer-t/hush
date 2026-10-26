<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, type NoteMeta, type NoteSummary, type OpenedNote } from "$lib/api";
  import { confirmTrash } from "$lib/confirmTrash";
  import Icon from "$lib/Icon.svelte";
  import { normalizeFolder } from "$lib/markdown";
  import MarkdownEditor from "$lib/MarkdownEditor.svelte";
  import Preview from "$lib/Preview.svelte";

  type Props = {
    opened: OpenedNote;
    focusTitle: boolean;
    /** The remembered "Preview only" choice, shared across notes for the session. */
    previewOnly: boolean;
    /** Whether the details panel beside the editor is shown. */
    detailsShown: boolean;
    ontoggledetails: () => void;
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
    detailsShown,
    ontoggledetails,
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

<section class="note">
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
      <button
        class="ghost icon"
        aria-label={detailsShown ? "Hide details panel" : "Show details panel"}
        aria-pressed={detailsShown}
        title={detailsShown ? "Hide details panel" : "Show details panel"}
        onclick={ontoggledetails}
      >
        <Icon name="panelRight" />
      </button>
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

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="panes" class:preview-only={showPreviewOnly}>
    <div class="source">
      <MarkdownEditor bind:this={editor} doc={initial.body} {onchange} />
    </div>
    <div class="rendered">
      <Preview {body} large={showPreviewOnly} {onwikilink} />
    </div>
  </div>
</section>

<style>
  .note {
    display: grid;
    grid-template-rows: auto auto 1fr;
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
