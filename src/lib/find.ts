/**
 * Find in the open note: the find bar's query and its match count. The matching itself is
 * CodeMirror's search (`MarkdownEditor.svelte`), in Source and in Live.
 */

export type FindOptions = { caseSensitive: boolean; wholeWord: boolean; regexp: boolean };

/** The find bar's query; `replace` is used by the editor only. */
export type FindSpec = FindOptions & { query: string; replace: string };

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

/** "3 of 12", or the total alone when no match is current. */
export function formatCount(current: number, total: number): string {
  if (total === 0) return "No matches";
  if (current < 0) return total === 1 ? "1 match" : `${total} matches`;
  return `${current + 1} of ${total}`;
}
