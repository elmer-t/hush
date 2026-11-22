// Where the user was in a note: the selection and the line at the top of the editor, so a
// note opens again where it was left (#242). Positions are document offsets, not pixels:
// pixels don't survive a mode switch, another width or the editor's estimated heights.
//
// The vault keeps them for the recently opened notes (`db/positions.rs`). This module keeps
// them for the session too, in memory only: the editor is rebuilt on every note and view
// switch, and with history off the vault stores none. Locking reloads the page, which
// discards the memory. Nothing here touches localStorage.

/** `anchor` and `head` are the selection's ends; `top` is the offset at the top left of what is shown. */
export type NotePosition = { anchor: number; head: number; top: number };

function clampOffset(value: unknown, length: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return 0;
  return Math.min(Math.floor(value), length);
}

/** `position` with every offset inside a note of `length`: the body can have changed since. */
export function clampPosition(position: NotePosition, length: number): NotePosition {
  return {
    anchor: clampOffset(position.anchor, length),
    head: clampOffset(position.head, length),
    top: clampOffset(position.top, length),
  };
}

export function samePosition(a: NotePosition | null, b: NotePosition | null): boolean {
  if (!a || !b) return a === b;
  return a.anchor === b.anchor && a.head === b.head && a.top === b.top;
}

/** As many notes as the recently opened list can hold. */
const MEMORY_LIMIT = 1000;

/** The session's positions by note id; the notes used last are kept when it is full. */
export class PositionMemory {
  #positions = new Map<number, NotePosition>();
  #enabled = true;
  #limit: number;

  constructor(limit = MEMORY_LIMIT) {
    this.#limit = limit;
  }

  get enabled(): boolean {
    return this.#enabled;
  }

  /** The setting "Reopen notes where you left off"; off forgets everything. */
  setEnabled(on: boolean) {
    this.#enabled = on;
    if (!on) this.#positions.clear();
  }

  get(id: number): NotePosition | null {
    return this.#positions.get(id) ?? null;
  }

  set(id: number, position: NotePosition) {
    if (!this.#enabled) return;
    // A Map keeps insertion order, so the first key is the one used longest ago.
    this.#positions.delete(id);
    this.#positions.set(id, position);
    if (this.#positions.size > this.#limit) this.#positions.delete(this.#positions.keys().next().value!);
  }
}

/** The positions of this session, shared by every editor. */
export const notePositions = new PositionMemory();

/**
 * Where a note's editor starts, or null for the top as before. This session's position is
 * newer than the vault's. A new note starts at the top: its title gets the focus.
 * Anything that opens a note at a place of its own (a find match, a heading link) must
 * not ask this: that place wins.
 */
export function startPosition(open: {
  isNew: boolean;
  remembered: NotePosition | null;
  stored: NotePosition | null | undefined;
  length: number;
}): NotePosition | null {
  if (open.isNew) return null;
  const position = open.remembered ?? open.stored ?? null;
  return position && clampPosition(position, open.length);
}
