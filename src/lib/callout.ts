/**
 * Callouts (#184): a block quote whose first line starts with `[!type]`, as in Obsidian and
 * GitHub. Read here for the Live editor (`liveBlocks.ts`).
 */

/** The kinds a callout is drawn as: each has a colour and an icon (`app.css`). */
export const CALLOUT_KINDS = [
  "note",
  "abstract",
  "info",
  "todo",
  "tip",
  "success",
  "question",
  "warning",
  "important",
  "caution",
  "failure",
  "danger",
  "bug",
  "example",
  "quote",
] as const;

export type CalloutKind = (typeof CALLOUT_KINDS)[number];

/** Obsidian's other names for a kind. GitHub's `important` and `caution` are kinds of their own. */
const ALIASES: Record<string, CalloutKind> = {
  summary: "abstract",
  tldr: "abstract",
  hint: "tip",
  check: "success",
  done: "success",
  help: "question",
  faq: "question",
  attention: "warning",
  fail: "failure",
  missing: "failure",
  error: "danger",
  cite: "quote",
};

export type Callout = {
  /** How it is drawn; an unknown type is a `note`. */
  kind: CalloutKind;
  /** The length of the marker in the line: `[!type]`, a fold sign, and the white space after it. */
  marker: number;
  /** The title written after the marker, trimmed; empty when there is none. */
  title: string;
  /** The title shown when none is written: the type as written, with a capital. */
  label: string;
};

/** `[!type]`, a `+` or `-` (Obsidian's fold signs, not acted on), then white space or the end of the line. */
const MARKER = /^\[!([A-Za-z0-9_-]+)\][+-]?(?:[ \t]+|$)/;

/**
 * The callout that `line` opens, or null. `line` is the first line of a block quote's first
 * paragraph, without the `>` and the white space after it. The type is case-insensitive.
 */
export function parseCallout(line: string): Callout | null {
  const match = MARKER.exec(line);
  if (!match) return null;
  const type = match[1].toLowerCase();
  const kind = ALIASES[type] ?? ((CALLOUT_KINDS as readonly string[]).includes(type) ? (type as CalloutKind) : "note");
  return { kind, marker: match[0].length, title: line.slice(match[0].length).trim(), label: type[0].toUpperCase() + type.slice(1) };
}
