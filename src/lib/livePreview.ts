import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { syntaxTree } from "@codemirror/language";
import { StateField, type EditorState, type Extension, type Range } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { buildLive, frontmatterView, inlineHandlers, type LiveSpec } from "$lib/liveDecorations";
import { codeHighlighting, codeLanguage } from "$lib/codeHighlight";
import { blockHandlers } from "$lib/liveBlocks";
import { liveBlocks, liveWidget } from "$lib/liveBlocksView";
import { liveMermaid } from "$lib/liveMermaid";
import { wikilinkMarkdown } from "$lib/wikilink";

/**
 * The Live editor (#134): the note's markdown, formatted in place by decorations over the
 * unchanged source. What is hidden or styled comes from `liveDecorations.ts`; this module
 * turns it into CodeMirror decorations for the visible ranges and themes them like the preview.
 */

/** Markdown with GFM and `[[wikilinks]]`, as the preview reads it; code in fences in its own language (#180). */
export function liveLanguage(): Extension {
  return markdown({ base: markdownLanguage, extensions: [wikilinkMarkdown], codeLanguages: codeLanguage });
}

const HIDE = Decoration.replace({});
const marks = new Map<string, Decoration>();
const lines = new Map<string, Decoration>();

function toDecoration(spec: LiveSpec): Range<Decoration> {
  switch (spec.kind) {
    case "hide":
      return HIDE.range(spec.from, spec.to);
    case "line": {
      let d = lines.get(spec.class);
      if (!d) lines.set(spec.class, (d = Decoration.line({ class: spec.class })));
      return d.range(spec.from);
    }
    case "mark": {
      if (spec.attributes) return Decoration.mark({ class: spec.class, attributes: spec.attributes }).range(spec.from, spec.to);
      let d = marks.get(spec.class);
      if (!d) marks.set(spec.class, (d = Decoration.mark({ class: spec.class })));
      return d.range(spec.from, spec.to);
    }
    case "widget":
      return liveWidget(spec.widget).range(spec.from, spec.to);
  }
}

/** Inline formatting (#134) and block elements (#135). */
const liveHandlers = { ...inlineHandlers, ...blockHandlers };

function decorate(view: EditorView): DecorationSet {
  return Decoration.set(buildLive(view.state, view.visibleRanges, liveHandlers).map(toDecoration), true);
}

const inline = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = decorate(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.selectionSet || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
        this.decorations = decorate(u.view);
      }
    }
  },
  { decorations: (p) => p.decorations },
);

/** The collapsed frontmatter: one muted line naming its keys. Clicking it opens it. */
class PropertiesWidget extends WidgetType {
  constructor(readonly keys: string) {
    super();
  }
  eq(other: PropertiesWidget) {
    return other.keys === this.keys;
  }
  toDOM(view: EditorView) {
    const dom = document.createElement("div");
    dom.className = "cm-live-properties";
    dom.title = "Show the frontmatter";
    const label = document.createElement("span");
    label.className = "cm-live-properties-label";
    label.textContent = "Properties";
    dom.append(label);
    if (this.keys) {
      const keys = document.createElement("span");
      keys.className = "cm-live-properties-keys";
      keys.textContent = this.keys;
      dom.append(keys);
    }
    dom.addEventListener("mousedown", (e) => {
      e.preventDefault();
      view.dispatch({ selection: { anchor: 0 } });
      view.focus();
    });
    return dom;
  }
  ignoreEvent() {
    return true;
  }
}

const FM_LINE = Decoration.line({ class: "cm-live-frontmatter" });

function frontmatterDecorations(state: EditorState): DecorationSet {
  const fm = frontmatterView(state);
  if (!fm) return Decoration.none;
  if (fm.collapsed) {
    const widget = new PropertiesWidget(fm.keys.join(", "));
    return Decoration.set(Decoration.replace({ widget, block: true }).range(fm.from, fm.to));
  }
  const ranges: Range<Decoration>[] = [];
  for (let n = 1; n <= state.doc.lineAt(fm.to).number; n++) ranges.push(FM_LINE.range(state.doc.line(n).from));
  return Decoration.set(ranges);
}

