// The icons and colours a vault can have (#241). An icon is stored in the vault as two
// IDs, `{ icon, color }`; the backend checks only their form, so these lists can grow
// without a migration. An ID this version doesn't know draws the Hush mark.
import type { VaultIcon } from "$lib/api";

type Choice = { id: string; label: string };

/** Stroke icons on a 24px grid, drawn for Hush; `paths` is the SVG's inner markup. */
export const VAULT_ICONS: readonly (Choice & { paths: string })[] = [
  { id: "book", label: "Book", paths: '<path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z"/><path d="M5 17a3 3 0 0 1 3-3h11"/>' },
  { id: "notebook", label: "Notebook", paths: '<rect x="6" y="3" width="13" height="18" rx="2"/><path d="M4 7h3M4 12h3M4 17h3M11 8h4"/>' },
  { id: "pen", label: "Pen", paths: '<path d="M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4"/>' },
  { id: "archive", label: "Archive box", paths: '<rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9M10 13h4"/>' },
  { id: "briefcase", label: "Briefcase", paths: '<rect x="3" y="7" width="18" height="13" rx="2"/><path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2M3 13h18"/>' },
  { id: "chart", label: "Chart", paths: '<path d="M4 4v16h16"/><path d="M8 16v-4M12 16V8M16 16v-6"/>' },
  { id: "wallet", label: "Wallet", paths: '<path d="M17 8V5H6a2 2 0 0 0 0 4h13a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H6a2 2 0 0 1-2-2V7"/><path d="M16 14h.01"/>' },
  { id: "home", label: "Home", paths: '<path d="M4 11l8-7 8 7v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z"/><path d="M10 21v-6h4v6"/>' },
  { id: "people", label: "People", paths: '<circle cx="9" cy="8" r="3"/><path d="M3 20a6 6 0 0 1 12 0M16 5.2a3 3 0 0 1 0 5.6M18 14.5a6 6 0 0 1 3 5.5"/>' },
  { id: "heart", label: "Heart", paths: '<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>' },
  { id: "star", label: "Star", paths: '<path d="M12 3l2.7 5.6 6.1.8-4.5 4.3 1.1 6.1L12 16.9 6.6 19.8l1.1-6.1L3.2 9.4l6.1-.8z"/>' },
  { id: "lightbulb", label: "Light bulb", paths: '<path d="M9 18h6M10 21h4M12 3a6 6 0 0 0-4 10.5c.7.7 1 1.5 1 2.5h6c0-1 .3-1.8 1-2.5A6 6 0 0 0 12 3z"/>' },
  { id: "graduation", label: "Graduation cap", paths: '<path d="M2 9l10-5 10 5-10 5z"/><path d="M6 11.5V16c1.5 1.5 3.5 2.5 6 2.5s4.5-1 6-2.5v-4.5M22 9v5"/>' },
  { id: "flask", label: "Flask", paths: '<path d="M9 3h6M10 3v6l-5 9a2 2 0 0 0 1.8 3h10.4a2 2 0 0 0 1.8-3l-5-9V3M7.5 15h9"/>' },
  { id: "code", label: "Code", paths: '<path d="M8 7l-5 5 5 5M16 7l5 5-5 5M13.5 5l-3 14"/>' },
  { id: "wrench", label: "Wrench", paths: '<path d="M15 4a5 5 0 0 0-4.6 6.9L4 17.3V20h2.7l6.4-6.4A5 5 0 0 0 20 9l-3 3-3-1-1-3z"/>' },
  { id: "key", label: "Key", paths: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3M14 9l2 2"/>' },
  { id: "shield", label: "Shield", paths: '<path d="M12 3l8 3v6c0 4.5-3.2 7.8-8 9-4.8-1.2-8-4.5-8-9V6z"/>' },
  { id: "anchor", label: "Anchor", paths: '<circle cx="12" cy="5" r="2"/><path d="M12 7v14M8 11h8M4 13a8 8 0 0 0 16 0"/>' },
  { id: "ship", label: "Ship", paths: '<path d="M4 14l1.5 5h13L20 14l-8-3z"/><path d="M7 12.2V7h10v5.2M10 7V4h4v3M12 11v8"/>' },
  { id: "plane", label: "Paper plane", paths: '<path d="M21 3L3 10.5l7 2.5 2.5 7z"/><path d="M10 13l5-4"/>' },
  { id: "globe", label: "Globe", paths: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>' },
  { id: "mountain", label: "Mountain", paths: '<path d="M3 19l6-11 4 7 2-3 6 7z"/>' },
  { id: "leaf", label: "Leaf", paths: '<path d="M5 19C5 10 10 5 20 4c0 10-4 15-12 15z"/><path d="M4 21c2-5 5-8 10-11"/>' },
  { id: "camera", label: "Camera", paths: '<path d="M4 8h3l2-3h6l2 3h3a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z"/><circle cx="12" cy="13" r="3.5"/>' },
  { id: "music", label: "Music", paths: '<path d="M9 18V5l11-2v13"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>' },
];

/** Each is the CSS token `--vault-<id>` in app.css, with a light and a dark value. */
export const VAULT_COLORS: readonly Choice[] = [
  { id: "red", label: "Red" },
  { id: "orange", label: "Orange" },
  { id: "amber", label: "Amber" },
  { id: "green", label: "Green" },
  { id: "teal", label: "Teal" },
  { id: "blue", label: "Blue" },
  { id: "violet", label: "Violet" },
  { id: "pink", label: "Pink" },
];

/** The colour a first icon gets. */
export const DEFAULT_VAULT_COLOR = "blue";

export const vaultColorToken = (id: string) => `var(--vault-${id})`;

export type ResolvedVaultIcon = { label: string; paths: string; color: string };

/**
 * What to draw for a vault's icon, or null to draw the Hush mark: the vault has no
 * icon, or a newer Hush stored an icon or a colour this version doesn't have.
 */
export function resolveVaultIcon(icon: VaultIcon | null | undefined): ResolvedVaultIcon | null {
  if (!icon) return null;
  const drawing = VAULT_ICONS.find((i) => i.id === icon.icon);
  const color = VAULT_COLORS.find((c) => c.id === icon.color);
  if (!drawing || !color) return null;
  return { label: drawing.label, paths: drawing.paths, color: vaultColorToken(color.id) };
}

/**
 * Where an arrow key, Home or End moves to from choice `index` in a grid of `count`
 * choices laid out in rows of `columns`. Stays put at an edge; null for any other key.
 */
export function gridMove(index: number, key: string, count: number, columns: number): number | null {
  const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns }[key];
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (step === undefined) return null;
  const next = index + step;
  return next >= 0 && next < count ? next : index;
}
