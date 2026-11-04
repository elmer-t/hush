import { describe, expect, it } from "vitest";
import { findInSegments, findInText, findPattern, formatCount, nearestMatch, step } from "$lib/find";

const opts = { caseSensitive: false, wholeWord: false, regexp: false };

function texts(text: string, query: string, o = {}) {
  const pattern = findPattern(query, { ...opts, ...o });
  return pattern ? findInText(text, pattern).map(({ from, to }) => text.slice(from, to)) : null;
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

describe("findInText", () => {
  it("is case-insensitive by default", () => {
    expect(texts("Note, note, NOTE", "note")).toEqual(["Note", "note", "NOTE"]);
  });

  it("matches case when asked", () => {
    expect(texts("Note, note, NOTE", "note", { caseSensitive: true })).toEqual(["note"]);
  });

  it("keeps offsets right for characters whose lowercase is longer", () => {
    const text = "İstanbul and istanbul";
    const pattern = findPattern("istanbul", opts)!;
    for (const { from, to } of findInText(text, pattern)) expect(to - from).toBe(8);
  });

  it("matches whole words only when asked, including accented letters", () => {
    expect(texts("cat category scat cat_ café", "cat", { wholeWord: true })).toEqual(["cat"]);
    expect(texts("café cafés", "café", { wholeWord: true })).toEqual(["café"]);
  });

  it("does not return overlapping matches", () => {
    expect(findInText("aaaa", findPattern("aa", opts)!)).toEqual([
      { from: 0, to: 2 },
      { from: 2, to: 4 },
    ]);
  });

  it("skips empty regexp matches instead of looping", () => {
    expect(texts("abc", "x*", { regexp: true })).toEqual([]);
  });
});

describe("findInSegments", () => {
  it("finds a match spanning inline formatting", () => {
    // `a **b** c` renders as the text nodes "a ", "b", " c".
    const matches = findInSegments(["a ", "b", " c"], findPattern("a b c", opts)!);
    expect(matches).toEqual([{ start: { segment: 0, offset: 0 }, end: { segment: 2, offset: 2 } }]);
  });

  it("ends a match at the end of a segment, not the start of the next", () => {
    const matches = findInSegments(["say ", "bold", " now"], findPattern("bold", opts)!);
    expect(matches).toEqual([{ start: { segment: 1, offset: 0 }, end: { segment: 1, offset: 4 } }]);
  });

  it("respects the case toggle across segments", () => {
    expect(findInSegments(["Hel", "lo"], findPattern("hello", opts)!)).toHaveLength(1);
    expect(findInSegments(["Hel", "lo"], findPattern("hello", { ...opts, caseSensitive: true })!)).toHaveLength(0);
  });

  it("does not match across a block break", () => {
    // Two paragraphs, "end" and "start": no "end start" match, though each word is found.
    const segments = ["the end", null, "start here"];
    expect(findInSegments(segments, findPattern("end start", opts)!)).toEqual([]);
    expect(findInSegments(segments, findPattern("end", opts)!)).toHaveLength(1);
    expect(findInSegments(segments, findPattern("end\\sstart", { ...opts, regexp: true })!)).toEqual([]);
  });

  it("reads a soft line break inside a paragraph as a space", () => {
    const matches = findInSegments(["one\ntwo"], findPattern("one two", opts)!);
    expect(matches).toEqual([{ start: { segment: 0, offset: 0 }, end: { segment: 0, offset: 7 } }]);
  });

  it("finds nothing in no segments", () => {
    expect(findInSegments([], findPattern("x", opts)!)).toEqual([]);
  });
});

describe("step", () => {
  it("moves forward and back, wrapping around", () => {
    expect(step(0, 3, 1)).toBe(1);
    expect(step(2, 3, 1)).toBe(0);
    expect(step(0, 3, -1)).toBe(2);
  });

  it("starts at the first or last match when there is no current one", () => {
    expect(step(-1, 3, 1)).toBe(0);
    expect(step(-1, 3, -1)).toBe(2);
  });

  it("is -1 without matches", () => {
    expect(step(0, 0, 1)).toBe(-1);
  });
});

describe("nearestMatch", () => {
  const starts = [5, 10, 20];
  it("picks the first match at or after the position", () => {
    expect(nearestMatch(starts, 0)).toBe(0);
    expect(nearestMatch(starts, 10)).toBe(1);
    expect(nearestMatch(starts, 11)).toBe(2);
  });

  it("wraps to the first match past the last one", () => {
    expect(nearestMatch(starts, 21)).toBe(0);
  });

  it("is -1 without matches", () => {
    expect(nearestMatch([], 3)).toBe(-1);
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
