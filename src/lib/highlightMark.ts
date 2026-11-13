import type { MarkdownConfig } from "@lezer/markdown";
import { tags as t } from "@lezer/highlight";

/** Whether the `==` at `pos` in `src` can open and can close a highlight. */
export type HighlightDelimiter = { open: boolean; close: boolean };

const SPACE = /\s|^$/;
const PUNCTUATION = /[\p{S}\p{P}]/u;

/**
 * The `==` of `==highlighted==` at `pos` in `src` (#181), or null when there is none: exactly
 * two `=` (of a longer run, the last two), which open or close by the rule CommonMark has for
 * `**`: an opener is not followed by white space, a closer not preceded by it, and next to
 * punctuation only at a word's edge. So `a == b` and `x ==` stay text, and `a==b==c` highlights
 * `b`, as in Obsidian. Shared by the markdown-it rule (the preview) and the Lezer extension
 * (the Live editor), which both pair the delimiters the way they pair `~~`; code is theirs too,
 * as both parse code spans first.
 */
export function highlightDelimiter(src: string, pos: number): HighlightDelimiter | null {
  if (src.charCodeAt(pos) !== 0x3d || src.charCodeAt(pos + 1) !== 0x3d || src.charCodeAt(pos + 2) === 0x3d) return null;
  const before = src.slice(Math.max(0, pos - 1), pos);
  const after = src.slice(pos + 2, pos + 3);
  const sBefore = SPACE.test(before);
  const sAfter = SPACE.test(after);
  const pBefore = PUNCTUATION.test(before);
  const pAfter = PUNCTUATION.test(after);
  return {
    open: !sAfter && (!pAfter || sBefore || pBefore),
    close: !sBefore && (!pBefore || sAfter || pAfter),
  };
}

const HighlightDelim = { resolve: "Highlight", mark: "HighlightMark" };

/** The Lezer markdown extension for the Live editor: a `Highlight` node with its two `HighlightMark`s. */
export const highlightMarkdown: MarkdownConfig = {
  defineNodes: [{ name: "Highlight" }, { name: "HighlightMark", style: t.processingInstruction }],
  parseInline: [
    {
      name: "Highlight",
      after: "Emphasis",
      parse(cx, next, pos) {
        if (next !== 0x3d) return -1;
        const delimiter = highlightDelimiter(cx.text, pos - cx.offset);
        return delimiter ? cx.addDelimiter(HighlightDelim, pos, pos + 2, delimiter.open, delimiter.close) : -1;
      },
    },
  ],
};
