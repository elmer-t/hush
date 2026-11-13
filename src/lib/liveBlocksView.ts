import { syntaxTree } from "@codemirror/language";
import { StateField, type EditorState, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import { findBlocks, shownBlocks, tableCellPos, toggleTask, type LiveBlock } from "$lib/liveBlocks";
import type { LiveWidget } from "$lib/liveDecorations";
import { renderMarkdown } from "$lib/markdown";

/**
 * The CodeMirror side of the Live editor's block elements (#135): widgets, the state field
 * for tables and images, and their theme. What to show comes from `liveBlocks.ts`.
 */

class BulletWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const dom = document.createElement("span");
    dom.className = "cm-live-bullet";
    dom.textContent = "•";
    return dom;
  }
}

class RuleWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const dom = document.createElement("span");
    dom.className = "cm-live-hr";
    return dom;
  }
}

/** A task's checkbox. Clicking it ticks the task in the source and leaves the cursor alone. */
class TaskWidget extends WidgetType {
  constructor(
    readonly checked: boolean,
    /** The offset of the task's `[` from the widget's start. */
    readonly marker: number,
  ) {
    super();
  }
  eq(other: TaskWidget) {
    return other.checked === this.checked && other.marker === this.marker;
  }
  toDOM(view: EditorView) {
    const dom = document.createElement("span");
    dom.className = "cm-live-task";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = this.checked;
    box.tabIndex = -1;
    box.setAttribute("aria-label", this.checked ? "Done" : "Not done");
    dom.append(box);
    // The source is the state: the box shows it, and a click changes the source, not the box.
    dom.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (e.button !== 0) return;
      const spec = toggleTask(view.state, view.posAtDOM(dom) + this.marker);
      if (spec) view.dispatch(spec);
    });
    dom.addEventListener("click", (e) => e.preventDefault());
    return dom;
  }
  ignoreEvent() {
    return true;
  }
}

/** The title of a callout that has none written: its type, in place of the `[!type]` marker. */
class CalloutLabelWidget extends WidgetType {
  constructor(readonly label: string) {
    super();
  }
  eq(other: CalloutLabelWidget) {
    return other.label === this.label;
  }
  toDOM() {
    const dom = document.createElement("span");
    dom.className = "cm-live-callout-label";
    dom.textContent = this.label;
    return dom;
  }
}

const BULLET = Decoration.replace({ widget: new BulletWidget() });
const RULE = Decoration.replace({ widget: new RuleWidget() });

/** The decoration for an inline widget spec from `liveDecorations.ts`. */
export function liveWidget(widget: LiveWidget): Decoration {
  switch (widget.type) {
    case "bullet":
      return BULLET;
    case "rule":
      return RULE;
    case "task":
      return Decoration.replace({ widget: new TaskWidget(widget.checked, widget.marker) });
    case "callout":
      return Decoration.replace({ widget: new CalloutLabelWidget(widget.label) });
  }
}

/** Markdown rendered as the preview renders it: the same renderer and sanitiser. */
function rendered(source: string): HTMLTemplateElement {
  const template = document.createElement("template");
  template.innerHTML = renderMarkdown(source);
  return template;
}

