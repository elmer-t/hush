import type { MarkdownConfig } from "@lezer/markdown";

/** A `[[wikilink]]` found in some text. Offsets are into that text. */
export type Wikilink = {
  from: number;
  to: number;
  /** The note it names: the part before `#` and `|`, trimmed. */
  target: string;
  /** What it reads as: the alias, else the target part with any `#heading`, trimmed. */
  text: string;
  /** Where `text` sits in the source, so a view can show it without rewriting anything. */
  textFrom: number;
  textTo: number;
};

/**
 * The `[[target#heading|alias]]` starting at `pos` in `src`, the way `markdown::parse` reads
 * links: not `![[embeds]]`, no brackets or line breaks inside, and an empty target
 * (`[[#heading]]`) stays text. Code is the caller's concern: both markdown-it and Lezer parse
 * code spans before they would reach the `[[`. Shared by the markdown-it rule (the preview)
 * and the Lezer extension (the Live editor), so the two read links the same way.
 */
export function parseWikilink(src: string, pos: number): Wikilink | null {
  if (src.charCodeAt(pos) !== 0x5b || src.charCodeAt(pos + 1) !== 0x5b || src.charCodeAt(pos - 1) === 0x21) return null;
  const end = src.indexOf("]]", pos + 2);
  if (end < 0) return null;
  const innerFrom = pos + 2;
  const inner = src.slice(innerFrom, end);
  if (/[[\]\n]/.test(inner)) return null;
  const bar = inner.indexOf("|");
  const targetPart = bar < 0 ? inner : inner.slice(0, bar);
  const hash = targetPart.indexOf("#");
  const target = (hash < 0 ? targetPart : targetPart.slice(0, hash)).trim();
  if (!target) return null;
  const alias = bar < 0 ? "" : inner.slice(bar + 1);
  // The shown text, trimmed, as offsets into `src`.
  const [partFrom, part] = alias.trim() ? [innerFrom + bar + 1, alias] : [innerFrom, targetPart];
  const lead = part.length - part.trimStart().length;
  const text = part.trim();
  return { from: pos, to: end + 2, target, text, textFrom: partFrom + lead, textTo: partFrom + lead + text.length };
}

/**
 * The Lezer markdown extension for the Live editor: a `Wikilink` node over each link, taking
 * precedence over `[` links as the markdown-it rule does. Its parts are found again with
 * `parseWikilink` on the node's text.
 */
export const wikilinkMarkdown: MarkdownConfig = {
  defineNodes: ["Wikilink"],
  parseInline: [
    {
      name: "Wikilink",
      before: "Link",
      parse(cx, next, pos) {
        if (next !== 0x5b) return -1;
        const link = parseWikilink(cx.text, pos - cx.offset);
        if (!link) return -1;
        return cx.addElement(cx.elt("Wikilink", pos, link.to + cx.offset));
      },
    },
  ],
};
