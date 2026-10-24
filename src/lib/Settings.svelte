<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage } from "$lib/api";

  let minutes = $state(10);
  let minutesStatus = $state("");
  let minutesError = $state("");

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let changing = $state(false);
  let passphraseStatus = $state("");
  let passphraseError = $state("");

  let trashDays = $state(30);
  let trashStatus = $state("");
  let trashError = $state("");

  let author = $state("");
  let authorStatus = $state("");
  let authorError = $state("");

  onMount(async () => {
    [minutes, author, trashDays] = await Promise.all([
      api.autoLockMinutes(),
      api.defaultAuthor().then((a) => a ?? ""),
      api.trashDays(),
    ]);
  });

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
  <form onsubmit={saveAuthor}>
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

  <form onsubmit={saveMinutes}>
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

  <form onsubmit={saveTrashDays}>
    <h2>Trash</h2>
    <label>
      Delete notes in the trash forever after this many days (0 = never)
      <input type="number" min="0" max="3650" step="1" bind:value={trashDays} required />
    </label>
    <p class="muted">Checked each time the vault is unlocked.</p>
    {#if trashError}<p class="error" role="alert">{trashError}</p>{/if}
    <div class="actions">
      {#if trashStatus}<span class="muted">{trashStatus}</span>{/if}
      <button type="submit">Save</button>
    </div>
  </form>

  <form onsubmit={changePassphrase}>
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
    max-width: 440px;
    margin: 0 auto;
    display: grid;
    gap: 2rem;
  }

  form {
    display: grid;
    gap: 0.8rem;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  p {
    margin: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.75rem;
  }
</style>