/** A table or image, rendered from its source. Clicking it puts the cursor in that source. */
class BlockWidget extends WidgetType {
  constructor(
    readonly kind: LiveBlock["kind"],
    readonly source: string,
    readonly block: boolean,
  ) {
    super();
  }
  eq(other: BlockWidget) {
    return other.kind === this.kind && other.source === this.source && other.block === this.block;
  }
  /** A guess for a table not yet drawn (header and rows at about 37px each). */
  get estimatedHeight() {
    return this.kind === "table" ? this.source.split("\n").length * 37 : -1;
  }
  toDOM(view: EditorView) {
    const dom: HTMLElement = document.createElement(this.block ? "div" : "span");
    const content = rendered(this.source).content;
    if (this.kind === "table") {
      dom.className = "cm-live-table";
      dom.append(content);
      // Links in the table are Live links (#137): the link handler opens them and shows the bar.
      for (const a of dom.querySelectorAll("a")) a.classList.add(a.classList.contains("wikilink") ? "cm-live-wikilink" : "cm-live-link");
    } else {
      dom.className = this.block ? "cm-live-image-block" : "cm-live-image";
      // The image's paragraph, unwrapped.
      const p = content.firstElementChild;
      dom.append(...(p?.tagName === "P" ? [...p.childNodes] : [content]));
      // Loaded images change the height: have CodeMirror measure again, keeping the scroll.
      for (const img of dom.querySelectorAll("img")) img.addEventListener("load", () => view.requestMeasure());
    }
    dom.addEventListener("mousedown", (e) => {
      if (e.button !== 0 || onLink(e)) return;
      e.preventDefault();
      const from = view.posAtDOM(dom);
      let pos = from;
      const cell = (e.target as Element).closest?.("td, th") as HTMLTableCellElement | null;
      const table = cell?.closest("table");
      if (this.kind === "table" && cell && table) {
        const row = [...table.rows].indexOf(cell.parentElement as HTMLTableRowElement);
        pos = tableCellPos(view.state, from, row, cell.cellIndex);
      }
      view.dispatch({ selection: { anchor: pos } });
      view.focus();
    });
    return dom;
  }
  /** Events on a link go to the editor, for the link handler; the rest are the widget's. */
  ignoreEvent(e: Event) {
    return !onLink(e);
  }
}

const onLink = (e: Event) => e.target instanceof Element && e.target.closest(".cm-live-link, .cm-live-wikilink") !== null;

function blockDecorations(state: EditorState, blocks: readonly LiveBlock[]): DecorationSet {
  const ranges: Range<Decoration>[] = shownBlocks(state, blocks).map((b) =>
    Decoration.replace({ widget: new BlockWidget(b.kind, state.sliceDoc(b.from, b.to), b.block), block: b.block }).range(
      b.from,
      b.to,
    ),
  );
  return Decoration.set(ranges, true);
}

type BlocksValue = { blocks: LiveBlock[]; decorations: DecorationSet };

/**
 * Tables and images, from a state field: block widgets may not come from a view plugin, and
 * covering the whole note keeps measured heights in the height map, so content above the
 * cursor doesn't change height as it scrolls. Widgets are only drawn (and rendered) in view.
 */