/**
 * Frontmatter decorations come from a state field: a block that replaces line breaks may not
 * come from a view plugin. They are cheap (the block is at the top), so they cover it all.
 */
const frontmatter = StateField.define<DecorationSet>({
  create: frontmatterDecorations,
  update: (value, tr) => (tr.docChanged || tr.selection ? frontmatterDecorations(tr.state) : value),
  provide: (f) => EditorView.decorations.from(f),
});

/** The preview's typography (`Preview.svelte`, `large`), for the editor. */
const theme = EditorView.theme({
  "&": { height: "100%", color: "var(--text)", backgroundColor: "transparent" },
  ".cm-scroller": { fontFamily: "var(--serif)", fontSize: "18px", lineHeight: "1.7" },
  ".cm-content": {
    maxWidth: "calc(680px + 48px)",
    margin: "0 auto",
    padding: "28px 0",
    caretColor: "var(--text)",
    overflowWrap: "anywhere",
  },
  ".cm-line": { padding: "0 24px" },
  ".cm-cursor": { borderLeftColor: "var(--text)" },
  ".cm-line.cm-live-h1, .cm-line.cm-live-h2, .cm-line.cm-live-h3, .cm-line.cm-live-h4, .cm-line.cm-live-h5, .cm-line.cm-live-h6":
    { fontWeight: "600", lineHeight: "1.3", paddingTop: "0.5em", paddingBottom: "0.15em" },
  ".cm-line.cm-live-h1": { fontSize: "26px" },
  ".cm-line.cm-live-h2": { fontSize: "21px" },
  ".cm-line.cm-live-h3": { fontSize: "18px" },
  ".cm-line.cm-live-h4": { fontSize: "1em" },
  ".cm-line.cm-live-h5": { fontSize: "0.83em" },
  ".cm-line.cm-live-h6": { fontSize: "0.67em" },
  ".cm-live-strong": { fontWeight: "600" },
  ".cm-live-em": { fontStyle: "italic" },
  ".cm-live-strike": { textDecoration: "line-through" },
  ".cm-live-code": {
    fontFamily: "var(--mono)",
    fontSize: "0.8em",
    background: "var(--code)",
    border: "1px solid var(--border)",
    padding: "0.1em 0.3em",
    borderRadius: "4px",
  },
  ".cm-live-link, .cm-live-wikilink": {
    color: "var(--accent)",
    textDecoration: "underline",
    textDecorationThickness: "1px",
    textUnderlineOffset: "3px",
  },
  ".cm-live-wikilink": { textDecorationStyle: "dotted" },
  ".cm-live-syntax": { color: "var(--muted)", fontStyle: "normal", fontWeight: "normal" },
  ".cm-live-code .cm-live-syntax": { fontFamily: "var(--mono)" },
  ".cm-live-properties": {
    display: "flex",
    gap: "8px",
    margin: "0 24px 8px",
    padding: "2px 0 6px",
    borderBottom: "1px solid var(--border)",
    fontFamily: "var(--sans)",
    fontSize: "13px",
    color: "var(--muted)",
    cursor: "text",
    overflow: "hidden",
    whiteSpace: "nowrap",
    textOverflow: "ellipsis",
  },
  ".cm-live-properties-label": { fontWeight: "600" },
  ".cm-line.cm-live-frontmatter": { fontFamily: "var(--mono)", fontSize: "13px", color: "var(--muted)" },
});

/** Everything the Live editor adds on top of the shared editor setup. */
export function livePreview(): Extension {
  return [liveLanguage(), codeHighlighting(), inline, frontmatter, theme, liveMermaid(), liveBlocks()];
}
