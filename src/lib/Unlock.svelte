<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import {
    api,
    errorMessage,
    isApiError,
    VAULT_UPGRADE_EVENT,
    type LockReason,
    type RecentVault,
    type VaultInfo,
    type VaultUpgrade,
  } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import PassphraseStrength from "$lib/PassphraseStrength.svelte";
  // The 256 px Tauri app icon, so the unlock screen always matches the installed icon.
  import appIcon from "../../src-tauri/icons/128x128@2x.png";

  let { lockReason, onunlock }: { lockReason: LockReason | null; onunlock: (vault: VaultInfo) => void } = $props();

  const FILTERS = [{ name: "Hush vault", extensions: ["hush"] }];

  type Target = { mode: "open" | "create"; path: string; name: string };

  let recent = $state<RecentVault[]>([]);
  let target = $state<Target | null>(null);
  let passphrase = $state("");
  let confirm = $state("");
  let showPassphrase = $state(false);
  // The create form only submits once the user has said they know there is no recovery.
  let understood = $state(false);
  let busy = $state(false);
  // A schema upgrade running while the vault opens, shown after a second.
  let upgrade = $state<VaultUpgrade | null>(null);
  let showUpgrade = $state(false);
  let error = $state<{ kind: string; message: string } | null>(null);
  let passphraseInput = $state<HTMLInputElement>();

  const creating = $derived(target?.mode === "create");
  // A vault picked with "Open existing…" that isn't in the recent list yet.
  const picked = $derived(
    target && target.mode === "open" && !recent.some((v) => v.path === target!.path) ? target : null,
  );

  onMount(async () => {
    await refreshRecent();
    // Ready to type the passphrase for the most recent vault.
    const first = recent.find((v) => v.exists);
    if (first && !target) await choose("open", first.path);
  });

  async function refreshRecent() {
    recent = await api.recentVaults();
  }

  function fileName(path: string): string {
    return (path.split(/[\\/]/).pop() ?? path).replace(/\.hush$/i, "");
  }

  function folderOf(path: string): string {
    return path.slice(0, path.length - (path.split(/[\\/]/).pop() ?? "").length);
  }

  async function choose(mode: Target["mode"], path: string) {
    target = { mode, path, name: fileName(path) };
    passphrase = confirm = "";
    understood = showPassphrase = false;
    error = null;
    await tick();
    passphraseInput?.focus();
  }

  async function pickExisting() {
    const path = await open({ filters: FILTERS, multiple: false, directory: false });
    if (path) await choose("open", path);
  }

  async function pickNew() {
    const path = await save({ filters: FILTERS, defaultPath: "Notes.hush" });
    if (path) await choose("create", withExtension(path));
  }

  function withExtension(path: string): string {
    return /\.hush$/i.test(path) ? path : `${path}.hush`;
  }

  async function forget(path: string) {
    await api.forgetRecentVault(path);
    if (target?.path === path) target = null;
    await refreshRecent();
  }

  function back() {
    const first = recent.find((v) => v.exists);
    if (first) void choose("open", first.path);
    else target = null;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!target || busy) return;
    if (target.mode === "create" && passphrase !== confirm) {
      error = { kind: "mismatch", message: "The passphrases do not match." };
      return;
    }
    busy = true;
    error = null;
    upgrade = null;
    showUpgrade = false;
    const shown = setTimeout(() => (showUpgrade = true), 1000);
    const unlisten = listen<VaultUpgrade>(VAULT_UPGRADE_EVENT, (e) => (upgrade = e.payload));
    try {
      const vault =
        target.mode === "create"
          ? await api.createVault(withExtension(target.path.trim()), passphrase)
          : await api.openVault(target.path, passphrase);
      passphrase = confirm = "";
      onunlock(vault);
    } catch (err) {
      error = { kind: isApiError(err) ? err.kind : "", message: errorMessage(err) };
      if (isApiError(err) && err.kind === "wrongPassphrase") passphrase = "";
      busy = false;
      await tick();
      passphraseInput?.focus();
    } finally {
      clearTimeout(shown);
      void unlisten.then((stop) => stop());
      upgrade = null;
    }
  }
</script>