const blocks = StateField.define<BlocksValue>({
  create(state) {
    const found = findBlocks(state);
    return { blocks: found, decorations: blockDecorations(state, found) };
  },
  update(value, tr) {
    const reparsed = syntaxTree(tr.startState) !== syntaxTree(tr.state);
    if (!tr.docChanged && !reparsed && !tr.selection) return value;
    const found = tr.docChanged || reparsed ? findBlocks(tr.state) : value.blocks;
    return { blocks: found, decorations: blockDecorations(tr.state, found) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decorations),
});

/** The preview's block styles (`Preview.svelte`), for lines and widgets. No vertical margins on widgets: CodeMirror can't measure them. */
const theme = EditorView.theme({
  ".cm-line.cm-live-li-2": { paddingLeft: "calc(24px + 22px)" },
  ".cm-line.cm-live-li-3": { paddingLeft: "calc(24px + 44px)" },
  ".cm-line.cm-live-li-4": { paddingLeft: "calc(24px + 66px)" },
  ".cm-line.cm-live-li-5": { paddingLeft: "calc(24px + 88px)" },
  ".cm-line.cm-live-li-6": { paddingLeft: "calc(24px + 110px)" },
  ".cm-live-bullet": { display: "inline-block", minWidth: "0.6em", color: "var(--text)" },
  ".cm-live-task": { display: "inline-block", minWidth: "1.2em" },
  ".cm-live-task input": { margin: "0", verticalAlign: "-0.1em", cursor: "pointer", accentColor: "var(--accent)" },
  ".cm-line.cm-live-quote, .cm-line.cm-live-quote-nested": { color: "var(--text-2)" },
  ".cm-line.cm-live-quote": {
    paddingLeft: "calc(24px + 3px + 16px)",
    backgroundImage: "linear-gradient(to right, transparent 24px, var(--border-strong) 24px, var(--border-strong) 27px, transparent 27px)",
  },
  ".cm-line.cm-live-quote.cm-live-quote-nested": {
    paddingLeft: "calc(24px + 2 * (3px + 16px))",
    backgroundImage:
      "linear-gradient(to right, transparent 24px, var(--border-strong) 24px, var(--border-strong) 27px, transparent 27px, transparent 43px, var(--border-strong) 43px, var(--border-strong) 46px, transparent 46px)",
  },
  // Callouts (#184): the bar in the kind's colour and a tint behind the text; `app.css` sets
  // `--callout-color` and the title's icon per kind, for these lines and for the preview.
  ".cm-line.cm-live-callout": {
    color: "var(--text)",
    backgroundImage:
      "linear-gradient(to right, transparent 24px, var(--callout-color) 24px, var(--callout-color) 27px, var(--callout-tint) 27px, var(--callout-tint) calc(100% - 24px), transparent calc(100% - 24px))",
  },
  ".cm-line.cm-live-callout.cm-live-quote-nested": {
    backgroundImage:
      "linear-gradient(to right, transparent 24px, var(--border-strong) 24px, var(--border-strong) 27px, transparent 27px, transparent 43px, var(--callout-color) 43px, var(--callout-color) 46px, var(--callout-tint) 46px, var(--callout-tint) calc(100% - 24px), transparent calc(100% - 24px))",
  },
  ".cm-line.cm-live-callout-title": { color: "var(--callout-color)", fontWeight: "600" },
  ".cm-line.cm-live-codeblock": {
    margin: "0 24px",
    padding: "0 14px",
    background: "var(--code)",
    borderLeft: "1px solid var(--border)",
    borderRight: "1px solid var(--border)",
    fontFamily: "var(--mono)",
    fontSize: "0.8em",
    lineHeight: "1.6",
  },
  ".cm-line.cm-live-codeblock-first": {
    borderTop: "1px solid var(--border)",
    borderTopLeftRadius: "8px",
    borderTopRightRadius: "8px",
    paddingTop: "4px",
  },
  ".cm-line.cm-live-codeblock-last": {
    borderBottom: "1px solid var(--border)",
    borderBottomLeftRadius: "8px",
    borderBottomRightRadius: "8px",
    paddingBottom: "4px",
  },
  // A hidden fence is the box's top or bottom padding.
  ".cm-line.cm-live-fence": { lineHeight: "8px" },
  ".cm-live-codeblock .cm-live-syntax": { fontFamily: "var(--mono)" },
  ".cm-live-hr": { display: "inline-block", width: "100%", verticalAlign: "middle", borderTop: "1px solid var(--border)" },
  ".cm-live-table": { padding: "4px 24px 12px", fontFamily: "var(--sans)", fontSize: "14px", lineHeight: "1.5", cursor: "text", overflowX: "auto" },
  ".cm-live-table table": { borderCollapse: "collapse", margin: "0" },
  ".cm-live-table th, .cm-live-table td": { border: "1px solid var(--border)", padding: "6px 10px" },
  ".cm-live-table code": {
    fontFamily: "var(--mono)",
    fontSize: "0.9em",
    background: "var(--code)",
    border: "1px solid var(--border)",
    padding: "0.1em 0.3em",
    borderRadius: "4px",
  },
  ".cm-live-table a": { color: "var(--accent)", textDecoration: "underline", textUnderlineOffset: "3px" },
  ".cm-live-table img, .cm-live-image img, .cm-live-image-block img": { maxWidth: "100%" },
  ".cm-live-image-block": { padding: "0 24px", cursor: "text" },
  ".cm-live-image-block img": { display: "block" },
  ".cm-live-image img": { verticalAlign: "bottom" },
});

/** Tables and images as widgets, and the block styles. */
export function liveBlocks(): Extension {
  return [blocks, theme];
}
