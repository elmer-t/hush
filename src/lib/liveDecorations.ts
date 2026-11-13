import { syntaxTree } from "@codemirror/language";
import type { EditorState, Text } from "@codemirror/state";
import type { SyntaxNode, SyntaxNodeRef } from "@lezer/common";
import { parseWikilink } from "$lib/wikilink";

/**
 * What the Live editor shows over the markdown source, worked out from the Lezer syntax tree
 * and the selection. It only says which ranges to hide or style: `livePreview.ts` turns that
 * into CodeMirror decorations, and the document text is never changed.
 *
 * The rule throughout: a span's markdown syntax is hidden while no selection range touches
 * the span (its ends included), and shown, muted, while one does, so it can be edited. A
 * hidden range therefore never borders the cursor, and the arrow keys move through revealed
 * text one character at a time.
 */

export type DocRange = { from: number; to: number };

export type LiveSpec =
  /** Source text not shown. Never spans a line break. */
  | { kind: "hide"; from: number; to: number }
  /** Text shown with a class, and attributes for later link handling. */
  | { kind: "mark"; from: number; to: number; class: string; attributes?: Record<string, string> }
  /** A class on the line starting at `from`. */
  | { kind: "line"; from: number; class: string }
  /** Source text shown as a small widget (`liveBlocks.ts`). Never spans a line break. */
  | { kind: "widget"; from: number; to: number; widget: LiveWidget };

/** What an inline widget shows: a list bullet, a task checkbox (its `[` at `from + marker`), a rule. */
export type LiveWidget =
  | { type: "bullet" }
  | { type: "task"; checked: boolean; marker: number }
  | { type: "rule" };

export class BuildContext {
  readonly specs: LiveSpec[] = [];
  constructor(readonly state: EditorState) {}

  /** Whether a selection range touches `from`..`to`, ends included: its source is then shown. */
  revealed(from: number, to: number): boolean {
    return this.state.selection.ranges.some((r) => r.from <= to && r.to >= from);
  }

  /** Whether the cursor or selection is on any line of `from`..`to`. */
  revealedLines(from: number, to: number): boolean {
    const doc = this.state.doc;
    return this.revealed(doc.lineAt(from).from, doc.lineAt(to).to);
  }

  /** Hides `from`..`to`, split at line breaks, which a view plugin may not hide. */
  hide(from: number, to: number) {
    const doc = this.state.doc;
    while (from < to) {
      const line = doc.lineAt(from);
      const end = Math.min(to, line.to);
      if (end > from) this.specs.push({ kind: "hide", from, to: end });
      from = line.to + 1;
    }
  }

  mark(from: number, to: number, cls: string, attributes?: Record<string, string>) {
    if (to <= from) return;
    this.specs.push(attributes ? { kind: "mark", from, to, class: cls, attributes } : { kind: "mark", from, to, class: cls });
  }

  line(from: number, cls: string) {
    this.specs.push({ kind: "line", from: this.state.doc.lineAt(from).from, class: cls });
  }

  widget(from: number, to: number, widget: LiveWidget) {
    if (to > from) this.specs.push({ kind: "widget", from, to, widget });
  }

  /** Markdown syntax: hidden, or shown muted while its span is `revealed`. */
  syntax(from: number, to: number, revealed: boolean) {
    if (revealed) this.mark(from, to, "cm-live-syntax");
    else this.hide(from, to);
  }
}

/**
 * Handles one syntax node: adds its specs to `cx`. Returning false skips the node's children.
 * Parts 2 to 4 (block elements, diagrams, link clicks) add handlers by node name.
 */
export type NodeHandler = (node: SyntaxNode, cx: BuildContext) => boolean | void;

/** A span styled with `cls` whose `markName` children are its syntax. */
function span(cls: string, markName: string): NodeHandler {
  return (node, cx) => {
    cx.mark(node.from, node.to, cls);
    const revealed = cx.revealed(node.from, node.to);
    for (const m of node.getChildren(markName)) cx.syntax(m.from, m.to, revealed);
  };
}

const isSpace = (c: string) => c === " " || c === "\t";

function heading(level: number): NodeHandler {
  return (node, cx) => {
    cx.line(node.from, `cm-live-h${level}`);
    const revealed = cx.revealedLines(node.from, node.to);
    const doc = cx.state.doc;
    const marks = node.getChildren("HeaderMark");
    marks.forEach((m, i) => {
      let { from, to } = m;
      if (i === 0) while (to < node.to && isSpace(doc.sliceString(to, to + 1))) to++;
      else while (from > node.from && isSpace(doc.sliceString(from - 1, from))) from--;
      cx.syntax(from, to, revealed);
    });
  };
}

const linkMark = (cx: BuildContext, from: number, to: number, url: SyntaxNode) =>
  cx.mark(from, to, "cm-live-link", { "data-href": cx.state.sliceDoc(url.from, url.to) });

/** `[text](url "title")`: the text as a link. Links without a URL (references) stay text. */
const link: NodeHandler = (node, cx) => {
  const marks = node.getChildren("LinkMark");
  const url = node.getChild("URL");
  if (!url || marks.length < 3 || cx.state.sliceDoc(marks[1].from, marks[1].to) !== "]") return;
  const revealed = cx.revealed(node.from, node.to);
  linkMark(cx, marks[0].to, marks[1].from, url);
  cx.syntax(marks[0].from, marks[0].to, revealed);
  cx.syntax(marks[1].from, node.to, revealed);
};

