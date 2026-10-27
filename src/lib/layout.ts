// The app layout that holds no note data: column widths, which panels are shown, the
// editor mode, the main view and the graph's display controls. Stored per device in
// localStorage, not in the vault: it is needed before unlock (the first render after it),
// and a vault opened on a laptop and a desktop keeps a layout that suits each screen.
// Anything that names note content (note ids, folders, tags) goes in the vault instead;
// see `commands/layout.rs`.

export type Column = { min: number; max: number; default: number; storageKey: string };

export const LEFT: Column = { min: 200, max: 480, default: 280, storageKey: "hush.layout.leftWidth" };
export const RIGHT: Column = { min: 240, max: 520, default: 300, storageKey: "hush.layout.rightWidth" };
/** The editor column never gets narrower than this. */
export const EDITOR_MIN = 360;

/** The Activity panel's height; `loadWidth` and `saveWidth` store it like a width. */
export const ACTIVITY: Column = { min: 120, max: 900, default: 300, storageKey: "hush.layout.activityHeight" };
/** The header and editor above the Activity panel never get shorter than this together. */
export const ABOVE_ACTIVITY_MIN = 240;

/**
 * The Activity panel height to show: the stored height, less what a short window needs
 * for the header and editor, but never below the panel's minimum. The stored height is
 * left alone.
 */
export const fitHeight = (windowHeight: number, height: number) =>
  Math.round(Math.max(ACTIVITY.min, Math.min(height, windowHeight - ABOVE_ACTIVITY_MIN)));

/** The Activity panel's split: the share of its width the Jobs side gets. */
export const SPLIT = { default: 0.6, jobsMin: 320, logMin: 260 } as const;

export const isSplit = (value: unknown): value is number =>
  typeof value === "number" && value > 0 && value < 1;

/**
 * The Jobs side's width in an Activity panel `width` wide, from the stored share. The
 * Event log keeps its minimum first, as it did before the split could be dragged; the
 * Jobs side then keeps its own if there is room.
 */
export const jobsWidth = (width: number, share: number) =>
  Math.round(Math.max(0, Math.min(Math.max(width * share, SPLIT.jobsMin), width - SPLIT.logMin)));

export const clamp = (width: number, column: Column) =>
  Math.round(Math.min(column.max, Math.max(column.min, width)));

export function loadWidth(column: Column): number {
  try {
    const stored = Number(localStorage.getItem(column.storageKey));
    return stored ? clamp(stored, column) : column.default;
  } catch {
    return column.default;
  }
}

export function saveWidth(column: Column, width: number) {
  try {
    localStorage.setItem(column.storageKey, String(width));
  } catch {
    // Storage can be unavailable; the width then lasts until the window closes.
  }
}

/**
 * The widths to show when the window is too narrow for both stored widths and the
 * editor: the shown side columns give up the difference in proportion to how far each
 * is above its minimum. The stored widths are left alone. `null` is a hidden column.
 */
export function fitWidths(
  windowWidth: number,
  left: number | null,
  right: number | null,
): { left: number; right: number } {
  let l = left ?? 0;
  let r = right ?? 0;
  const over = l + r + EDITOR_MIN - windowWidth;
  if (over > 0) {
    const roomL = left === null ? 0 : l - LEFT.min;
    const roomR = right === null ? 0 : r - RIGHT.min;
    const room = roomL + roomR;
    if (room > 0) {
      const take = Math.min(over, room);
      l -= (take * roomL) / room;
      r -= (take * roomR) / room;
    }
  }
  return { left: Math.round(l), right: Math.round(r) };
}

/** localStorage keys of the layout values other than the widths. */
export const KEYS = {
  previewOnly: "hush.layout.previewOnly",
  showLeft: "hush.layout.showLeft",
  showRight: "hush.layout.showRight",
  showInternals: "hush.layout.showInternals",
  showActivity: "hush.layout.showActivity",
  activitySplit: "hush.layout.activitySplit",
  view: "hush.layout.view",
  graph: "hush.graph.display",
} as const;

export const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";

export const oneOf =
  <T extends string>(...values: T[]) =>
  (value: unknown): value is T =>
    values.includes(value as T);

/**
 * A stored layout value, or `fallback` when storage is unavailable, holds nothing, or
 * holds something that isn't valid JSON or that `valid` rejects.
 */
export function loadStored<T>(key: string, fallback: T, valid: (value: unknown) => value is T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const value: unknown = JSON.parse(raw);
    return valid(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

export function saveStored(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Storage can be unavailable; the value then lasts until the window closes.
  }
}

/**
 * `run` at most once per `ms`, with the last value given: for drags and sliders.
 * `flush` runs a pending call now and returns what it returned; `cancel` drops it.
 */
export function debounced<T>(ms: number, run: (value: T) => unknown) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: { value: T } | null = null;
  const flush = () => {
    clearTimeout(timer);
    const p = pending;
    pending = null;
    return p ? run(p.value) : undefined;
  };
  return {
    call(value: T) {
      pending = { value };
      clearTimeout(timer);
      timer = setTimeout(flush, ms);
    },
    flush,
    cancel() {
      clearTimeout(timer);
      pending = null;
    },
  };
}
