import { syntaxTree } from "@codemirror/language";
import { isolateHistory } from "@codemirror/commands";
import { Transaction, type EditorState, type TransactionSpec } from "@codemirror/state";
import type { SyntaxNode } from "@lezer/common";
import { parseCallout, type Callout } from "$lib/callout";
import { frontmatterRange, type BuildContext, type NodeHandler } from "$lib/liveDecorations";

/**
 * Part 2 of the Live editor (#135): block elements. Like part 1, this only says what to hide,
 * style or show as a widget over the unchanged source; the one edit it makes is ticking a task.
 *
 * Line-prefix markers (list bullets, `>`) reveal while the cursor is on their line; multi-line
 * blocks (code, tables) reveal while the cursor is on any of their lines (`cursorInBlock`).
 *
 * Lists, tasks, quotes, code and rules are handlers for the view plugin's visible-range build
 * (`blockHandlers`). Tables and images change line heights, so they come from a state field
 * over the whole note (`findBlocks`), which keeps CodeMirror's height map right; their widgets
 * are only drawn, and rendered, when they scroll into view.
 */

/** Whether a selection range touches any line of `from`..`to`: the block then shows its source. */
export function cursorInBlock(state: EditorState, from: number, to: number): boolean {
  const start = state.doc.lineAt(from).from;
  const end = state.doc.lineAt(to).to;
  return state.selection.ranges.some((r) => r.from <= end && r.to >= start);
}

/** Calls `f` with the start of each line of `from`..`to`. */
function eachLine(state: EditorState, from: number, to: number, f: (lineFrom: number) => void) {
  const doc = state.doc;
  for (let n = doc.lineAt(from).number, last = doc.lineAt(to).number; n <= last; n++) f(doc.line(n).from);
}

const isList = (n: SyntaxNode | null) => n?.name === "BulletList" || n?.name === "OrderedList";

/** `- item`: a bullet (numbers stay), the line marked with its depth; tasks get a checkbox. */
const listItem: NodeHandler = (node, cx) => {
  const mark = node.getChild("ListMark");
  if (!mark) return;
  let depth = 0;
  for (let p = node.parent; p; p = p.parent) if (isList(p)) depth++;
  // The item's own lines, up to a nested list, which marks its lines itself.
  let end = node.to;
  for (let c = node.firstChild; c; c = c.nextSibling) {
    if (isList(c)) {
      end = cx.state.doc.lineAt(c.from).from - 1;
      break;
    }
  }
  eachLine(cx.state, node.from, Math.max(node.from, end), (from) => {
    cx.line(from, "cm-live-li");
    cx.line(from, `cm-live-li-${Math.min(depth, 6)}`);
  });
  if (cx.revealedLines(mark.from, mark.to)) return;
  const bullet = node.parent?.name === "BulletList";
  // A task is the item's first paragraph, on the list marker's line, as in the preview (`markdown.ts`).
  const marker = node.getChild("Task")?.getChild("TaskMarker");
  if (marker && marker.from <= cx.state.doc.lineAt(mark.from).to) {
    const checked = cx.state.sliceDoc(marker.from + 1, marker.from + 2) !== " ";
    const from = bullet ? mark.from : marker.from;
    cx.widget(from, marker.to, { type: "task", checked, marker: marker.from - from });
  } else if (bullet) {
    cx.widget(mark.from, mark.to, { type: "bullet" });
  }
};

/** The `QuoteMark`s that belong to `quote`, not to a quote nested in it. */
function quoteMarks(quote: SyntaxNode): SyntaxNode[] {
  const out: SyntaxNode[] = [];
  const walk = (n: SyntaxNode) => {
    for (let c = n.firstChild; c; c = c.nextSibling) {
      if (c.name === "QuoteMark") out.push(c);
      else if (c.name !== "Blockquote" && c.name !== "FencedCode" && c.name !== "CodeBlock") walk(c);
    }
  };
  walk(quote);
  return out;
}

/** The callout a quote is (#184), with where its marker starts: `parseCallout` on the first line of its first paragraph. */
function calloutOf(state: EditorState, quote: SyntaxNode): (Callout & { from: number }) | null {
  let first = quote.firstChild;
  while (first?.name === "QuoteMark") first = first.nextSibling;
  const line = state.doc.lineAt(quote.from);
  if (first?.name !== "Paragraph" || first.from > line.to) return null;
  const callout = parseCallout(state.sliceDoc(first.from, Math.min(first.to, line.to)));
  return callout && { ...callout, from: first.from };
}

