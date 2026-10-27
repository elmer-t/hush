// Browser-style back/forward history for the main area. In memory only: it is never
// stored, and the workspace clears it on lock.

export type NavView = "note" | "graph" | "settings" | "import" | "trash";

/** One step: the main area's view and, for the note and graph views, the open note. */
export interface NavEntry {
  view: NavView;
  noteId: number | null;
}

export interface NavHistory {
  entries: NavEntry[];
  /** The current entry; -1 while empty. */
  index: number;
}

export const NAV_CAP = 100;

export const emptyHistory = (): NavHistory => ({ entries: [], index: -1 });

export const sameEntry = (a: NavEntry, b: NavEntry) => a.view === b.view && a.noteId === b.noteId;

/** The current entry, or null while empty. */
export const current = (h: NavHistory): NavEntry | null => h.entries[h.index] ?? null;

/**
 * Navigating somewhere new: drops the forward entries and appends `entry`, unless it is
 * the current one. The oldest entries go once there are more than `cap`.
 */
export function push(h: NavHistory, entry: NavEntry, cap = NAV_CAP): NavHistory {
  const at = current(h);
  if (at && sameEntry(at, entry)) return h;
  const entries = [...h.entries.slice(0, h.index + 1), { ...entry }];
  const drop = Math.max(0, entries.length - cap);
  return { entries: entries.slice(drop), index: entries.length - 1 - drop };
}

/** Replaces the current entry, e.g. when the open note was trashed: that isn't a step. */
export function replace(h: NavHistory, entry: NavEntry): NavHistory {
  if (h.index < 0) return push(h, entry);
  const entries = [...h.entries];
  entries[h.index] = { ...entry };
  // The neighbours may now be the same entry.
  return collapse({ entries, index: h.index });
}

/**
 * The nearest entry in `direction` (-1 back, 1 forward) that `valid` accepts and that
 * differs from the current one, with its index; null if there is none, so it stays put.
 */
export function step(
  h: NavHistory,
  direction: -1 | 1,
  valid: (entry: NavEntry) => boolean = () => true,
): { index: number; entry: NavEntry } | null {
  const at = current(h);
  for (let i = h.index + direction; i >= 0 && i < h.entries.length; i += direction) {
    const entry = h.entries[i];
    if (at && sameEntry(at, entry)) continue;
    if (valid(entry)) return { index: i, entry };
  }
  return null;
}

/** Moves to entry `index` without adding a step (after `step` found it). */
export const moveTo = (h: NavHistory, index: number): NavHistory => ({ entries: h.entries, index });

/** Whether there is anything to go back or forward to. */
export const canGo = (h: NavHistory, direction: -1 | 1, valid?: (entry: NavEntry) => boolean) =>
  step(h, direction, valid) !== null;

/** Merges identical consecutive entries, keeping the current position on the same entry. */
function collapse(h: NavHistory): NavHistory {
  const entries: NavEntry[] = [];
  let index = h.index;
  h.entries.forEach((entry, i) => {
    const last = entries[entries.length - 1];
    if (last && sameEntry(last, entry)) {
      if (i <= h.index) index--;
    } else entries.push(entry);
  });
  return { entries, index };
}
