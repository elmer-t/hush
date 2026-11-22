import { describe, expect, it } from "vitest";
import { clampPosition, PositionMemory, samePosition, startPosition } from "$lib/notePositions";

describe("clampPosition", () => {
  it("keeps offsets inside the note", () => {
    expect(clampPosition({ anchor: 10, head: 20, top: 5 }, 100)).toEqual({ anchor: 10, head: 20, top: 5 });
    expect(clampPosition({ anchor: 0, head: 100, top: 100 }, 100)).toEqual({ anchor: 0, head: 100, top: 100 });
  });

  it("pulls offsets past the end back to the end, for a note that got shorter", () => {
    expect(clampPosition({ anchor: 150, head: 90, top: 400 }, 100)).toEqual({ anchor: 100, head: 90, top: 100 });
    expect(clampPosition({ anchor: 5, head: 7, top: 3 }, 0)).toEqual({ anchor: 0, head: 0, top: 0 });
  });

  it("reads anything that is not a whole offset as the start", () => {
    const odd = { anchor: -4, head: Number.NaN, top: 2.7 };
    expect(clampPosition(odd, 100)).toEqual({ anchor: 0, head: 0, top: 2 });
    expect(clampPosition({ anchor: "12", head: null, top: undefined } as never, 100)).toEqual({ anchor: 0, head: 0, top: 0 });
    expect(clampPosition({ anchor: Infinity, head: 1, top: 1 }, 100)).toEqual({ anchor: 0, head: 1, top: 1 });
  });
});

describe("samePosition", () => {
  it("compares the three offsets", () => {
    expect(samePosition({ anchor: 1, head: 2, top: 3 }, { anchor: 1, head: 2, top: 3 })).toBe(true);
    expect(samePosition({ anchor: 1, head: 2, top: 3 }, { anchor: 1, head: 2, top: 4 })).toBe(false);
    expect(samePosition(null, { anchor: 0, head: 0, top: 0 })).toBe(false);
    expect(samePosition(null, null)).toBe(true);
  });
});

describe("PositionMemory", () => {
  const at = (n: number) => ({ anchor: n, head: n, top: n });

  it("remembers the last position of each note", () => {
    const memory = new PositionMemory();
    expect(memory.get(1)).toBeNull();
    memory.set(1, at(10));
    memory.set(2, at(20));
    memory.set(1, at(11));
    expect(memory.get(1)).toEqual(at(11));
    expect(memory.get(2)).toEqual(at(20));
  });

  it("the off switch forgets everything and remembers nothing", () => {
    const memory = new PositionMemory();
    memory.set(1, at(10));
    memory.setEnabled(false);
    expect(memory.enabled).toBe(false);
    expect(memory.get(1)).toBeNull();
    memory.set(1, at(12));
    expect(memory.get(1)).toBeNull();
    memory.setEnabled(true);
    expect(memory.get(1)).toBeNull();
    memory.set(1, at(13));
    expect(memory.get(1)).toEqual(at(13));
  });

  it("turning it on while it is on keeps what it has", () => {
    const memory = new PositionMemory();
    memory.set(1, at(10));
    memory.setEnabled(true);
    expect(memory.get(1)).toEqual(at(10));
  });

  it("forget drops one note", () => {
    const memory = new PositionMemory();
    memory.set(1, at(10));
    memory.set(2, at(20));
    memory.forget(1);
    expect(memory.get(1)).toBeNull();
    expect(memory.get(2)).toEqual(at(20));
  });

  it("clear forgets every position and keeps remembering", () => {
    const memory = new PositionMemory();
    memory.set(1, at(10));
    memory.clear();
    expect(memory.get(1)).toBeNull();
    expect(memory.enabled).toBe(true);
    memory.set(1, at(11));
    expect(memory.get(1)).toEqual(at(11));
  });

  it("keeps the notes used last when it is full", () => {
    const memory = new PositionMemory(3);
    for (const id of [1, 2, 3]) memory.set(id, at(id));
    memory.set(1, at(100)); // 1 is now the most recent
    memory.set(4, at(4));
    expect(memory.get(2)).toBeNull();
    expect(memory.get(1)).toEqual(at(100));
    expect(memory.get(3)).toEqual(at(3));
    expect(memory.get(4)).toEqual(at(4));
  });
});

describe("startPosition", () => {
  const session = { anchor: 50, head: 60, top: 40 };
  const vault = { anchor: 5, head: 6, top: 4 };

  it("prefers this session's position over the vault's, which is older", () => {
    expect(startPosition({ isNew: false, remembered: session, stored: vault, length: 100 })).toEqual(session);
  });

  it("uses the vault's after an unlock or a restart", () => {
    expect(startPosition({ isNew: false, remembered: null, stored: vault, length: 100 })).toEqual(vault);
  });

  it("is null when nothing is remembered: the note opens at the top as before", () => {
    expect(startPosition({ isNew: false, remembered: null, stored: null, length: 100 })).toBeNull();
  });

  it("is null for a new note, whose title gets the focus", () => {
    expect(startPosition({ isNew: true, remembered: session, stored: vault, length: 100 })).toBeNull();
  });

  it("clamps to the note as it is now", () => {
    expect(startPosition({ isNew: false, remembered: session, stored: null, length: 45 })).toEqual({ anchor: 45, head: 45, top: 40 });
  });
});
