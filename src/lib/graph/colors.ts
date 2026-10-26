// Node colours for the graph: a fixed palette readable on light and dark backgrounds,
// picked by a hash of the folder, tag or topic, so a value keeps its colour.
const PALETTE = [
  "#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948",
  "#b07aa1", "#ff9da7", "#9c755f", "#86bcb6", "#d37295", "#8cd17d",
];

/** The colour of nodes without the value coloured by (no tag, no topic). */
export const NONE = "#9a9d96";

export function colorOf(value: string | null | undefined): string {
  if (!value) return NONE;
  let h = 2166136261;
  for (let i = 0; i < value.length; i++) h = Math.imul(h ^ value.charCodeAt(i), 16777619);
  return PALETTE[(h >>> 0) % PALETTE.length];
}
