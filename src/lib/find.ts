/**
 * Find in the open note: the matching shared by the find bar's preview search. The editor
 * uses CodeMirror's own search; this is for the rendered text of the preview.
 */

export type FindOptions = { caseSensitive: boolean; wholeWord: boolean; regexp: boolean };

/** The find bar's query; `replace` is used by the editor only. */
export type FindSpec = FindOptions & { query: string; replace: string };

export type Match = { from: number; to: number };

/** A point in a list of text segments (in the preview, text nodes). */
export type SegmentPoint = { segment: number; offset: number };
export type SegmentMatch = { start: SegmentPoint; end: SegmentPoint };

const WORD = "[\\p{L}\\p{N}_]";

/** The pattern for `query`, or null when there is nothing to search for or it isn't a valid regexp. */
export function findPattern(query: string, { caseSensitive, wholeWord, regexp }: FindOptions): RegExp | null {
  if (!query) return null;
  let source = regexp ? query : query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (wholeWord) source = `(?<!${WORD})(?:${source})(?!${WORD})`;
  try {
    return new RegExp(source, caseSensitive ? "gu" : "giu");
  } catch {
    return null;
  }
}

/** The non-overlapping, non-empty matches of `pattern` in `text`. */
export function findInText(text: string, pattern: RegExp): Match[] {
  const matches: Match[] = [];
  pattern.lastIndex = 0;
  for (let m = pattern.exec(text); m; m = pattern.exec(text)) {
    if (m[0].length === 0) {
      pattern.lastIndex++;
      continue;
    }
    matches.push({ from: m.index, to: m.index + m[0].length });
  }
  return matches;
}

/**
 * The matches of `pattern` in text split over `segments`, so a match can span inline
 * formatting (`a **b** c` is the segments "a ", "b", " c"). A null segment is a block
 * break that no match crosses. A newline inside a segment counts as a space, the way a
 * soft line break in a paragraph is shown.
 */
export function findInSegments(segments: (string | null)[], pattern: RegExp): SegmentMatch[] {
  const starts: number[] = [];
  const breaks = new Set<number>();
  let text = "";
  segments.forEach((segment, i) => {
    starts.push(text.length);
    if (segment === null) {
      breaks.add(text.length);
      text += "\n";
    } else {
      text += segment.replace(/[\r\n]/g, " ");
    }
  });

  // The segment holding offset `pos`; for an end offset, the one it ends, not the next.
  const locate = (pos: number, end: boolean): SegmentPoint => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (end ? starts[mid] < pos : starts[mid] <= pos) lo = mid;
      else hi = mid - 1;
    }
    return { segment: lo, offset: pos - starts[lo] };
  };

  const crossesBreak = ({ from, to }: Match) => {
    for (let i = from; i < to; i++) if (breaks.has(i)) return true;
    return false;
  };

  return findInText(text, pattern)
    .filter((m) => !crossesBreak(m))
    .map((m) => ({ start: locate(m.from, false), end: locate(m.to, true) }));
}

/** The match after (`dir` 1) or before (-1) `index` of `count`, wrapping around; -1 without matches. */
export function step(index: number, count: number, dir: 1 | -1): number {
  if (count === 0) return -1;
  if (index < 0) return dir === 1 ? 0 : count - 1;
  return (index + dir + count) % count;
}

/** The first match starting at or after `pos`, wrapping to the first; -1 without matches. */
export function nearestMatch(starts: number[], pos: number): number {
  if (starts.length === 0) return -1;
  const i = starts.findIndex((start) => start >= pos);
  return i < 0 ? 0 : i;
}

/** "3 of 12", or the total alone when no match is current. */
export function formatCount(current: number, total: number): string {
  if (total === 0) return "No matches";
  if (current < 0) return total === 1 ? "1 match" : `${total} matches`;
  return `${current + 1} of ${total}`;
}
