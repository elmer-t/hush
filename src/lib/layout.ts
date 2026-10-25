// Widths of the left sidebar and the details panel. Stored per device in localStorage,
// not in the vault: they hold no note data and are needed before unlock.

export type Column = { min: number; max: number; default: number; storageKey: string };

export const LEFT: Column = { min: 200, max: 480, default: 280, storageKey: "hush.layout.leftWidth" };
export const RIGHT: Column = { min: 240, max: 520, default: 300, storageKey: "hush.layout.rightWidth" };
/** The editor column never gets narrower than this. */
export const EDITOR_MIN = 360;
/** The details panel's width while hidden: a rail with the button that brings it back. */
export const RAIL = 48;

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
  fixed = 0,
): { left: number; right: number } {
  let l = left ?? 0;
  let r = right ?? 0;
  const over = l + r + fixed + EDITOR_MIN - windowWidth;
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
