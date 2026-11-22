import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

/**
 * The editor's themes and highlight style, made once for the app. Each call of
 * `EditorView.theme` or `HighlightStyle.define` makes a style module with class names of
 * its own, and an editor adds the modules it has not seen to the document's style sheet,
 * where they stay. Made per editor (one is mounted for every note that opens), they would
 * add rules on every open: the browser then works out the style of the whole page again,
 * which takes longer the more rows the note tree shows, and the rules pile up.
 */

/** Source: the markdown's own tokens. */
export const sourceHighlight = HighlightStyle.define([
  // The source is monospaced, so headings stand out by weight alone.
  { tag: [t.heading1, t.heading2, t.heading3, t.heading4, t.heading5, t.heading6], fontWeight: "600" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "600" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: [t.link, t.url], color: "var(--accent)" },
  { tag: [t.processingInstruction, t.meta, t.contentSeparator, t.quote, t.list], color: "var(--muted)" },
]);

/** Source: the monospaced page. */
export const sourceTheme = EditorView.theme({
  "&": { height: "100%", color: "var(--text)", backgroundColor: "var(--code)" },
  ".cm-scroller": { fontFamily: "var(--mono)", fontSize: "13px", lineHeight: "1.75" },
  ".cm-content": { padding: "20px 0", caretColor: "var(--text)" },
  ".cm-line": { padding: "0 24px" },
  ".cm-cursor": { borderLeftColor: "var(--text)" },
  ".cm-activeLine": { backgroundColor: "var(--hover)" },
});

/** Both modes. */
export const editorTheme = EditorView.theme({
  "&.cm-focused": { outline: "none" },
  // The first selector is as specific as CodeMirror's own rule for a focused editor, which
  // otherwise wins with its light lavender, in the dark theme too (#245).
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "var(--text-selection)",
  },
  // Find: the note's find bar drives CodeMirror's search, whose own panel stays hidden.
  ".cm-panels": { display: "none" },
  "& .cm-searchMatch": { backgroundColor: "var(--mark)", color: "var(--mark-text)", borderRadius: "2px" },
  "& .cm-searchMatch.cm-searchMatch-selected": {
    backgroundColor: "var(--mark-current)",
    color: "var(--mark-current-text)",
  },
});
