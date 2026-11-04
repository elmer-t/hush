// How the Tags settings list tags at 10,000 of them: most-used first, rare tags folded
// away, and a page at a time.

/** Tags on fewer notes than this go in the closed "Rare tags" group. */
export const RARE_BELOW = 2;
/** Rows rendered per page; "Show more" adds another. */
export const PAGE_SIZE = 100;

export type TagCounts = { tag: string; notes: number; description: string };

const byName = new Intl.Collator(undefined, { sensitivity: "base" });

/**
 * Splits `items` into the main list and the rare group, each sorted by note count
 * (highest first), then by name ignoring case. A tag with a description stays in the
 * main list whatever its count, since it can already be suggested. `info` gives an
 * item's counts; an item without any counts as rare.
 */
export function groupTags<T>(items: T[], info: (item: T) => TagCounts | undefined): { main: T[]; rare: T[] } {
  const keyed = items.map((item) => ({ item, info: info(item) }));
  keyed.sort(
    (a, b) =>
      (b.info?.notes ?? 0) - (a.info?.notes ?? 0) ||
      byName.compare(a.info?.tag ?? "", b.info?.tag ?? "") ||
      (a.info?.tag ?? "").localeCompare(b.info?.tag ?? ""),
  );
  const main: T[] = [];
  const rare: T[] = [];
  for (const { item, info } of keyed) {
    (info && (info.notes >= RARE_BELOW || info.description) ? main : rare).push(item);
  }
  return { main, rare };
}

/** How many of `total` rows to render after `pages` pages. */
export function shownCount(total: number, pages: number): number {
  return Math.min(total, pages * PAGE_SIZE);
}
