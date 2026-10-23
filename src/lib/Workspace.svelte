<script lang="ts">
  import { api, type VaultInfo } from "$lib/api";
  import Settings from "$lib/Settings.svelte";

  let { vault }: { vault: VaultInfo } = $props();

  // Auto-lock is enforced by the backend; activity is reported at most this often.
  const TOUCH_INTERVAL_MS = 15_000;

  let showSettings = $state(false);
  let lastTouch = 0;

  function activity() {
    const now = Date.now();
    if (now - lastTouch < TOUCH_INTERVAL_MS) return;
    lastTouch = now;
    void api.touch();
  }

  function onkeydown(e: KeyboardEvent) {
    activity();
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l") {
      e.preventDefault();
      void api.lockVault();
    }
  }
</script>

<svelte:window {onkeydown} onpointerdown={activity} onpointermove={activity} onwheel={activity} />

<div class="workspace">
  <header>
    <strong title={vault.path}>{vault.name}</strong>
    <div class="actions">
      <button onclick={() => (showSettings = !showSettings)} aria-pressed={showSettings}>Settings</button>
      <button onclick={() => api.lockVault()} title="Lock (Ctrl+L)">Lock</button>
    </div>
  </header>

  <main>
    {#if showSettings}
      <Settings />
    {:else}
      <p class="muted empty">No notes yet.</p>
    {/if}
  </main>
</div>

<style>
  .workspace {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .actions {
    display: flex;
    gap: 0.5rem;
  }

  main {
    overflow: auto;
    padding: 1.5rem 16px;
  }

  .empty {
    text-align: center;
    margin-top: 20vh;
  }
</style>
