<script lang="ts">
  // Chooses a vault's icon and colour (#241): a grid of icons with "No icon" first, and a
  // row of colours. Each is a radio group: arrow keys, Home and End move the choice, and
  // a choice applies at once.
  import type { VaultIcon as VaultIconValue } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import VaultIcon from "$lib/VaultIcon.svelte";
  import { DEFAULT_VAULT_COLOR, gridMove, VAULT_COLORS, VAULT_ICONS, vaultColorToken } from "$lib/vaultIcons";

  type Props = { value: VaultIconValue | null; onchange: (icon: VaultIconValue | null) => void };
  let { value, onchange }: Props = $props();

  // An icon or colour a newer Hush stored is not in these lists: nothing is checked then.
  /** 0 is "No icon", -1 an icon that is not in the list. */
  const iconIndex = $derived.by(() => {
    if (!value) return 0;
    const at = VAULT_ICONS.findIndex((i) => i.id === value.icon);
    return at < 0 ? -1 : at + 1;
  });
  const colorIndex = $derived(value ? VAULT_COLORS.findIndex((c) => c.id === value.color) : -1);
  /** The colour the icons are shown in: the vault's, or the one a first icon gets. */
  const color = $derived(colorIndex >= 0 ? value!.color : DEFAULT_VAULT_COLOR);

  /** `index` 0 is "No icon". */
  function chooseIcon(index: number) {
    onchange(index === 0 ? null : { icon: VAULT_ICONS[index - 1].id, color });
  }

  function chooseColor(index: number) {
    if (value) onchange({ icon: value.icon, color: VAULT_COLORS[index].id });
  }

  /** How many choices are on the group's first row. */
  function columns(choices: HTMLElement[]): number {
    const top = choices[0].offsetTop;
    return choices.filter((c) => c.offsetTop === top).length;
  }

  function onkeydown(e: KeyboardEvent, choose: (index: number) => void) {
    const group = e.currentTarget as HTMLElement;
    const choices = [...group.querySelectorAll<HTMLElement>('[role="radio"]')];
    const at = choices.indexOf(e.target as HTMLElement);
    if (at < 0) return;
    const next = gridMove(at, e.key, choices.length, columns(choices));
    if (next === null) return;
    e.preventDefault();
    if (next === at) return;
    choices[next].focus();
    choose(next);
  }

  /** One tab stop per group: the checked choice, or the first when none is. */
  const tabindex = (index: number, checked: number) => (index === Math.max(checked, 0) ? 0 : -1);
</script>

<!-- svelte-ignore a11y_interactive_supports_focus (the radios inside take the focus) -->
<div class="icons" role="radiogroup" aria-label="Icon" onkeydown={(e) => onkeydown(e, chooseIcon)}>
  <button
    type="button"
    class="choice none"
    role="radio"
    aria-checked={iconIndex === 0}
    aria-label="No icon"
    title="No icon"
    tabindex={tabindex(0, iconIndex)}
    onclick={() => chooseIcon(0)}
  >
    <Icon name="close" size={18} />
  </button>
  {#each VAULT_ICONS as choice, i (choice.id)}
    <button
      type="button"
      class="choice"
      role="radio"
      aria-checked={iconIndex === i + 1}
      aria-label={choice.label}
      title={choice.label}
      tabindex={tabindex(i + 1, iconIndex)}
      onclick={() => chooseIcon(i + 1)}
    >
      <VaultIcon icon={{ icon: choice.id, color }} size={28} />
    </button>
  {/each}
</div>

<!-- svelte-ignore a11y_interactive_supports_focus (the radios inside take the focus) -->
<div class="colors" role="radiogroup" aria-label="Colour" onkeydown={(e) => onkeydown(e, chooseColor)}>
  {#each VAULT_COLORS as choice, i (choice.id)}
    <button
      type="button"
      class="swatch"
      role="radio"
      aria-checked={colorIndex === i}
      aria-label={choice.label}
      title={choice.label}
      tabindex={tabindex(i, colorIndex)}
      disabled={!value}
      style:--vault-color={vaultColorToken(choice.id)}
      onclick={() => chooseColor(i)}
    ></button>
  {/each}
</div>

<style>
  .icons,
  .colors {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .choice {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    min-height: 0;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: var(--muted);
  }

  .choice:hover {
    background: var(--hover);
  }

  .choice[aria-checked="true"] {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .swatch {
    width: 28px;
    height: 28px;
    min-height: 0;
    padding: 0;
    border: 2px solid var(--surface);
    border-radius: 50%;
    background: var(--vault-color);
    box-shadow: 0 0 0 1px var(--border-strong);
  }

  .swatch[aria-checked="true"] {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .swatch:disabled {
    opacity: 0.35;
  }
</style>