/** The quotes directly or deeper inside `quote` that are callouts: they colour their own lines. */
function nestedCallouts(state: EditorState, quote: SyntaxNode): SyntaxNode[] {
  const out: SyntaxNode[] = [];
  const walk = (n: SyntaxNode) => {
    for (let c = n.firstChild; c; c = c.nextSibling) {
      if (c.name === "Blockquote" && calloutOf(state, c)) out.push(c);
      else if (c.name !== "FencedCode" && c.name !== "CodeBlock") walk(c);
    }
  };
  walk(quote);
  return out;
}

/**
 * `> quote`: the markers hidden off the cursor's line, every line styled with the quote bar.
 * A callout's lines also get its kind, and its `[!type]` marker hides off the cursor's line:
 * the written title stays, or the type's name is shown in the marker's place.
 */
const blockquote: NodeHandler = (node, cx) => {
  let nested = false;
  for (let p = node.parent; p && !nested; p = p.parent) nested = p.name === "Blockquote";
  eachLine(cx.state, node.from, node.to, (from) => cx.line(from, nested ? "cm-live-quote-nested" : "cm-live-quote"));
  for (const m of quoteMarks(node)) {
    const to = cx.state.sliceDoc(m.to, m.to + 1) === " " ? m.to + 1 : m.to;
    cx.syntax(m.from, to, cx.revealedLines(m.from, m.to));
  }
  const callout = calloutOf(cx.state, node);
  if (!callout) return;
  const inner = nestedCallouts(cx.state, node);
  eachLine(cx.state, node.from, node.to, (from) => {
    if (inner.some((q) => from >= cx.state.doc.lineAt(q.from).from && from <= q.to)) return;
    cx.line(from, "cm-live-callout");
    cx.line(from, `cm-live-callout-${callout.kind}`);
  });
  cx.line(node.from, "cm-live-callout-title");
  const to = callout.from + callout.marker;
  if (cx.revealedLines(node.from, node.from)) cx.mark(callout.from, to, "cm-live-syntax");
  else if (callout.title) cx.hide(callout.from, to);
  else cx.widget(callout.from, to, { type: "callout", label: callout.label });
};

/** The first word of a fence's info string, lowercased, as `markdown.ts` reads it. */
export function fenceLanguage(state: EditorState, fence: SyntaxNode): string {
  const info = fence.getChild("CodeInfo");
  return info ? state.sliceDoc(info.from, info.to).trim().split(/\s+/)[0].toLowerCase() : "";
}

/** The lines of a code block, styled as one box; fences hidden while the cursor is outside. */
function codeLines(node: SyntaxNode, cx: BuildContext, fences: SyntaxNode[]) {
  const doc = cx.state.doc;
  const first = doc.lineAt(node.from);
  const last = doc.lineAt(node.to);
  const revealed = cursorInBlock(cx.state, node.from, node.to);
  const open = fences[0];
  const close = fences.length > 1 ? fences[fences.length - 1] : null;
  eachLine(cx.state, node.from, node.to, (from) => {
    cx.line(from, "cm-live-codeblock");
    if (from === first.from) cx.line(from, "cm-live-codeblock-first");
    if (from === last.from) cx.line(from, "cm-live-codeblock-last");
    const fence = (open && from === first.from) || (close && from === doc.lineAt(close.from).from);
    if (fence && !revealed) cx.line(from, "cm-live-fence");
  });
  if (open) cx.syntax(open.from, first.to, revealed);
  if (close) cx.syntax(close.from, doc.lineAt(close.to).to, revealed);
}

/**
 * Fenced code: an editable monospaced box. Top-level ```mermaid fences are left to the
 * diagram widget (#136); mermaid fences in lists or quotes are code here.
 */
const fencedCode: NodeHandler = (node, cx) => {
  if (node.parent?.name === "Document" && fenceLanguage(cx.state, node) === "mermaid") return false;
  codeLines(node, cx, node.getChildren("CodeMark"));
  return false;
};

const indentedCode: NodeHandler = (node, cx) => {
  codeLines(node, cx, []);
  return false;
};

/** `---`: a rule off the cursor's line. Frontmatter fences never get here (`buildLive`). */
const horizontalRule: NodeHandler = (node, cx) => {
  if (!cx.revealedLines(node.from, node.to)) cx.widget(node.from, node.to, { type: "rule" });
};

