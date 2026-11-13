import { syntaxTree } from "@codemirror/language";
import { Facet, StateEffect, StateField, type EditorState, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet } from "@codemirror/view";
import { frontmatterRange } from "$lib/liveDecorations";
import { diagram, readTheme } from "$lib/mermaid";
import { onSchemeChange } from "$lib/theme";

/**
 * Mermaid diagrams in the Live editor (#136). A ```mermaid fence is shown as its rendered
 * diagram while the selection is outside it; while the selection is on one of its lines the
 * source is shown, editable, with the diagram kept below it and updated after a pause in typing.
 * Rendering goes through `mermaid.ts`: one queue and one cache for every diagram.
 *
 * Only fences at the top level of the note count: fences in lists and quotes are left to the
 * block handling of #135. Other code fences are #135's too.
 */

export type MermaidFence = {
  /** Start of the opening fence's line. */
  from: number;
  /** End of the closing fence's line (or of the note, for an unclosed fence). */
  to: number;
  /** Where a click on the diagram puts the cursor: the start of the source, or the end of the opening line. */
  sourceFrom: number;
  /** The diagram's source, with the fence's indent taken off, as markdown-it reads a fence. */
  source: string;
  /** Whether the selection touches one of its lines, so the source is shown. */
  open: boolean;
};

const isMermaid = (info: string) => info.trim().split(/\s+/)[0].toLowerCase() === "mermaid";

/** The top-level mermaid fences of the note, in order. */
export function mermaidFences(state: EditorState): MermaidFence[] {
  const doc = state.doc;
  const fmEnd = frontmatterRange(doc)?.to ?? -1;
  const fences: MermaidFence[] = [];
  const cursor = syntaxTree(state).cursor();
  if (!cursor.firstChild()) return fences;
  do {
    if (cursor.name !== "FencedCode" || cursor.from <= fmEnd) continue;
    const node = cursor.node;
    const info = node.getChild("CodeInfo");
    if (!info || !isMermaid(state.sliceDoc(info.from, info.to))) continue;
    const open = doc.lineAt(node.from);
    const marks = node.getChildren("CodeMark");
    const close = marks.length > 1 ? doc.lineAt(marks[marks.length - 1].from) : null;
    const last = close ? close.number - 1 : doc.lineAt(node.to).number;
    // markdown-it takes up to the opening fence's indent off each source line.
    const indent = /^ */.exec(open.text)![0].length;
    let source = "";
    for (let n = open.number + 1; n <= last; n++) {
      const text = doc.line(n).text;
      let cut = 0;
      while (cut < indent && text[cut] === " ") cut++;
      // Only the note's last line, in an unclosed fence, has no line break after it.
      source += text.slice(cut) + (n < doc.lines ? "\n" : "");
    }
    const from = open.from;
    const to = close ? close.to : doc.lineAt(node.to).to;
    fences.push({
      from,
      to,
      sourceFrom: open.number < last ? open.to + 1 : open.to,
      source,
      open: state.selection.ranges.some((r) => r.from <= to && r.to >= from),
    });
  } while (cursor.nextSibling());
  return fences;
}

/** Last measured heights by theme and source, so a diagram scrolled back into view keeps its place. */
const heights = new Map<string, number>();
const handles = new WeakMap<HTMLElement, ReturnType<typeof diagram>>();

export class MermaidWidget extends WidgetType {
  constructor(
    readonly source: string,
    readonly theme: string,
    /** Shown below the source being edited, rather than in place of the fence. */
    readonly editing: boolean,
  ) {
    super();
  }

  /**
   * Same diagram, same theme: the DOM is kept, so typing elsewhere never re-renders it. A change
   * of `editing` alone goes through `updateDOM`, which only restyles the element.
   */
  eq(other: MermaidWidget) {
    return other.source === this.source && other.theme === this.theme && other.editing === this.editing;
  }

  get estimatedHeight() {
    return heights.get(`${this.theme}\n${this.source}`) ?? -1;
  }

  toDOM(view: EditorView) {
    const dom = document.createElement("div");
    dom.className = "cm-live-mermaid mermaid-block";
    dom.title = "Edit the diagram";
    const handle = diagram(dom, () =>
      view.requestMeasure({
        read: () => dom.offsetHeight,
        write: (h) => {
          if (h > 0) heights.set(`${this.theme}\n${this.source}`, h);
        },
      }),
    );
    handles.set(dom, handle);
    this.mark(dom);
    handle.set(this.source);
    dom.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const pos = view.posAtDOM(dom);
      const fence = mermaidFences(view.state).find((f) => f.from <= pos && pos <= f.to);
      if (fence) view.dispatch({ selection: { anchor: fence.sourceFrom }, scrollIntoView: true });
      view.focus();
    });
    // Links in diagrams do nothing for now; part 4 (#137) decides what they open.
    dom.addEventListener("click", (e) => e.preventDefault());
    return dom;
  }

  /** The source was edited (or the theme changed): render into the same element, the old diagram faded meanwhile. */
  updateDOM(dom: HTMLElement) {
    const handle = handles.get(dom);
    if (!handle) return false;
    this.mark(dom);
    handle.set(this.source, true);
    return true;
  }

  private mark(dom: HTMLElement) {
    dom.classList.toggle("cm-live-mermaid-editing", this.editing);
  }

  destroy(dom: HTMLElement) {
    handles.get(dom)?.destroy();
  }

  ignoreEvent() {
    return true;
  }
}

