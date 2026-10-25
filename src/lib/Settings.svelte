<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage } from "$lib/api";
  import TagSettings from "$lib/TagSettings.svelte";

  type Props = {
    /** Opens the Suggestions view for a tag. */
    onreviewtag: (tag: string) => void;
  };
  let { onreviewtag }: Props = $props();

  let minutes = $state(10);
  let minutesStatus = $state("");
  let minutesError = $state("");

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let changing = $state(false);
  let passphraseStatus = $state("");
  let passphraseError = $state("");

  let confirmTrash = $state(true);
  let allCores = $state(false);
  let coresError = $state("");
  let confirmError = $state("");

  let trashDays = $state(30);
  let trashStatus = $state("");
  let trashError = $state("");

  let author = $state("");
  let authorStatus = $state("");
  let authorError = $state("");

  onMount(async () => {
    api.indexAllCores().then((on) => (allCores = on), () => {});
    [minutes, author, trashDays, confirmTrash] = await Promise.all([
      api.autoLockMinutes(),
      api.defaultAuthor().then((a) => a ?? ""),
      api.trashDays(),
      api.confirmTrash(),
    ]);
  });

  async function saveAllCores() {
    coresError = "";
    try {
      await api.setIndexAllCores(allCores);
    } catch (err) {
      coresError = errorMessage(err);
      allCores = !allCores;
    }
  }

  // Saved on change: a checkbox has no Save button.
  async function saveConfirmTrash() {
    confirmError = "";
    try {
      await api.setConfirmTrash(confirmTrash);
    } catch (err) {
      confirmError = errorMessage(err);
      confirmTrash = !confirmTrash;
    }
  }

  async function saveTrashDays(e: SubmitEvent) {
    e.preventDefault();
    trashStatus = trashError = "";
    try {
      await api.setTrashDays(trashDays);
      trashStatus = "Saved.";
    } catch (err) {
      trashError = errorMessage(err);
    }
  }

  async function saveAuthor(e: SubmitEvent) {
    e.preventDefault();
    authorStatus = authorError = "";
    try {
      await api.setDefaultAuthor(author);
      authorStatus = "Saved.";
    } catch (err) {
      authorError = errorMessage(err);
    }
  }

  async function saveMinutes(e: SubmitEvent) {
    e.preventDefault();
    minutesStatus = minutesError = "";
    try {
      await api.setAutoLockMinutes(minutes);
      minutesStatus = "Saved.";
    } catch (err) {
      minutesError = errorMessage(err);
    }
  }

  async function changePassphrase(e: SubmitEvent) {
    e.preventDefault();
    passphraseStatus = passphraseError = "";
    if (next !== confirm) {
      passphraseError = "The new passphrases do not match.";
      return;
    }
    changing = true;
    try {
      await api.changePassphrase(current, next);
      passphraseStatus = "Passphrase changed.";
      current = next = confirm = "";
    } catch (err) {
      passphraseError = errorMessage(err);
    } finally {
      changing = false;
    }
  }
</script>

<div class="settings">
  <h1>Settings</h1>
  <form class="card" onsubmit={saveAuthor}>
    <h2>Default author</h2>
    <label>
      Added to the frontmatter of new notes
      <input bind:value={author} placeholder="No default author" />
    </label>
    {#if authorError}<p class="error" role="alert">{authorError}</p>{/if}
    <div class="actions">
      {#if authorStatus}<span class="muted">{authorStatus}</span>{/if}
      <button type="submit">Save</button>
    </div>
  </form>

  <TagSettings onreview={onreviewtag} />

  <form class="card" onsubmit={saveMinutes}>
    <h2>Auto-lock</h2>
    <label>
      Lock after this many idle minutes
      <input type="number" min="1" max="1440" step="1" bind:value={minutes} required />
    </label>
    {#if minutesError}<p class="error" role="alert">{minutesError}</p>{/if}
    <div class="actions">
      {#if minutesStatus}<span class="muted">{minutesStatus}</span>{/if}
      <button type="submit">Save</button>
    </div>
  </form>

  <section class="card">
    <h2>Indexing</h2>
    <p class="muted">
      Hush works out what each note is about on this computer, in the background. Indexing pauses while the vault is
      locked and picks up where it left off, most recently changed notes first.
    </p>
    <label class="check">
      <input type="checkbox" bind:checked={allCores} onchange={saveAllCores} />
      Use all cores while indexing (faster, but the computer may get busy and warm)
    </label>
    {#if coresError}<p class="error" role="alert">{coresError}</p>{/if}
  </section>

  <form class="card" onsubmit={saveTrashDays}>
    <h2>Trash</h2>
    <label>
      Delete notes in the trash forever after this many days (0 = never)
      <input type="number" min="0" max="3650" step="1" bind:value={trashDays} required />
    </label>
    <p class="muted">Checked each time the vault is unlocked.</p>
    <label class="check">
      <input type="checkbox" bind:checked={confirmTrash} onchange={saveConfirmTrash} />
      Confirm before moving notes to the trash
    </label>
    {#if confirmError}<p class="error" role="alert">{confirmError}</p>{/if}
    {#if trashError}<p class="error" role="alert">{trashError}</p>{/if}
    <div class="actions">
      {#if trashStatus}<span class="muted">{trashStatus}</span>{/if}
      <button type="submit">Save</button>
    </div>
  </form>

  <form class="card" onsubmit={changePassphrase}>
    <h2>Change passphrase</h2>
    <label>
      Current passphrase
      <input type="password" autocomplete="off" bind:value={current} disabled={changing} required />
    </label>
    <label>
      New passphrase
      <input type="password" autocomplete="off" bind:value={next} disabled={changing} required />
    </label>
    <label>
      Repeat new passphrase
      <input type="password" autocomplete="off" bind:value={confirm} disabled={changing} required />
    </label>
    <p class="muted">There is no recovery. Make sure you can remember the new passphrase.</p>
    {#if passphraseError}<p class="error" role="alert">{passphraseError}</p>{/if}
    <div class="actions">
      {#if passphraseStatus}<span class="muted">{passphraseStatus}</span>{/if}
      <button type="submit" class="primary" disabled={changing}>
        {changing ? "Changing…" : "Change passphrase"}
      </button>
    </div>
  </form>
</div>

<style>
  .settings {
    max-width: 560px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  h1 {
    margin: 0;
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  label > input {
    min-height: 36px;
    border-radius: 8px;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--text);
  }

  p {
    margin: 0;
    font-size: 13px;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
  }
</style>
