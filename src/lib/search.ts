import { HIGHLIGHT_END, HIGHLIGHT_START, type DateField, type ReadFilter, type SearchQuery, type SearchScope } from "$lib/api";

/** What the filters panel edits. Dates are `YYYY-MM-DD` from date inputs, or empty. */
export type Filters = {
  tags: string[];
  topic: string;
  author: string;
  dateField: DateField;
  from: string;
  to: string;
  attributes: [string, string][];
  /** 0 for any rating. */
  minRating: number;
  /** "" for any read status. */
  readStatus: ReadFilter | "";
};

export function emptyFilters(): Filters {
  return { tags: [], topic: "", author: "", dateField: "created", from: "", to: "", attributes: [], minRating: 0, readStatus: "" };
}

export function activeFilterCount(f: Filters): number {
  return (
    f.tags.length + f.attributes.length + Number(!!f.topic) + Number(!!f.author) + Number(!!(f.from || f.to)) +
    Number(f.minRating > 0) +
    Number(!!f.readStatus)
  );
}

/** Local midnight of `date` plus `days`, as Unix millis; dates are shown in local time too. */
function localDay(date: string, days = 0): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + days).getTime() : null;
}

export function toQuery(text: string, folder: string | null, f: Filters, offset: number, limit: number): SearchQuery {
  return {
    text,
    tags: [...f.tags],
    topic: f.topic || null,
    author: f.author || null,
    folder,
    dateField: f.dateField,
    from: localDay(f.from),
    // The end date is inclusive in the UI and exclusive in the query.
    to: localDay(f.to, 1),
    attributes: f.attributes.map(([k, v]) => [k, v]),
    minRating: f.minRating || null,
    readStatus: f.readStatus || null,
    limit,
    offset,
  };
}

/** Splits a snippet into plain and matched parts, so it renders without raw HTML. */
export function highlightParts(snippet: string): { text: string; match: boolean }[] {
  const parts: { text: string; match: boolean }[] = [];
  for (const [i, chunk] of snippet.split(HIGHLIGHT_START).entries()) {
    const [inside, after] = i === 0 ? [null, chunk] : splitOnce(chunk, HIGHLIGHT_END);
    if (inside) parts.push({ text: inside, match: true });
    if (after) parts.push({ text: after, match: false });
  }
  return parts;
}

function splitOnce(s: string, sep: string): [string, string] {
  const at = s.indexOf(sep);
  return at < 0 ? [s, ""] : [s.slice(0, at), s.slice(at + sep.length)];
}

/** The folder a query is limited to. Whole-vault scope only applies to a search: without
 *  text or filters the list still browses the selected folder. */
export function searchFolder(selected: string | null, scope: SearchScope, searching: boolean): string | null {
  return searching && scope === "vault" ? null : selected;
}

/** Everything but the text that a search depends on, as one key for `searchScheduler`. */
export function searchKey(f: Filters, selected: string | null, scope: SearchScope): string {
  return JSON.stringify([f, selected, scope]);
}

/** `path` in at most `max` characters: the first segment, `…`, and as many trailing
 *  segments as fit; the last segment is always kept whole. */
export function shortPath(path: string, max = 24): string {
  if (path.length <= max) return path;
  const parts = path.split("/");
  const last = parts[parts.length - 1];
  let tail = last;
  for (let i = parts.length - 2; i > 0; i--) {
    const next = `${parts[i]}/${tail}`;
    if (`${parts[0]}/…/${next}`.length > max) break;
    tail = next;
  }
  const head = `${parts[0]}/…/${tail}`;
  return parts.length > 2 && head.length <= max ? head : `…/${last}`;
}

/** Where a search looks, as shown after `in`. */
export function scopeLabel(folder: string | null): string {
  return folder === null ? "whole vault" : shortPath(folder);
}

/** The idle search box names the folder a search would be limited to. */
export function searchPlaceholder(selected: string | null, scope: SearchScope): string {
  return selected !== null && scope === "folder" ? `Search ${shortPath(selected)}` : "Search notes";
}

/** `42 results`, `50+ results`, `1 result`. */
export function resultCount(count: number, hasMore: boolean): string {
  const noun = count === 1 && !hasMore ? "result" : "results";
  return `${count.toLocaleString()}${hasMore ? "+" : ""} ${noun}`;
}

/** ` · newest first` or ` · best match first` for a text search, else nothing.
 *  A very broad search lists the newest matches rather than ranking them (`newestFirst`). */
export function resultOrder(text: string, newestFirst: boolean): string {
  return text.trim() ? (newestFirst ? " · newest first" : " · best match first") : "";
}

/** The line above the results: how many there are, where the search looked (`folder`,
 *  null for the whole vault; left out to omit it) and, for a text search, their order. */
export function searchSummary(
  count: number,
  hasMore: boolean,
  text: string,
  newestFirst: boolean,
  folder?: string | null,
): string {
  const where = folder === undefined ? "" : ` in ${scopeLabel(folder)}`;
  return `${resultCount(count, hasMore)}${where}${resultOrder(text, newestFirst)}`;
}