const SOURCE_LINE = Decoration.line({ class: "cm-live-mermaid-source" });

/** The decorations for every mermaid fence, with `theme` as the current theme's key. */
export function mermaidDecorations(state: EditorState, theme: string): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  for (const fence of mermaidFences(state)) {
    if (!fence.open) {
      const widget = new MermaidWidget(fence.source, theme, false);
      ranges.push(Decoration.replace({ widget, block: true }).range(fence.from, fence.to));
      continue;
    }
    for (let n = state.doc.lineAt(fence.from).number; n <= state.doc.lineAt(fence.to).number; n++) {
      ranges.push(SOURCE_LINE.range(state.doc.line(n).from));
    }
    const widget = new MermaidWidget(fence.source, theme, true);
    ranges.push(Decoration.widget({ widget, block: true, side: 1 }).range(fence.to));
  }
  return Decoration.set(ranges, true);
}

/** The theme's key changed (light/dark): diagrams render again. */
export const mermaidThemeChanged = StateEffect.define<string>();

/** Reads the current theme's key; the editor uses `readTheme` from `mermaid.ts`. */
const themeReader = Facet.define<() => string, () => string>({
  combine: (values) => values[0] ?? (() => ""),
});

type MermaidState = { theme: string; decorations: DecorationSet };

/** Block decorations, which may replace line breaks, have to come from the state, not a view plugin. */
export const mermaidField = StateField.define<MermaidState>({
  create(state) {
    const theme = state.facet(themeReader)();
    return { theme, decorations: mermaidDecorations(state, theme) };
  },
  update(value, tr) {
    let theme = value.theme;
    for (const e of tr.effects) if (e.is(mermaidThemeChanged)) theme = e.value;
    if (
      theme === value.theme &&
      !tr.docChanged &&
      !tr.selection &&
      syntaxTree(tr.startState) === syntaxTree(tr.state)
    ) {
      return value;
    }
    return { theme, decorations: mermaidDecorations(tr.state, theme) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decorations),
});

/** Tells the field when the colour scheme changes (the OS's or the user's). */
const schemeWatcher = ViewPlugin.fromClass(
  class {
    stop: () => void;
    constructor(readonly view: EditorView) {
      this.stop = onSchemeChange(this.onChange);
    }
    onChange = () => {
      const theme = this.view.state.facet(themeReader)();
      if (theme !== this.view.state.field(mermaidField).theme) {
        this.view.dispatch({ effects: mermaidThemeChanged.of(theme) });
      }
    };
    destroy() {
      this.stop();
    }
  },
);

/** The diagrams' look in Live. */
const theme = EditorView.theme({
  ".cm-live-mermaid": {
    margin: "0 24px 16px",
    fontFamily: "var(--sans)",
    fontSize: "14px",
    lineHeight: "1.5",
    cursor: "pointer",
    overflowX: "auto",
  },
  ".cm-live-mermaid.cm-live-mermaid-editing": { marginTop: "8px", cursor: "text" },
  ".cm-live-mermaid svg": { display: "block", height: "auto", margin: "0 auto" },
  ".cm-live-mermaid svg p": { margin: "0" },
  ".cm-live-mermaid[data-stale]": { opacity: "0.6" },
  ".cm-live-mermaid > .mermaid-source": {
    margin: "0",
    color: "var(--muted)",
    fontFamily: "var(--mono)",
    fontSize: "13px",
    whiteSpace: "pre-wrap",
  },
  ".cm-live-mermaid .mermaid-error": {
    padding: "10px 14px",
    border: "1px solid var(--danger)",
    borderRadius: "8px",
    background: "var(--danger-soft)",
    cursor: "text",
  },
  ".cm-live-mermaid .mermaid-error strong": { color: "var(--danger)" },
  ".cm-live-mermaid .mermaid-error p": {
    margin: "4px 0 8px",
    fontFamily: "var(--mono)",
    fontSize: "12px",
    whiteSpace: "pre-wrap",
  },
  ".cm-live-mermaid .mermaid-error pre": {
    maxHeight: "240px",
    margin: "0",
    overflow: "auto",
    fontFamily: "var(--mono)",
    fontSize: "12px",
  },
  ".cm-line.cm-live-mermaid-source": { fontFamily: "var(--mono)", fontSize: "14px", lineHeight: "1.6" },
});

/** Mermaid diagrams for the Live editor. `readKey` gives the current theme's key. */
export function liveMermaid(readKey: () => string = () => readTheme().key): Extension {
  const extensions: Extension[] = [themeReader.of(readKey), mermaidField, theme];
  if (typeof window !== "undefined") extensions.push(schemeWatcher);
  return extensions;
}
