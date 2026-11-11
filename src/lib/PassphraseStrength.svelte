<script lang="ts">
  // Strength meter, warning and guidance under a new passphrase (create vault, change
  // passphrase). Advisory only: the minimum length is enforced in Rust.
  import { PASSPHRASE_GUIDANCE, passphraseStrength } from "$lib/passphrase";
  import Icon from "$lib/Icon.svelte";

  let { passphrase, id }: { passphrase: string; id?: string } = $props();

  const LABELS = { empty: "", tooShort: "Too short", weak: "Weak", fair: "Fair", strong: "Strong" };
  const FILLED = { empty: 0, tooShort: 1, weak: 1, fair: 2, strong: 3 };

  const strength = $derived(passphraseStrength(passphrase));
</script>

<div class="strength" {id} data-level={strength.level}>
  {#if strength.level !== "empty"}
    <div class="meter" aria-hidden="true">
      {#each [1, 2, 3] as n (n)}
        <span class:on={n <= FILLED[strength.level]}></span>
      {/each}
    </div>
  {/if}
  <p class="status" aria-live="polite">
    {#if strength.level !== "empty"}
      <span class="label">{LABELS[strength.level]}</span>
      {#if strength.warning}
        <span class="warning"><Icon name="warning" size={13} strokeWidth={2} />{strength.warning}</span>
      {/if}
    {/if}
  </p>
  <p class="guidance">{PASSPHRASE_GUIDANCE}</p>
</div>

<style>
  .strength {
    --level: var(--border-strong);
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }

  .strength[data-level="tooShort"],
  .strength[data-level="weak"] {
    --level: var(--danger);
  }

  .strength[data-level="fair"] {
    --level: var(--warn);
  }

  .strength[data-level="strong"] {
    --level: var(--ok);
  }

  .meter {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px;
  }

  .meter span {
    height: 4px;
    border-radius: 2px;
    background: var(--border);
  }

  .meter span.on {
    background: var(--level);
  }

  p {
    margin: 0;
  }

  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
  }

  .label {
    font-weight: 600;
    color: var(--level);
  }

  .warning {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text);
  }

  .warning :global(svg) {
    flex: none;
    color: var(--level);
  }

  .guidance {
    color: var(--text-2);
  }
</style>