/** `<https://…>`: the URL as a link, the angle brackets hidden. */
const autolink: NodeHandler = (node, cx) => {
  const url = node.getChild("URL");
  if (!url) return;
  const revealed = cx.revealed(node.from, node.to);
  linkMark(cx, url.from, url.to, url);
  for (const m of node.getChildren("LinkMark")) cx.syntax(m.from, m.to, revealed);
};

/** A bare URL, linked as markdown-it's `linkify` does. URLs inside links are theirs. */
const bareUrl: NodeHandler = (node, cx) => {
  const parent = node.parent?.name;
  if (parent === "Link" || parent === "Image" || parent === "Autolink" || parent === "LinkReference") return;
  linkMark(cx, node.from, node.to, node);
};

/** `[[target#heading|alias]]`: the text `markdown.ts` renders, as a link. */
const wikilink: NodeHandler = (node, cx) => {
  const link = parseWikilink(cx.state.sliceDoc(node.from, node.to), 0);
  if (!link) return;
  const revealed = cx.revealed(node.from, node.to);
  const textFrom = node.from + link.textFrom;
  const textTo = node.from + link.textTo;
  cx.syntax(node.from, textFrom, revealed);
  cx.mark(textFrom, textTo, "cm-live-wikilink", { "data-target": link.target });
  cx.syntax(textTo, node.to, revealed);
};

/** Part 1: inline formatting, headings and links. */
export const inlineHandlers: Record<string, NodeHandler> = {
  StrongEmphasis: span("cm-live-strong", "EmphasisMark"),
  Emphasis: span("cm-live-em", "EmphasisMark"),
  Strikethrough: span("cm-live-strike", "StrikethroughMark"),
  Highlight: span("cm-live-mark", "HighlightMark"),
  Subscript: span("cm-live-sub", "SubscriptMark"),
  Superscript: span("cm-live-sup", "SuperscriptMark"),
  InlineCode: span("cm-live-code", "CodeMark"),
  ATXHeading1: heading(1),
  ATXHeading2: heading(2),
  ATXHeading3: heading(3),
  ATXHeading4: heading(4),
  ATXHeading5: heading(5),
  ATXHeading6: heading(6),
  Link: link,
  Autolink: autolink,
  URL: bareUrl,
  Wikilink: wikilink,
  // Images are part 2; until then they, and anything in their alt text, stay source.
  Image: () => false,
};

/**
 * The specs for the syntax nodes that overlap `ranges` (the visible ranges), skipping the
 * frontmatter, which is never formatted as markdown (see `frontmatterView`).
 */
export function buildLive(
  state: EditorState,
  ranges: readonly DocRange[],
  handlers: Record<string, NodeHandler> = inlineHandlers,
): LiveSpec[] {
  const cx = new BuildContext(state);
  const fmEnd = frontmatterRange(state.doc)?.to ?? -1;
  const tree = syntaxTree(state);
  const seen = new Set<string>();
  for (const { from, to } of ranges) {
    tree.iterate({
      from,
      to,
      enter(ref: SyntaxNodeRef) {
        if (ref.from <= fmEnd && ref.type.name !== "Document") {
          // Inside the frontmatter: skip it, but enter a node that runs on past it.
          return ref.to > fmEnd ? undefined : false;
        }
        const handler = handlers[ref.type.name];
        if (!handler) return;
        // A node reaching into two visible ranges is entered twice; its specs are added once.
        const key = `${ref.type.id}:${ref.from}:${ref.to}`;
        if (seen.has(key)) return;
        seen.add(key);
        return handler(ref.node, cx);
      },
    });
  }
  return cx.specs;
}

const isFence = (line: string, closing: boolean) => line === "---" || (closing && line === "...");

/**
 * The frontmatter block, from the start of the note to the end of its closing line, matched
 * the way `FRONTMATTER` in `markdown.ts` does (one `---` line, then the first `---` or `...`
 * line), but line by line, so typing in a long note doesn't copy it into a string.
 */
export function frontmatterRange(doc: Text): DocRange | null {
  if (doc.lines < 2 || !isFence(doc.line(1).text.replace(/\r$/, ""), false)) return null;
  for (let n = 2; n <= doc.lines; n++) {
    const line = doc.line(n);
    if (isFence(line.text.replace(/\r$/, ""), true)) return { from: 0, to: line.to };
  }
  return null;
}

export type FrontmatterView = DocRange & {
  /** Shown as one "Properties" line while the selection is outside it; as raw text otherwise. */
  collapsed: boolean;
  /** The top-level keys, for the collapsed line. */
  keys: string[];
};

export function frontmatterView(state: EditorState): FrontmatterView | null {
  const range = frontmatterRange(state.doc);
  if (!range) return null;
  const keys: string[] = [];
  for (let n = 2; n < state.doc.lineAt(range.to).number; n++) {
    const key = /^([^\s:#-][^:]*):/.exec(state.doc.line(n).text)?.[1].trim();
    if (key) keys.push(key);
  }
  const collapsed = !state.selection.ranges.some((r) => r.from <= range.to && r.to >= range.from);
  return { ...range, collapsed, keys };
}
