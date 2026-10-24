<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, type NoteMeta, type NoteSummary, type OpenedNote } from "$lib/api";
  import { normalizeFolder } from "$lib/markdown";
  import MarkdownEditor from "$lib/MarkdownEditor.svelte";
  import Preview from "$lib/Preview.svelte";

  type Props = {
    opened: OpenedNote;
    focusTitle: boolean;
    onsaved: (summary: NoteSummary, meta: NoteMeta) => void;
    onrenamed: (opened: OpenedNote) => void;
    ondeleted: (id: number) => void;
  };
  let { opened, focusTitle, onsaved, onrenamed, ondeleted }: Props = $props();

  const SAVE_DELAY_MS = 500;

  // The parent remounts this component per note, so the initial note is all it needs.
  const initial = untrack(() => opened.note);
  const id = initial.id;

  let body = $state(initial.body);
  let savedBody = initial.body;
  let title = $state(initial.title);
  let folder = $state(initial.folder);
  let committed = { title: initial.title, folder: initial.folder };
  let previewOnly = $state(false);
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

  function nameKeydown(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") {
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      ({ title, folder } = committed);
      e.currentTarget.blur();
    }
  }

  /** Moves the note to the trash. No confirmation: it can be restored from there. */
  async function remove() {
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
    <div class="names">
      <input
        class="folder"
        aria-label="Folder"
        placeholder="No folder"
        spellcheck="false"
        bind:value={folder}
        onblur={commitName}
        onkeydown={nameKeydown}
      />
      <input
        class="title"
        aria-label="Title"
        bind:this={titleInput}
        bind:value={title}
        onblur={commitName}
        onkeydown={nameKeydown}
      />
    </div>
    <div class="tools">
      <span class="muted status" class:failed={status === "failed"}>{statusText}</span>
      <button onclick={() => (previewOnly = !previewOnly)} aria-pressed={previewOnly}>
        {previewOnly ? "Split view" : "Preview only"}
      </button>
      <button class="danger" onclick={remove} title="Move to the trash">Delete</button>
      <button class="danger" onclick={removeForever} title="Delete for good, skipping the trash">Delete forever</button>
    </div>
  </header>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="panes" class:preview-only={previewOnly}>
    <div class="source">
      <MarkdownEditor bind:this={editor} doc={initial.body} {onchange} />
    </div>
    <div class="rendered">
      <Preview {body} />
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
    align-items: flex-start;
    gap: 1rem;
    padding: 0.75rem 1.25rem 0.5rem;
  }

  .names {
    flex: 1;
    min-width: 0;
    display: grid;
  }

  .names input {
    border-color: transparent;
    background: transparent;
    padding: 0.1em 0.3em;
  }

  .names input:hover {
    border-color: var(--border);
  }

  .folder {
    font-size: 0.85em;
    color: var(--muted);
  }

  .title {
    font-size: 1.5em;
    font-weight: 700;
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding-top: 0.3rem;
  }

  .status {
    font-size: 0.85em;
    min-width: 5em;
    text-align: right;
  }

  .status.failed {
    color: var(--danger);
  }

  .danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .error {
    padding: 0 1.25rem 0.5rem;
  }

  .panes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    min-height: 0;
    border-top: 1px solid var(--border);
  }

  .source,
  .rendered {
    min-height: 0;
    overflow: auto;
  }

  .source {
    border-right: 1px solid var(--border);
  }

  /* The editor stays mounted so its undo history survives toggling. */
  .preview-only {
    grid-template-columns: 1fr;
  }

  .preview-only .source {
    display: none;
  }

  .preview-only .rendered {
    max-width: 50rem;
    width: 100%;
    justify-self: center;
  }
</style>
