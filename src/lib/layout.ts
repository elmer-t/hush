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
