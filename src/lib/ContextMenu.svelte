<script lang="ts">
  import { tick } from "svelte";
  import Icon, { type IconName } from "$lib/Icon.svelte";

  export type MenuItem = { label: string; icon: IconName; action: () => void; danger?: boolean };

  type Props = {
    /** Viewport coordinates the menu opens at; it is kept on screen. */
    x: number;
    y: number;
    label: string;
    items: MenuItem[];
    /** `refocus` is true when closed with Escape, so focus can go back where it was. */
    onclose: (refocus: boolean) => void;
  };
  let { x, y, label, items, onclose }: Props = $props();

  let menu = $state<HTMLElement>();
  let left = $state(0);
  let top = $state(0);
  // Hidden until measured, so it never flashes at the corner.
  let placed = $state(false);

  // Place at the pointer, flipped back inside the window when it would overflow.
  $effect(() => {
    const el = menu;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    left = Math.max(4, Math.min(x, window.innerWidth - width - 4));
    top = Math.max(4, Math.min(y, window.innerHeight - height - 4));
    placed = true;
    void tick().then(() => buttons()[0]?.focus());
  });

  function buttons(): HTMLButtonElement[] {
    return [...(menu?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
  }

  function onkeydown(e: KeyboardEvent) {
    const list = buttons();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    let handled = true;
    switch (e.key) {
      case "ArrowDown":
        list[(i + 1) % list.length]?.focus();
        break;
      case "ArrowUp":
        list[(i - 1 + list.length) % list.length]?.focus();
        break;
      case "Home":
        list[0]?.focus();
        break;
      case "End":
        list[list.length - 1]?.focus();
        break;
      case "Escape":
        onclose(true);
        break;
      case "Tab":
        onclose(false);
        break;
      default:
        handled = false;
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function run(item: MenuItem) {
    onclose(false);
    item.action();
  }

  function outside(e: Event) {
    if (!menu?.contains(e.target as Node)) onclose(false);
  }
</script>

<svelte:window onpointerdowncapture={outside} onblur={() => onclose(false)} onresize={() => onclose(false)} />
<svelte:document onscrollcapture={() => onclose(false)} />

<div
  class="context-menu"
  role="menu"
  tabindex="-1"
  aria-label={label}
  bind:this={menu}
  style:left="{left}px"
  style:top="{top}px"
  style:visibility={placed ? "visible" : "hidden"}
  {onkeydown}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#each items as item (item.label)}
    <button role="menuitem" class="ghost" class:danger={item.danger} tabindex="-1" onclick={() => run(item)}>
      <Icon name={item.icon} size={14} />
      {item.label}
    </button>
  {/each}
</div>

<style>
  .context-menu {
    position: fixed;
    z-index: 50;
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--raised);
    box-shadow: var(--shadow);
  }

  button {
    justify-content: flex-start;
    color: var(--text);
    font-size: 13px;
  }

  button.danger {
    color: var(--danger);
  }

  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
