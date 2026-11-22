<script lang="ts">
  // A vault's icon at a given size, in the top bar and the picker (#241). Without an
  // icon, or with one this version doesn't know, it is the Hush mark.
  import type { VaultIcon } from "$lib/api";
  import { resolveVaultIcon } from "$lib/vaultIcons";
  // Same shipped Tauri icon as the unlock screen.
  import appIcon from "../../src-tauri/icons/128x128@2x.png";

  type Props = { icon: VaultIcon | null | undefined; size?: number };
  let { icon, size = 28 }: Props = $props();
  const drawn = $derived(resolveVaultIcon(icon));
</script>

{#if drawn}
  <span class="vault-icon" style:width="{size}px" style:height="{size}px" style:--vault-color={drawn.color}>
    <svg
      width={size * 0.64}
      height={size * 0.64}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {@html drawn.paths}
    </svg>
  </span>
{:else}
  <img class="mark" src={appIcon} alt="" width={size} height={size} />
{/if}

<style>
  .vault-icon {
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 22%;
    color: var(--vault-color);
    background: color-mix(in srgb, var(--vault-color) 14%, transparent);
  }

  .mark {
    flex: none;
    display: block;
  }
</style>