{#snippet passphraseField(id: string, label: string, strength = false)}
  <div class="field">
    <label for={id}>{label}</label>
    <div class="secret" class:invalid={error?.kind === "wrongPassphrase"}>
      <input
        {id}
        type={showPassphrase ? "text" : "password"}
        autocomplete="off"
        spellcheck="false"
        bind:this={passphraseInput}
        bind:value={passphrase}
        aria-describedby={strength ? `${id}-strength` : undefined}
        disabled={busy}
        required
      />
      <button
        type="button"
        class="ghost icon"
        aria-label={showPassphrase ? "Hide passphrase" : "Show passphrase"}
        aria-pressed={showPassphrase}
        onclick={() => (showPassphrase = !showPassphrase)}
      >
        <Icon name="eye" />
      </button>
    </div>
    {#if strength}
      <PassphraseStrength {passphrase} id="{id}-strength" />
    {/if}
  </div>
{/snippet}

{#snippet problem()}
  {#if error?.kind === "vaultInUse"}
    <p class="callout" role="alert">
      <Icon name="warning" strokeWidth={2} />
      <span>{target?.name}.hush is already open in another Hush window. Switch to that window, or lock it there first.</span>
    </p>
  {:else if error}
    <p class="error inline" role="alert">
      <Icon name="alert" size={14} strokeWidth={2} />
      {error.kind === "wrongPassphrase" ? `Wrong passphrase for ${target?.name}.hush.` : error.message}
    </p>
  {/if}
{/snippet}

<main>
  <h1 class="brand">
    <img src={appIcon} alt="Hush" width="96" height="96" />
  </h1>

  {#if creating && target}
    <form class="card" onsubmit={submit}>
      <div class="intro">
        <h2>Create a new vault</h2>
        <p>One encrypted file holds every note in the vault.</p>
      </div>

      <div class="field">
        <label for="vault-path">Save as</label>
        <div class="row">
          <input id="vault-path" class="mono" spellcheck="false" bind:value={target.path} disabled={busy} required />
          <button type="button" onclick={pickNew} disabled={busy}>Choose…</button>
        </div>
      </div>

      {@render passphraseField("new-passphrase", "Passphrase", true)}

      <div class="field">
        <label for="repeat-passphrase">Repeat passphrase</label>
        <input
          id="repeat-passphrase"
          class="tall"
          type={showPassphrase ? "text" : "password"}
          autocomplete="off"
          bind:value={confirm}
          disabled={busy}
          required
        />
      </div>

      <div class="no-recovery">
        <p>
          <Icon name="warning" size={18} strokeWidth={2} />
          <span>
            <strong>There is no recovery.</strong> If you forget this passphrase, nobody can open this vault again, including
            you. Hush never stores it.
          </span>
        </p>
        <label class="check">
          <input type="checkbox" bind:checked={understood} disabled={busy} />
          I understand the passphrase cannot be recovered
        </label>
      </div>

      {@render problem()}

      <button type="submit" class="primary submit" disabled={busy || !passphrase || !understood}>
        {busy ? "Creating…" : "Create vault"}
      </button>
      <button type="button" class="link back" onclick={back} disabled={busy}>Open an existing vault instead</button>
    </form>
  {:else}
    <form class="card" onsubmit={submit}>
      <div class="intro">
        <h2>Unlock a vault</h2>
        <p>{recent.length > 0 || picked ? "Recent vaults on this computer." : "Open a vault file, or create a new one."}</p>
      </div>

      {#if lockReason === "idle"}
        <p class="callout">
          <Icon name="clock" strokeWidth={2} />
          <span>The vault was locked after a period of inactivity.</span>
        </p>
      {/if}

      {#if recent.length > 0 || picked}
        <div class="vaults" role="radiogroup" aria-label="Recent vaults">
          {#if picked}
            {@render vaultRow(picked.path, picked.name, true, false)}
          {/if}
          {#each recent as vault (vault.path)}
            {@render vaultRow(vault.path, vault.name, vault.exists, true)}
          {/each}
        </div>
      {/if}

      {#if target}
        {@render passphraseField("passphrase", "Passphrase")}
        {@render problem()}
        <button type="submit" class="primary submit" disabled={busy || !passphrase}>
          {busy ? "Unlocking…" : "Unlock"}
        </button>
        {#if busy && showUpgrade && upgrade && upgrade.total > 0}
          <p class="muted upgrade" role="status">
            {upgrade.step === "folderSizes" ? "Measuring note sizes for the folder tree" : "Upgrading the search index"}:
            {Math.floor((upgrade.done / upgrade.total) * 100)}% ({upgrade.done.toLocaleString()}
            of {upgrade.total.toLocaleString()} notes). This happens once.
          </p>
        {/if}
      {/if}

      <div class="other">
        <button type="button" onclick={pickExisting} disabled={busy}>
          <Icon name="folder" />
          Open existing…
        </button>
        <button type="button" onclick={pickNew} disabled={busy}>
          <Icon name="plus" strokeWidth={2} />
          Create new…
        </button>
      </div>
    </form>
  {/if}

  <p class="footnote">Encrypted with SQLCipher. Notes never leave this computer.</p>
</main>

{#snippet vaultRow(path: string, name: string, exists: boolean, forgettable: boolean)}
  {@const checked = target?.path === path}
  <div class="vault-row">
    <button
      type="button"
      role="radio"
      class="vault"
      class:checked
      aria-checked={checked}
      disabled={!exists || busy}
      title={path}
      onclick={() => choose("open", path)}
    >
      <Icon name="lock" size={18} />
      <span class="vault-text">
        <span class="vault-name">{name}.hush</span>
        <span class="vault-path">{exists ? folderOf(path) : `Not found: ${path}`}</span>
      </span>
    </button>
    {#if forgettable}
      <button
        type="button"
        class="ghost small icon forget"
        title="Remove from list"
        aria-label="Remove {name} from list"
        onclick={() => forget(path)}
      >
        <Icon name="close" size={14} strokeWidth={2} />
      </button>
    {/if}
  </div>
{/snippet}

<style>
  main {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 28px;
    padding: 48px 16px;
  }

  .brand {
    margin: 0;
    line-height: 0;
  }

  .brand img {
    display: block;
    width: 96px;
    height: 96px;
  }

  .card {
    width: 100%;
    max-width: 440px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 28px;
    border-radius: 14px;
    box-shadow: var(--shadow);
  }

  .card:has(.no-recovery) {
    max-width: 480px;
    gap: 18px;
  }

  .intro {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }

  .intro p {
    margin: 0;
    color: var(--text-2);
  }

  .vaults {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .vault-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .vault {
    flex: 1;
    min-width: 0;
    justify-content: flex-start;
    gap: 12px;
    padding: 10px 12px;
    border-radius: 10px;
    background: transparent;
    text-align: left;
    white-space: normal;
  }

  .vault :global(svg) {
    color: var(--muted);
  }

  .vault.checked {
    border-color: var(--accent);
    background: var(--selection);
  }

  .vault.checked :global(svg) {
    color: var(--accent);
  }

  .vault-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .vault-name {
    font-weight: 500;
  }

  .vault-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .forget {
    color: var(--muted);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field > label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
  }

  .secret {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 40px;
    padding: 0 4px 0 12px;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    background: var(--surface);
  }

  .secret:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .secret.invalid {
    border-color: var(--danger);
  }

  .secret input {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    font-size: 15px;
    letter-spacing: 0.1em;
  }

  input.tall,
  .row input {
    min-height: 38px;
    padding: 0 12px;
    border-radius: 8px;
  }

  input.tall {
    letter-spacing: 0.1em;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row input {
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }

  .row button {
    min-height: 38px;
    padding: 0 14px;
    background: transparent;
  }

  .inline {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }

  .no-recovery {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: 10px;
    background: var(--danger-soft);
  }

  .no-recovery p {
    margin: 0;
    display: flex;
    gap: 8px;
  }

  .no-recovery p :global(svg) {
    margin-top: 1px;
    color: var(--danger);
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding-left: 26px;
    font-size: 13px;
    color: var(--text);
  }

  .submit {
    min-height: 42px;
    font-size: 15px;
  }

  .back {
    align-self: center;
    font-size: 13px;
    text-decoration: underline;
  }

  .other {
    display: flex;
    gap: 8px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }

  .other button {
    flex: 1;
    min-height: 36px;
    background: transparent;
  }

  .footnote {
    margin: 0;
    max-width: 440px;
    text-align: center;
    font-size: 12px;
    color: var(--muted);
  }
  .upgrade {
    margin: 8px 0 0;
    font-size: 0.9em;
  }
</style>