/** A table the state field shows as a widget: nothing inside it is decorated meanwhile. */
const table: NodeHandler = (node, cx) => {
  if (node.parent?.name === "Document" && !cursorInBlock(cx.state, node.from, node.to)) return false;
};

/** Part 2: block elements, for `buildLive` alongside `inlineHandlers`. */
export const blockHandlers: Record<string, NodeHandler> = {
  ListItem: listItem,
  Blockquote: blockquote,
  FencedCode: fencedCode,
  CodeBlock: indentedCode,
  HorizontalRule: horizontalRule,
  Table: table,
};

/** A table or image shown as a rendered widget while the cursor is outside it. */
export type LiveBlock = {
  kind: "table" | "image";
  from: number;
  to: number;
  /** Whole lines, replaced by a block widget; else an image inline in its line. */
  block: boolean;
};

/** Nodes that can't hold a table or an image. */
const LEAF_BLOCKS = new Set(["FencedCode", "CodeBlock", "HTMLBlock", "CommentBlock", "ProcessingInstructionBlock", "InlineCode", "LinkReference"]);

/**
 * Every table at the top level and every `![alt](src)` image in the note, outside the
 * frontmatter. Tables in lists or quotes, reference images and `![[embeds]]` stay text.
 */
export function findBlocks(state: EditorState): LiveBlock[] {
  const out: LiveBlock[] = [];
  const doc = state.doc;
  const fmEnd = frontmatterRange(doc)?.to ?? -1;
  syntaxTree(state).iterate({
    enter(ref) {
      const name = ref.type.name;
      if (name === "Document") return;
      if (ref.from <= fmEnd) return ref.to > fmEnd ? undefined : false;
      if (LEAF_BLOCKS.has(name)) return false;
      if (name === "Table") {
        const top = ref.node.parent?.name === "Document";
        if (top) out.push({ kind: "table", from: doc.lineAt(ref.from).from, to: doc.lineAt(ref.to).to, block: true });
        return false;
      }
      if (name === "Image") {
        if (!ref.node.getChild("URL")) return false;
        const first = doc.lineAt(ref.from);
        const last = doc.lineAt(ref.to);
        const alone =
          doc.sliceString(first.from, ref.from).trim() === "" && doc.sliceString(ref.to, last.to).trim() === "";
        out.push(alone ? { kind: "image", from: first.from, to: last.to, block: true } : { kind: "image", from: ref.from, to: ref.to, block: false });
        return false;
      }
    },
  });
  return out;
}

/** The blocks to render: those the selection is not in (for an inline image, not touching). */
export function shownBlocks(state: EditorState, blocks: readonly LiveBlock[]): LiveBlock[] {
  return blocks.filter((b) =>
    b.block ? !cursorInBlock(state, b.from, b.to) : !state.selection.ranges.some((r) => r.from <= b.to && r.to >= b.from),
  );
}

/**
 * Ticks or unticks the task whose `[` is at `pos`: one character, as its own undo step, with
 * the selection left where it is. Null when `pos` is not a task marker.
 */
export function toggleTask(state: EditorState, pos: number): TransactionSpec | null {
  const node = syntaxTree(state).resolveInner(pos, 1);
  if (node.name !== "TaskMarker" || node.from !== pos) return null;
  const insert = state.sliceDoc(pos + 1, pos + 2) === " " ? "x" : " ";
  return {
    changes: { from: pos + 1, to: pos + 2, insert },
    annotations: [isolateHistory.of("full"), Transaction.userEvent.of("input.toggle")],
  };
}

/**
 * Where the cursor goes for a click on cell `col` of row `row` (0 the header) of the table
 * starting at `tableFrom`: the start of the cell's text, the end of the row for a cell the
 * row doesn't have, the end of the table for a row past it.
 */
export function tableCellPos(state: EditorState, tableFrom: number, row: number, col: number): number {
  let node: SyntaxNode | null = syntaxTree(state).resolveInner(tableFrom, 1);
  while (node && node.name !== "Table") node = node.parent;
  if (!node) return tableFrom;
  const rows = node.getChildren("TableHeader").concat(node.getChildren("TableRow"));
  const r = rows[row];
  if (!r) return node.to;
  const delims = r.getChildren("TableDelimiter");
  const leading = delims.length > 0 && delims[0].from === r.from;
  const after = leading ? delims[col] : col === 0 ? null : delims[col - 1];
  let pos = after ? after.to : leading || col > 0 ? r.to : r.from;
  while (pos < r.to && /[ \t]/.test(state.sliceDoc(pos, pos + 1))) pos++;
  return pos;
}
