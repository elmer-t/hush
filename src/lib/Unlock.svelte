<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, isApiError, type LockReason, type RecentVault, type VaultInfo } from "$lib/api";

  let { lockReason, onunlock }: { lockReason: LockReason | null; onunlock: (vault: VaultInfo) => void } = $props();

  const FILTERS = [{ name: "Hush vault", extensions: ["hush"] }];

  type Target = { mode: "open" | "create"; path: string; name: string };

  let recent = $state<RecentVault[]>([]);
  let target = $state<Target | null>(null);
  let passphrase = $state("");
  let confirm = $state("");
  let busy = $state(false);
  let error = $state("");
  let passphraseInput = $state<HTMLInputElement>();

  onMount(refreshRecent);

  async function refreshRecent() {
    recent = await api.recentVaults();
  }

  function fileName(path: string): string {
    return (path.split(/[\\/]/).pop() ?? path).replace(/\.hush$/i, "");
  }

  async function choose(mode: Target["mode"], path: string) {
    target = { mode, path, name: fileName(path) };
    passphrase = confirm = error = "";
    await tick();
    passphraseInput?.focus();
  }

  async function pickExisting() {
    const path = await open({ filters: FILTERS, multiple: false, directory: false });
    if (path) await choose("open", path);
  }

  async function pickNew() {
    const path = await save({ filters: FILTERS, defaultPath: "Notes.hush" });
    if (path) await choose("create", /\.hush$/i.test(path) ? path : `${path}.hush`);
  }

  async function forget(path: string) {
    await api.forgetRecentVault(path);
    await refreshRecent();
  }

  function back() {
    target = null;
    passphrase = confirm = error = "";
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!target || busy) return;
    if (target.mode === "create" && passphrase !== confirm) {
      error = "The passphrases do not match.";
      return;
    }
    busy = true;
    error = "";
    try {
      const vault =
        target.mode === "create"
          ? await api.createVault(target.path, passphrase)
          : await api.openVault(target.path, passphrase);
      passphrase = confirm = "";
      onunlock(vault);
    } catch (err) {
      error = errorMessage(err);
      if (isApiError(err) && err.kind === "wrongPassphrase") passphrase = "";
      busy = false;
      await tick();
      passphraseInput?.focus();
    }
  }
</script>

<main>
  <header>
    <h1>Hush</h1>
    <p class="muted">Private notes in an encrypted vault.</p>
  </header>

  {#if lockReason === "idle" && !target}
    <p class="notice">The vault was locked after a period of inactivity.</p>
  {/if}

  {#if target}
    <form onsubmit={submit}>
      <div>
        <h2>{target.mode === "create" ? "Create" : "Unlock"} {target.name}</h2>
        <p class="path muted" title={target.path}>{target.path}</p>
      </div>

      <label>
        Passphrase
        <input
          type="password"
          autocomplete="off"
          bind:this={passphraseInput}
          bind:value={passphrase}
          disabled={busy}
          required
        />
      </label>

      {#if target.mode === "create"}
        <label>
          Repeat passphrase
          <input type="password" autocomplete="off" bind:value={confirm} disabled={busy} required />
        </label>
        <p class="warning">
          There is no way to recover a forgotten passphrase. If you lose it, the notes in this vault are gone
          for good.
        </p>
      {/if}

      {#if error}<p class="error" role="alert">{error}</p>{/if}

      <div class="actions">
        <button type="button" onclick={back} disabled={busy}>Back</button>
        <button type="submit" class="primary" disabled={busy || !passphrase}>
          {#if busy}
            {target.mode === "create" ? "Creating…" : "Unlocking…"}
          {:else}
            {target.mode === "create" ? "Create vault" : "Unlock"}
          {/if}
        </button>
      </div>
    </form>
  {:else}
    {#if recent.length > 0}
      <section>
        <h2>Recent vaults</h2>
        <ul>
          {#each recent as vault (vault.path)}
            <li>
              <button class="vault" disabled={!vault.exists} onclick={() => choose("open", vault.path)}>
                <span>{vault.name}</span>
                <span class="path muted" title={vault.path}>
                  {vault.exists ? vault.path : `Not found: ${vault.path}`}
                </span>
              </button>
              <button class="forget" title="Remove from list" aria-label="Remove {vault.name} from list" onclick={() => forget(vault.path)}>
                ×
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <div class="actions">
      <button onclick={pickExisting}>Open existing…</button>
      <button class="primary" onclick={pickNew}>Create new…</button>
    </div>
  {/if}
</main>

<style>
  main {
    max-width: 440px;
    margin: 12vh auto 0;
    padding: 0 16px;
    display: grid;
    gap: 1.5rem;
  }

  h1 {
    margin: 0;
    font-size: 1.8rem;
  }

  h2 {
    margin: 0 0 0.4rem;
    font-size: 1rem;
  }

  header p {
    margin: 0.2rem 0 0;
  }

  form {
    display: grid;
    gap: 1rem;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.4rem;
  }

  li {
    display: flex;
    gap: 0.4rem;
  }

  .vault {
    flex: 1;
    min-width: 0;
    display: grid;
    text-align: left;
  }

  .forget {
    padding: 0 0.7em;
    color: var(--muted);
  }

  .path {
    margin: 0;
    font-size: 0.85em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .notice,
  .warning {
    margin: 0;
    padding: 0.6em 0.8em;
    border-radius: 6px;
    border: 1px solid var(--border);
  }

  .warning {
    background: var(--warning-bg);
    border-color: var(--warning-border);
  }
</style>
