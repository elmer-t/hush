<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { Compartment, EditorSelection, EditorState, type Extension } from "@codemirror/state";
  import { EditorView, drawSelection, highlightActiveLine, keymap } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import {
    closeSearchPanel,
    findNext,
    findPrevious,
    getSearchQuery,
    highlightSelectionMatches,
    openSearchPanel,
    replaceAll,
    replaceNext,
    search,
    searchKeymap,
    searchPanelOpen,
    SearchQuery,
    setSearchQuery,
  } from "@codemirror/search";
  import type { FindSpec } from "$lib/find";
  import type { EditorMode } from "$lib/layout";
  import LinkBar from "$lib/LinkBar.svelte";
  import { openLink } from "$lib/linkOpen";
  import { frontmatterRange } from "$lib/liveDecorations";
  import { liveLinks } from "$lib/liveLinks";
  import { livePreview } from "$lib/livePreview";
  import { tags as t } from "@lezer/highlight";

  type Props = {
    doc: string;
    /** Source (the monospaced markdown) or Live (formatted in place, #134); same document and history. */
    mode?: EditorMode;
    onchange: (doc: string) => void;
    /** The current find match (-1 for none) and the number of matches, while find is on. */
    onfindresult?: (current: number, total: number) => void;
    /** A `[[wikilink]]` in the Live view was clicked at viewport position `x`, `y`. */
    onwikilink?: (target: string, x: number, y: number) => void;
  };
  let { doc, mode = "source", onchange, onfindresult, onwikilink }: Props = $props();

  // Links in Live open as in the preview, and show where they go in the shared link bar (#137).
  let linkBar: ReturnType<typeof LinkBar>;
  const links = liveLinks((link, x, y) => openLink(link, x, y, onwikilink), {
    show: (el, link, x, y) => linkBar?.show(el, link, x, y),
    hide: () => linkBar?.hide(),
    shown: () => linkBar?.shown() ?? null,
    reset: () => linkBar?.reset(),
  });

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  const highlight = HighlightStyle.define([
    // The source is monospaced, so headings stand out by weight alone.
    { tag: [t.heading1, t.heading2, t.heading3, t.heading4, t.heading5, t.heading6], fontWeight: "600" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.strong, fontWeight: "600" },
    { tag: t.strikethrough, textDecoration: "line-through" },
    { tag: [t.link, t.url], color: "var(--accent)" },
    { tag: [t.processingInstruction, t.meta, t.contentSeparator, t.quote, t.list], color: "var(--muted)" },
  ]);

  const sourceTheme = EditorView.theme({
    "&": { height: "100%", color: "var(--text)", backgroundColor: "var(--code)" },
    ".cm-scroller": { fontFamily: "var(--mono)", fontSize: "13px", lineHeight: "1.75" },
    ".cm-content": { padding: "20px 0", caretColor: "var(--text)" },
    ".cm-line": { padding: "0 24px" },
    ".cm-cursor": { borderLeftColor: "var(--text)" },
    ".cm-activeLine": { backgroundColor: "var(--hover)" },
  });

  // Both modes.
  const theme = EditorView.theme({
    "&.cm-focused": { outline: "none" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "var(--selection)",
    },
    // Find: the note's find bar drives CodeMirror's search, whose own panel stays hidden.
    ".cm-panels": { display: "none" },
    "& .cm-searchMatch": { backgroundColor: "var(--mark)", color: "var(--mark-text)", borderRadius: "2px" },
    "& .cm-searchMatch.cm-searchMatch-selected": {
      backgroundColor: "var(--mark-current)",
      color: "var(--mark-current-text)",
    },
  });

  // The mode's own extensions; switching them keeps the document, selection and undo history.
  const modeConfig = new Compartment();
  const modeExtensions = (m: EditorMode): Extension =>
    m === "live"
      ? [livePreview(), links]
      : [highlightActiveLine(), markdown({ base: markdownLanguage }), syntaxHighlighting(highlight), sourceTheme];

  // The find bar replaces CodeMirror's panel: Ctrl+F and Esc are handled by the note.
  const findKeys = searchKeymap.filter((b) => b.key !== "Mod-f" && b.key !== "Escape");

  /** Reports the current match and the count after each change that can move them. */
  function reportFind(state: EditorState) {
    if (!onfindresult) return;
    const query = getSearchQuery(state);
    if (!query.valid) return onfindresult(-1, 0);
    const { from, to } = state.selection.main;
    let total = 0;
    let current = -1;
    const cursor = query.getCursor(state);
    for (let m = cursor.next(); !m.done; m = cursor.next()) {
      if (m.value.from === from && m.value.to === to) current = total;
      total++;
    }
    onfindresult(current, total);
  }

  let shownMode: EditorMode = untrack(() => mode);

  onMount(() => {
    const initial = untrack(() => doc);
    // Live starts below the frontmatter, so it opens collapsed.
    const fm = shownMode === "live" ? frontmatterRange(EditorState.create({ doc: initial }).doc) : null;
    const start = fm ? Math.min(fm.to + 1, initial.length) : 0;
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: initial,
        selection: EditorSelection.cursor(start),
        extensions: [
          history(),
          drawSelection(),
          highlightSelectionMatches(),
          EditorView.lineWrapping,
          modeConfig.of(modeExtensions(shownMode)),
          theme,
          search({ top: true, createPanel: () => ({ dom: document.createElement("div") }) }),
          keymap.of([...defaultKeymap, ...historyKeymap, ...findKeys, indentWithTab]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onchange(u.state.doc.toString());
            if (!searchPanelOpen(u.state)) return;
            const queryChanged = !getSearchQuery(u.state).eq(getSearchQuery(u.startState));
            if (u.docChanged || u.selectionSet || queryChanged || !searchPanelOpen(u.startState)) reportFind(u.state);
          }),
        ],
      }),
    });
    return () => view?.destroy();
  });

  // The line at the top of the editor, before the layout around it changes with the mode.
  let anchor = 0;
  $effect.pre(() => {
    if (mode !== shownMode && view) untrack(() => (anchor = view!.lineBlockAtHeight(view!.scrollDOM.scrollTop).from));
  });

  // Switches the mode and keeps that line at the top. Only the display changes: no document change.
  $effect(() => {
    const next = mode;
    if (next === shownMode || !view) return;
    shownMode = next;
    untrack(() =>
      view!.dispatch({
        effects: [modeConfig.reconfigure(modeExtensions(next)), EditorView.scrollIntoView(anchor, { y: "start" })],
      }),
    );
  });

  /** Replaces the document with `next` as one undoable change covering only what differs. */
  export function replace(next: string) {
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === next) return;
    let start = 0;
    const max = Math.min(current.length, next.length);
    while (start < max && current[start] === next[start]) start++;
    let endCurrent = current.length;
    let endNext = next.length;
    while (endCurrent > start && endNext > start && current[endCurrent - 1] === next[endNext - 1]) {
      endCurrent--;
      endNext--;
    }
    view.dispatch({ changes: { from: start, to: endCurrent, insert: next.slice(start, endNext) } });
  }

  export function focus() {
    view?.focus();
  }

  /**
   * Searches for `spec`, highlighting every match, or stops searching when it is null. A new
   * query (not a new replacement) selects the first match from the cursor, without focusing.
   */
  export function setFind(spec: FindSpec | null) {
    if (!view) return;
    if (!spec) {
      closeSearchPanel(view);
      return;
    }
    const query = new SearchQuery({
      search: spec.query,
      caseSensitive: spec.caseSensitive,
      wholeWord: spec.wholeWord,
      regexp: spec.regexp,
      literal: true,
      replace: spec.replace,
    });
    const before = getSearchQuery(view.state);
    const wasOpen = searchPanelOpen(view.state);
    if (!wasOpen) openSearchPanel(view);
    view.dispatch({ effects: setSearchQuery.of(query) });
    const sameSearch =
      wasOpen &&
      before.search === query.search &&
      before.caseSensitive === query.caseSensitive &&
      before.wholeWord === query.wholeWord &&
      before.regexp === query.regexp;
    if (sameSearch || !query.valid) return;
    // The first match from the cursor, wrapping to the first in the note.
    const { from } = view.state.selection.main;
    let next = query.getCursor(view.state, from).next();
    if (next.done) next = query.getCursor(view.state).next();
    if (next.done) return;
    const match = next.value;
    view.dispatch({
      selection: EditorSelection.single(match.from, match.to),
      effects: EditorView.scrollIntoView(match.from, { y: "center" }),
      userEvent: "select.search",
    });
  }

  /** Selects the next (`dir` 1) or previous (-1) match, wrapping around. */
  export function findStep(dir: 1 | -1) {
    if (view && searchPanelOpen(view.state)) (dir === 1 ? findNext : findPrevious)(view);
  }

  /** Replaces the current match and moves to the next, or replaces every match. */
  export function replaceMatch(all: boolean) {
    if (view && searchPanelOpen(view.state)) (all ? replaceAll : replaceNext)(view);
  }

  /** The selected text while the editor has focus, if it fits on one line. */
  export function selectedText(): string {
    if (!view?.hasFocus) return "";
    const { from, to } = view.state.selection.main;
    const text = view.state.sliceDoc(from, to);
    return text.length > 200 || text.includes("\n") ? "" : text;
  }
</script>

<div class="editor" bind:this={host}></div>
<LinkBar bind:this={linkBar} />

<style>
  .editor {
    height: 100%;
  }
</style>
