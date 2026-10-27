import { describe, expect, it } from "vitest";
import { canGo, current, emptyHistory, moveTo, push, replace, step, type NavEntry, type NavHistory } from "$lib/navHistory";

const note = (noteId: number | null): NavEntry => ({ view: "note", noteId });
const graph = (noteId: number | null): NavEntry => ({ view: "graph", noteId });
const settings: NavEntry = { view: "settings", noteId: 7 };

const pushAll = (entries: NavEntry[], h: NavHistory = emptyHistory()) => entries.reduce((h, e) => push(h, e), h);

/** Goes back or forward once, like the workspace does when the entry opens. */
function go(h: NavHistory, direction: -1 | 1, valid?: (e: NavEntry) => boolean) {
  const found = step(h, direction, valid);
  return found ? moveTo(h, found.index) : h;
}

describe("navHistory", () => {
  it("starts empty with nowhere to go", () => {
    const h = emptyHistory();
    expect(current(h)).toBeNull();
    expect(canGo(h, -1)).toBe(false);
    expect(canGo(h, 1)).toBe(false);
  });

  it("pushes entries and makes the last one current", () => {
    const h = pushAll([note(1), note(2)]);
    expect(h.entries).toEqual([note(1), note(2)]);
    expect(current(h)).toEqual(note(2));
    expect(canGo(h, -1)).toBe(true);
    expect(canGo(h, 1)).toBe(false);
  });

  it("goes back and forward through A, B, graph+B, C", () => {
    let h = pushAll([note(1), note(2), graph(2), graph(3)]);
    h = go(h, -1);
    expect(current(h)).toEqual(graph(2));
    h = go(h, -1);
    expect(current(h)).toEqual(note(2));
    h = go(h, -1);
    expect(current(h)).toEqual(note(1));
    expect(canGo(h, -1)).toBe(false);
    // Stays put at the start.
    expect(go(h, -1)).toBe(h);
    h = go(go(go(h, 1), 1), 1);
    expect(current(h)).toEqual(graph(3));
    expect(h.entries).toHaveLength(4);
  });

  it("drops the forward entries when navigating somewhere new after going back", () => {
    let h = pushAll([note(1), note(2), note(3)]);
    h = go(go(h, -1), -1);
    h = push(h, note(4));
    expect(h.entries).toEqual([note(1), note(4)]);
    expect(canGo(h, 1)).toBe(false);
  });

  it("collapses identical consecutive entries", () => {
    const h = pushAll([note(1), note(1), graph(1), graph(1), note(1)]);
    expect(h.entries).toEqual([note(1), graph(1), note(1)]);
  });

  it("keeps the forward entries when the pushed entry is the current one", () => {
    let h = go(pushAll([note(1), note(2)]), -1);
    h = push(h, note(1));
    expect(h.entries).toEqual([note(1), note(2)]);
    expect(canGo(h, 1)).toBe(true);
  });

  it("drops the oldest entries past the cap", () => {
    let h = emptyHistory();
    for (let id = 1; id <= 5; id++) h = push(h, note(id), 3);
    expect(h.entries).toEqual([note(3), note(4), note(5)]);
    expect(current(h)).toEqual(note(5));
    h = go(go(h, -1), -1);
    expect(current(h)).toEqual(note(3));
    expect(canGo(h, -1)).toBe(false);
  });

  it("treats views without a note as steps", () => {
    let h = pushAll([note(7), settings, note(7)]);
    h = go(h, -1);
    expect(current(h)).toEqual(settings);
    h = go(h, -1);
    expect(current(h)).toEqual(note(7));
  });

  it("skips entries that aren't valid any more, in the same direction", () => {
    const trashed = new Set([2, 3]);
    const valid = (e: NavEntry) => e.noteId === null || !trashed.has(e.noteId);
    let h = pushAll([note(1), note(2), graph(3), note(4)]);
    h = go(h, -1, valid);
    expect(current(h)).toEqual(note(1));
    h = go(h, 1, valid);
    expect(current(h)).toEqual(note(4));
    // Nothing valid back there: stays put.
    trashed.add(1);
    expect(step(h, -1, valid)).toBeNull();
    expect(canGo(h, -1, valid)).toBe(false);
  });

  it("skips an entry equal to the current one", () => {
    // A, B (trashed), A: back from the last A has nowhere new to go.
    const valid = (e: NavEntry) => e.noteId !== 2;
    const h = pushAll([note(1), note(2), note(1)]);
    expect(step(h, -1, valid)).toBeNull();
  });

  it("replaces the current entry without adding a step", () => {
    let h = pushAll([note(1), note(2)]);
    h = replace(h, note(null));
    expect(h.entries).toEqual([note(1), note(null)]);
    expect(current(h)).toEqual(note(null));
    // Replacing with the previous entry merges the two.
    h = replace(go(pushAll([note(1), note(2), note(3)]), -1), note(1));
    expect(h.entries).toEqual([note(1), note(3)]);
    expect(current(h)).toEqual(note(1));
    expect(replace(emptyHistory(), note(1)).entries).toEqual([note(1)]);
  });
});
