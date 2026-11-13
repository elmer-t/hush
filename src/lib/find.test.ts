import { describe, expect, it } from "vitest";
import { findPattern, formatCount } from "$lib/find";

const opts = { caseSensitive: false, wholeWord: false, regexp: false };

function texts(text: string, query: string, o = {}) {
  const pattern = findPattern(query, { ...opts, ...o });
  return pattern ? [...text.matchAll(pattern)].map((m) => m[0]).filter(Boolean) : null;
}

describe("findPattern", () => {
  it("is null for an empty query", () => {
    expect(findPattern("", opts)).toBeNull();
  });

  it("is null for an invalid regular expression", () => {
    expect(findPattern("a(", { ...opts, regexp: true })).toBeNull();
  });

  it("treats the query literally unless regexp is on", () => {
    expect(texts("a.c abc a.c", "a.c")).toEqual(["a.c", "a.c"]);
    expect(texts("a.c abc", "a.c", { regexp: true })).toEqual(["a.c", "abc"]);
  });
});

describe("findPattern matching", () => {
  it("is case-insensitive by default", () => {
    expect(texts("Note, note, NOTE", "note")).toEqual(["Note", "note", "NOTE"]);
  });

  it("matches case when asked", () => {
    expect(texts("Note, note, NOTE", "note", { caseSensitive: true })).toEqual(["note"]);
  });

  it("matches whole words only when asked, including accented letters", () => {
    expect(texts("cat category scat cat_ café", "cat", { wholeWord: true })).toEqual(["cat"]);
    expect(texts("café cafés", "café", { wholeWord: true })).toEqual(["café"]);
  });

  it("finds nothing for a regexp that only matches empty text", () => {
    expect(texts("abc", "x*", { regexp: true })).toEqual([]);
  });
});

describe("formatCount", () => {
  it("shows the position, a bare total, or no matches", () => {
    expect(formatCount(2, 12)).toBe("3 of 12");
    expect(formatCount(-1, 12)).toBe("12 matches");
    expect(formatCount(-1, 1)).toBe("1 match");
    expect(formatCount(-1, 0)).toBe("No matches");
  });
});
