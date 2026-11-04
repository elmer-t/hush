<script lang="ts">
  import { tick } from "svelte";
  import type { FindSpec } from "$lib/find";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    spec: FindSpec;
    /** Whether the replace row is shown. */
    replacing: boolean;
    /** Whether replacing is possible here (not in "Preview only"). */
    canReplace: boolean;
    /** "3 of 12", "No matches", or empty before anything is typed. */
    count: string;
    onstep: (dir: 1 | -1) => void;
    onreplace: (all: boolean) => void;
    onclose: () => void;
  };
  let { spec = $bindable(), replacing = $bindable(), canReplace, count, onstep, onreplace, onclose }: Props = $props();

  let findInput = $state<HTMLInputElement>();
  let replaceInput = $state<HTMLInputElement>();

  /** Focuses and selects the find field, or the replace field. */
  export async function focus(field: "find" | "replace" = "find") {
    await tick();
    const input = field === "replace" && replacing ? replaceInput : findInput;
    input?.focus();
    input?.select();
  }

  function findKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      onstep(e.shiftKey ? -1 : 1);
    }
  }

  function replaceKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      onreplace(e.ctrlKey || e.metaKey);
    }
  }

  // Esc anywhere in the bar, a toggle or button included, closes it.
  function barKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    onclose();
  }

  const toggles = [
    { key: "caseSensitive", label: "Aa", title: "Match case" },
    { key: "wholeWord", label: "W", title: "Whole word" },
    { key: "regexp", label: ".*", title: "Regular expression" },
  ] as const;
</script>

<!-- Esc from any of its controls bubbles up here. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="find" role="search" aria-label="Find in note" onkeydown={barKeydown}>
  <div class="row">
    {#if canReplace}
      <button
        class="ghost small icon"
        aria-label="Toggle replace"
        title="Toggle replace"
        aria-expanded={replacing}
        onclick={() => (replacing = !replacing)}
      >
        <Icon name={replacing ? "chevronDown" : "chevronRight"} size={14} />
      </button>
    {/if}
    <label class="field">
      <Icon name="search" size={14} />
      <span class="visually-hidden">Find</span>
      <input
        bind:this={findInput}
        bind:value={spec.query}
        placeholder="Find in note"
        spellcheck="false"
        onkeydown={findKeydown}
      />
      <span class="count" class:none={count === "No matches" || count === "Invalid pattern"} aria-live="polite">
        {count}
      </span>
    </label>
    <div class="toggles" role="group" aria-label="Find options">
      {#each toggles as t (t.key)}
        <button
          class="toggle"
          class:mono={t.key === "regexp"}
          title={t.title}
          aria-label={t.title}
          aria-pressed={spec[t.key]}
          onclick={() => (spec[t.key] = !spec[t.key])}
        >
          {t.label}
        </button>
      {/each}
    </div>
    <button class="ghost small icon" aria-label="Previous match" title="Previous match (Shift+Enter)" onclick={() => onstep(-1)}>
      <Icon name="chevronUp" size={16} />
    </button>
    <button class="ghost small icon" aria-label="Next match" title="Next match (Enter)" onclick={() => onstep(1)}>
      <Icon name="chevronDown" size={16} />
    </button>
    <button class="ghost small icon" aria-label="Close find" title="Close (Esc)" onclick={onclose}>
      <Icon name="close" size={14} />
    </button>
  </div>
  {#if canReplace && replacing}
    <div class="row replace-row">
      <label class="field">
        <span class="visually-hidden">Replace with</span>
        <input
          bind:this={replaceInput}
          bind:value={spec.replace}
          placeholder="Replace with"
          spellcheck="false"
          onkeydown={replaceKeydown}
        />
      </label>
      <button class="small" title="Replace (Enter)" onclick={() => onreplace(false)}>Replace</button>
      <button class="small" title="Replace all (Ctrl+Enter)" onclick={() => onreplace(true)}>Replace all</button>
    </div>
  {/if}
</div>

<style>
  .find {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 24px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* Lines the replace field up under the find field, past the toggle button. */
  .replace-row {
    padding-left: 34px;
  }

  .field {
    flex: 0 1 360px;
    flex-direction: row;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--muted);
  }

  .field:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .field input {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
  }

  .count {
    flex: none;
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .count.none {
    color: var(--danger);
  }

  .toggles {
    display: inline-flex;
    gap: 2px;
  }

  .toggle {
    width: 28px;
    min-height: 28px;
    padding: 0;
    border-color: transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
    font-weight: 500;
  }

  .toggle.mono {
    font-family: var(--mono);
  }

  .toggle:hover:not(:disabled) {
    border-color: transparent;
    background: var(--hover);
    color: var(--text);
  }

  .toggle[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--accent-soft-text);
  }
</style>
