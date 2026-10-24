<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, errorMessage, type OpenedNote, type TrashedNote } from "$lib/api";
  import { shortDate } from "$lib/dates";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    /** Called with the restored note, which may have a new title. */
    onrestored: (note: OpenedNote) => void;
    /** Called with the number of notes in the trash whenever it is loaded. */
    oncount: (count: number) => void;
  };
  let { onrestored, oncount }: Props = $props();

  const DAY_MS = 86_400_000;

  let notes = $state<TrashedNote[] | null>(null);
  let days = $state<number | null>(null);
  let busy = $state(false);
  let error = $state("");
  // What the confirmation dialog asks about; `null` while it is closed.
  let confirming = $state<{ title: string; text: string; action: string; run: () => Promise<unknown> } | null>(null);
  let dialog = $state<HTMLDialogElement>();

  onMount(() => {
    void load();
    api.trashDays().then((d) => (days = d), () => {});
  });

  async function load() {
    try {
      notes = await api.trashedNotes();
      oncount(notes.length);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function run(action: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await action();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
      await load();
    }
  }

  /** Days until the note is deleted for good, or null when the trash is kept forever. */
  function daysLeft(note: TrashedNote): number | null {
    if (!days) return null;
    return Math.max(0, days - Math.floor((Date.now() - note.deletedAt) / DAY_MS));
  }

  function restore(note: TrashedNote) {
    void run(async () => onrestored(await api.restoreNote(note.id)));
  }

  async function ask(question: NonNullable<typeof confirming>) {
    confirming = question;
    await tick();
    dialog?.showModal();
  }

  function answer(yes: boolean) {
    const question = confirming;
    dialog?.close();
    confirming = null;
    if (yes && question) void run(question.run);
  }

  function deleteForever(note: TrashedNote) {
    void ask({
      title: `Delete “${note.title}” forever?`,
      text: "The note is erased from the vault file. This cannot be undone.",
      action: "Delete forever",
      run: () => api.deleteNote(note.id),
    });
  }

  function empty() {
    const count = notes?.length ?? 0;
    void ask({
      title: "Empty the trash?",
      text: `${count === 1 ? "The note" : `All ${count} notes`} in the trash ${count === 1 ? "is" : "are"} erased from the vault file. This cannot be undone.`,
      action: "Empty trash",
      run: () => api.emptyTrash(),
    });
  }
</script>

<div class="trash">
  <header>
    <div class="intro">
      <h1>Trash</h1>
      <p>
        Notes here are hidden from the tree, search and links.
        {#if days === 0}
          They stay until you delete them forever.
        {:else if days !== null}
          Each unlock removes notes trashed more than {days} {days === 1 ? "day" : "days"} ago. You can change this in
          Settings.
        {/if}
      </p>
    </div>
    <button class="danger" onclick={empty} disabled={busy || !notes?.length}>
      <Icon name="trash" />
      Empty trash
    </button>
  </header>

  <div class="body">
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if notes && notes.length === 0}
      <p class="muted">The trash is empty.</p>
    {:else if notes}
      <div class="table">
        <table>
          <thead>
            <tr>
              <th scope="col">Title</th>
              <th scope="col">Original folder</th>
              <th scope="col">Deleted</th>
              {#if days}<th scope="col">Removed in</th>{/if}
              <th scope="col"><span class="visually-hidden">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each notes as note (note.id)}
              {@const left = daysLeft(note)}
              <tr>
                <td class="title">{note.title}</td>
                <td class="folder">{note.folder || "vault root"}</td>
                <td class="dim">{shortDate(note.deletedAt)}</td>
                {#if days}<td class="dim num">{left} {left === 1 ? "day" : "days"}</td>{/if}
                <td class="actions">
                  <button class="small" onclick={() => restore(note)} disabled={busy}>
                    <Icon name="restore" size={14} />
                    Restore
                  </button>
                  <button class="small ghost danger" onclick={() => deleteForever(note)} disabled={busy}>
                    Delete forever
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>

<dialog bind:this={dialog} aria-labelledby="trash-confirm-title" oncancel={() => (confirming = null)}>
  {#if confirming}
    <h2 id="trash-confirm-title">{confirming.title}</h2>
    <p>{confirming.text}</p>
    <div class="buttons">
      <button onclick={() => answer(false)}>Cancel</button>
      <button class="danger-fill" onclick={() => answer(true)}>{confirming.action}</button>
    </div>
  {/if}
</dialog>

<style>
  .trash {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 12px;
    padding: 20px 28px 16px;
    border-bottom: 1px solid var(--border);
  }

  .intro {
    flex: 1 1 320px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  p {
    margin: 0;
    color: var(--text-2);
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 8px 16px 24px;
  }

  .body > p {
    padding: 12px;
  }

  .table {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 640px;
    border-collapse: collapse;
    font-size: 13.5px;
  }

  th {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    font-weight: 500;
    color: var(--muted);
    text-align: left;
  }

  td {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }

  .title {
    font-weight: 500;
  }

  .folder {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .dim {
    color: var(--text-2);
  }

  .num {
    font-variant-numeric: tabular-nums;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  dialog {
    width: calc(100% - 32px);
    max-width: 420px;
    padding: 24px;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--raised);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  dialog::backdrop {
    background: var(--scrim);
  }

  dialog h2 {
    margin: 0 0 12px;
    font-size: 17px;
    font-weight: 600;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 20px;
  }

  .buttons button {
    min-height: 36px;
    padding: 0 14px;
  }
</style>
