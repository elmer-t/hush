<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, LOCKED_EVENT, type LockReason, type VaultInfo } from "$lib/api";
  import Unlock from "$lib/Unlock.svelte";
  import Workspace from "$lib/Workspace.svelte";

  const lockReason = new URLSearchParams(location.search).get("locked") as LockReason | null;

  let vault = $state<VaultInfo | null>(null);
  let ready = $state(false);

  onMount(() => {
    // Reloading on lock discards every piece of frontend state that could hold note content.
    const unlisten = listen<LockReason>(LOCKED_EVENT, (e) => location.replace(`/?locked=${e.payload}`));
    api.vaultStatus().then((v) => {
      vault = v;
      ready = true;
    });
    return () => void unlisten.then((stop) => stop());
  });
</script>

{#if ready}
  {#if vault}
    <Workspace {vault} />
  {:else}
    <Unlock {lockReason} onunlock={(v) => (vault = v)} />
  {/if}
{/if}
