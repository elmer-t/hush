import type { MarkdownConfig } from "@lezer/markdown";
import { tags as t } from "@lezer/highlight";

const TILDE = 0x7e;
const CARET = 0x5e;
const BACKSLASH = 0x5c;

/** What markdown lets a backslash escape; the preview shows the character without it. */
const ESCAPED = /\\([!-/:-@[-`{-~ ])/g;

/** A word so far that is a bare link (`https://host/~user`, `www.host/~user`): its `~` and `^` are the link's. */
const IN_LINK = /(?:^|\s)(?:[a-z][a-z0-9+.-]*:\/\/|www\.)\S*$/i;

/** A `~subscript~` or `^superscript^` found in some text. Offsets are into that text. */
export type SubSuper = {
  tag: "sub" | "sup";
  /** The end, after the closing marker. */
  to: number;
  /** The text between the markers, with backslash escapes resolved. */
  text: string;
};

/**
 * The `~text~` (subscript) or `^text^` (superscript) starting at `pos` in `src` (#182), read
 * by Pandoc's rule: no white space between the markers unless it is escaped (`~a\ b~`), and
 * the text is taken as it is, without other formatting. Both markers stand alone, so `~~` is
 * left to strikethrough and `~~a~b~~` is `a~b` struck through. A lone marker (`~5 minutes`,
 * `2^10`) has no partner before the next space and stays text, as does one in a bare link.
 * Shared by the markdown-it rule (the preview) and the Lezer extension (the Live editor);
 * code is theirs, as both parse code spans first.
 */
export function parseSubSuper(src: string, pos: number, end = src.length): SubSuper | null {
  const marker = src.charCodeAt(pos);
  if (marker !== TILDE && marker !== CARET) return null;
  if (src.charCodeAt(pos - 1) === marker || src.charCodeAt(pos + 1) === marker) return null;
  for (let i = pos + 1; i < end; i++) {
    const c = src.charCodeAt(i);
    if (c === BACKSLASH) {
      // An escaped line break is a hard break, not text.
      const next = src.charCodeAt(++i);
      if (i >= end || next === 0x0a || next === 0x0d) return null;
    } else if (c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0d) {
      return null;
    } else if (c === marker) {
      if (src.charCodeAt(i + 1) === marker) return null;
      if (IN_LINK.test(src.slice(Math.max(0, pos - 2000), pos))) return null;
      return { tag: marker === TILDE ? "sub" : "sup", to: i + 1, text: src.slice(pos + 1, i).replace(ESCAPED, "$1") };
    }
  }
  return null;
}

/**
 * The Lezer markdown extension for the Live editor: `Subscript` and `Superscript` nodes with
 * their marks. `@lezer/markdown` has parsers of these names, which `markdownLanguage` turns
 * on; they are removed here and replaced by one that reads `parseSubSuper`, so Live and the
 * preview cannot differ.
 */
export const subSuperMarkdown: MarkdownConfig = {
  remove: ["Subscript", "Superscript"],
  defineNodes: [
    { name: "Subscript" },
    { name: "SubscriptMark", style: t.processingInstruction },
    { name: "Superscript" },
    { name: "SuperscriptMark", style: t.processingInstruction },
  ],
  parseInline: [
    {
      name: "SubSuper",
      parse(cx, next, pos) {
        if (next !== TILDE && next !== CARET) return -1;
        const found = parseSubSuper(cx.text, pos - cx.offset, cx.end - cx.offset);
        if (!found) return -1;
        const [node, mark] = found.tag === "sub" ? ["Subscript", "SubscriptMark"] : ["Superscript", "SuperscriptMark"];
        const to = found.to + cx.offset;
        return cx.addElement(cx.elt(node, pos, to, [cx.elt(mark, pos, pos + 1), cx.elt(mark, to - 1, to)]));
      },
    },
  ],
};
