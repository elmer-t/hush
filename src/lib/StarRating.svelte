<script lang="ts">
  import { tick } from "svelte";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    /** 1–5, or null when unrated. */
    value: number | null;
    /** Called with the new rating; null clears it. */
    onchange: (rating: number | null) => void;
  };
  let { value, onchange }: Props = $props();

  const STARS = [1, 2, 3, 4, 5];

  // The rating as last set here, ahead of the save that brings `value` back.
  let current = $state<number | null>(null);
  $effect.pre(() => {
    current = value;
  });
  // The star under the pointer: the stars preview up to it without saving.
  let hover = $state<number | null>(null);
  let group = $state<HTMLElement>();

  const shown = $derived(hover ?? current ?? 0);
  // Roving tabindex: Tab reaches the checked star, or the first when unrated.
  const tabStop = $derived(current ?? 1);

  async function set(rating: number | null, focus = false) {
    if (rating === current) return;
    current = rating;
    onchange(rating);
    if (!focus) return;
    await tick();
    group?.querySelector<HTMLElement>(`[data-star="${rating ?? 1}"]`)?.focus();
  }

  function click(n: number) {
    // Clicking the current rating's star again clears it.
    void set(current === n ? null : n);
  }

  function keydown(e: KeyboardEvent) {
    const r = current ?? 0;
    // Keys show the real value, not a preview under a resting pointer.
    hover = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") void set(Math.min(5, r + 1), true);
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") void set(Math.max(1, r - 1), true);
    else if (e.key === "Home") void set(1, true);
    else if (e.key === "End") void set(5, true);
    else if (/^[1-5]$/.test(e.key)) void set(Number(e.key), true);
    else if (e.key === "0" || e.key === "Delete" || e.key === "Backspace") void set(null, true);
    else return;
    e.preventDefault();
  }

  const starLabel = (n: number) => `${n} ${n === 1 ? "star" : "stars"}`;
</script>

<div class="rating">
  <div
    class="stars"
    role="radiogroup"
    aria-label={current ? `Rating: ${current} of 5` : "Rating: none"}
    tabindex="-1"
    bind:this={group}
    onkeydown={keydown}
    onpointerleave={() => (hover = null)}
  >
    {#each STARS as n (n)}
      <button
        type="button"
        class="star"
        class:on={n <= shown}
        class:preview={hover !== null}
        role="radio"
        aria-checked={current === n}
        aria-label={starLabel(n)}
        title={current === n ? `${starLabel(n)}: click to clear` : starLabel(n)}
        tabindex={n === tabStop ? 0 : -1}
        data-star={n}
        onclick={() => click(n)}
        onpointerenter={() => (hover = n)}
      >
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
          <path d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z" />
        </svg>
      </button>
    {/each}
  </div>
  {#if current !== null}
    <button type="button" class="ghost small icon clear" aria-label="Clear rating" title="Clear rating" onclick={() => set(null)}>
      <Icon name="close" size={12} strokeWidth={2} />
    </button>
  {/if}
</div>

<style>
  .rating {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 30px;
  }

  .stars {
    display: flex;
    gap: 0;
  }

  .star {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }

  .star path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linejoin: round;
  }

  .star.on {
    color: var(--accent);
  }

  .star.on path {
    fill: currentColor;
  }

  .star.on.preview path {
    fill-opacity: 0.55;
  }

  .star:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .clear {
    color: var(--muted);
  }
</style>
