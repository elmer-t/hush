<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { EditorState } from "@codemirror/state";
  import { EditorView, drawSelection, highlightActiveLine, keymap } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
  import { tags as t } from "@lezer/highlight";

  let { doc, onchange }: { doc: string; onchange: (doc: string) => void } = $props();

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

  const theme = EditorView.theme({
    "&": { height: "100%", color: "var(--text)", backgroundColor: "var(--code)" },
    "&.cm-focused": { outline: "none" },
    ".cm-scroller": { fontFamily: "var(--mono)", fontSize: "13px", lineHeight: "1.75" },
    ".cm-content": { padding: "20px 0", caretColor: "var(--text)" },
    ".cm-line": { padding: "0 24px" },
    ".cm-cursor": { borderLeftColor: "var(--text)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "var(--selection)",
    },
    ".cm-activeLine": { backgroundColor: "var(--hover)" },
  });

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: untrack(() => doc),
        extensions: [
          history(),
          drawSelection(),
          highlightActiveLine(),
          highlightSelectionMatches(),
          EditorView.lineWrapping,
          markdown({ base: markdownLanguage }),
          syntaxHighlighting(highlight),
          theme,
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onchange(u.state.doc.toString());
          }),
        ],
      }),
    });
    return () => view?.destroy();
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
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
  }
</style>
