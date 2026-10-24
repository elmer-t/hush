<script lang="ts">
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, type OpenedNote, type TrashedNote } from "$lib/api";
  import { longDate } from "$lib/dates";

  type Props = {
    /** Called with the restored note, which may have a new title. */
    onrestored: (note: OpenedNote) => void;
  };
  let { onrestored }: Props = $props();

  let notes = $state<TrashedNote[] | null>(null);
  let days = $state<number | null>(null);
  let busy = $state(false);
  let error = $state("");

  onMount(() => {
    void load();
    api.trashDays().then((d) => (days = d), () => {});
  });

  async function load() {
    try {
      notes = await api.trashedNotes();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function run(action: () => Promise<void>) {
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

  function restore(note: TrashedNote) {
    void run(async () => onrestored(await api.restoreNote(note.id)));
  }

  async function deleteForever(note: TrashedNote) {
    const confirmed = await ask(`Delete “${note.title}” forever? This cannot be undone.`, {
      title: "Delete forever",
      kind: "warning",
      okLabel: "Delete forever",
      cancelLabel: "Cancel",
    });
    if (confirmed) void run(() => api.deleteNote(note.id));
  }

  async function empty() {
    const count = notes?.length ?? 0;
    const confirmed = await ask(
      `Delete ${count === 1 ? "the note" : `all ${count} notes`} in the trash forever? This cannot be undone.`,
      { title: "Empty trash", kind: "warning", okLabel: "Empty trash", cancelLabel: "Cancel" },
    );
    if (confirmed) void run(async () => void (await api.emptyTrash()));
  }
</script>

<div class="trash">
  <header>
    <div>
      <h2>Trash</h2>
      <p class="muted">
        {#if days === 0}
          Deleted notes stay here until you delete them forever.
        {:else if days !== null}
          Deleted notes are deleted forever {days} {days === 1 ? "day" : "days"} after they were moved here. You can
          change this in Settings.
        {/if}
      </p>
    </div>
    <button class="danger" onclick={empty} disabled={busy || !notes?.length}>Empty trash</button>
  </header>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if notes && notes.length === 0}
    <p class="muted">The trash is empty.</p>
  {:else if notes}
    <ul>
      {#each notes as note (note.id)}
        <li>
          <div class="name">
            <strong>{note.title}</strong>
            <span class="muted">{note.folder || "No folder"} · deleted {longDate(note.deletedAt)}</span>
          </div>
          <button onclick={() => restore(note)} disabled={busy}>Restore</button>
          <button class="danger" onclick={() => deleteForever(note)} disabled={busy}>Delete forever</button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .trash {
    max-width: 720px;
    margin: 0 auto;
    display: grid;
    gap: 1rem;
    align-content: start;
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  h2 {
    margin: 0 0 0.25rem;
    font-size: 1rem;
  }

  p {
    margin: 0;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }

  li:last-child {
    border-bottom: none;
  }

  .name {
    flex: 1;
    min-width: 0;
    display: grid;
  }

  .name > * {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name .muted {
    font-size: 0.85em;
  }

  .danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
