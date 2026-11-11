import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { Decoration, type DecorationSet } from "@codemirror/view";
import MarkdownIt from "markdown-it";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";
import {
  MermaidWidget,
  liveMermaid,
  mermaidDecorations,
  mermaidField,
  mermaidFences,
  mermaidThemeChanged,
} from "$lib/liveMermaid";

/** A state for `doc` with the cursor at `^` (or a selection between two `^`), parsed in full. */
function stateOf(marked: string, extensions: unknown[] = []) {
  const at = [...marked.matchAll(/\^/g)].map((m, i) => m.index! - i);
  const doc = marked.replaceAll("^", "");
  const selection = at.length === 2 ? EditorSelection.single(at[0], at[1]) : EditorSelection.cursor(at[0] ?? 0);
  return parsed(EditorState.create({ doc, selection, extensions: [liveLanguage(), ...(extensions as [])] }));
}

/** The content markdown-it gives each mermaid fence: what the preview renders and caches. */
function previewSources(doc: string) {
  return new MarkdownIt()
    .parse(doc, {})
    .filter((t) => t.type === "fence" && t.info.trim().split(/\s+/)[0].toLowerCase() === "mermaid")
    .map((t) => t.content);
}

const FLOW = "graph TD\n  A --> B\n";

describe("mermaidFences", () => {
  it("finds mermaid fences and gives the same source as the preview", () => {
    const docs = [
      "# T\n\n```mermaid\ngraph TD\n  A --> B\n```\n\nafter",
      "```mermaid\nA\n```\n```Mermaid extra words\nB\n\n```\n",
      "~~~mermaid\nsequenceDiagram\n  a->>b: hi\n~~~",
      "  ```mermaid\n    graph LR\n  x --> y\n y\n  ```\n",
      "````mermaid\n```\ninner\n```\n````",
      "```mermaid\n```",
      "```mermaid\nunclosed\nto the end",
      "```mermaid\nunclosed\n",
      "text\n```mermaid\nA\n```   \nmore",
    ];
    for (const doc of docs) {
      const state = stateOf(doc);
      expect(
        mermaidFences(state).map((f) => f.source),
        doc,
      ).toEqual(previewSources(doc));
    }
  });

  it("covers whole lines, from the opening fence to the end of the closing one", () => {
    const doc = "a\n\n```mermaid\nA\n```\n\nb";
    const [fence] = mermaidFences(stateOf(doc));
    expect(doc.slice(fence.from, fence.to)).toBe("```mermaid\nA\n```");
    expect(fence.sourceFrom).toBe(doc.indexOf("A\n```"));
  });

  it("puts the cursor target on the opening line's end when the fence is empty", () => {
    const doc = "```mermaid\n```";
    const [fence] = mermaidFences(stateOf(doc));
    expect(fence.sourceFrom).toBe("```mermaid".length);
  });

  it("ignores other fences, indented code, and fences in lists, quotes or frontmatter", () => {
    expect(mermaidFences(stateOf("```js\nA\n```\n\n```\nB\n```\n\n    mermaid\n"))).toEqual([]);
    expect(mermaidFences(stateOf("- item\n\n  ```mermaid\n  A\n  ```\n"))).toEqual([]);
    expect(mermaidFences(stateOf("> ```mermaid\n> A\n> ```\n"))).toEqual([]);
    expect(mermaidFences(stateOf("---\nx: |\n  ```mermaid\n  A\n  ```\n---\n"))).toEqual([]);
    expect(mermaidFences(stateOf("```mermaidx\nA\n```"))).toEqual([]);
  });

  it("opens a fence while the selection touches any of its lines", () => {
    const open = (doc: string) => mermaidFences(stateOf(doc)).map((f) => f.open);
    expect(open("^a\n\n```mermaid\nA\n```\n\nb")).toEqual([false]);
    expect(open("a\n^\n```mermaid\nA\n```\n\nb")).toEqual([false]);
    expect(open("a\n\n^```mermaid\nA\n```\n\nb")).toEqual([true]);
    expect(open("a\n\n```merm^aid\nA\n```\n\nb")).toEqual([true]);
    expect(open("a\n\n```mermaid\nA^\n```\n\nb")).toEqual([true]);
    expect(open("a\n\n```mermaid\nA\n```^\n\nb")).toEqual([true]);
    expect(open("a\n\n```mermaid\nA\n```\n^\nb")).toEqual([false]);
    expect(open("^a\n\n```mermaid\nA\n```\n\nb^")).toEqual([true]);
    expect(open("```mermaid\nA\n```\n^\n```mermaid\nB\n```")).toEqual([false, false]);
    expect(open("```mermaid\nA\n```\n\n```mermaid\nB^\n```")).toEqual([false, true]);
  });
});

describe("MermaidWidget", () => {
  it("is equal when source, theme and placement match", () => {
    expect(new MermaidWidget(FLOW, "light", false).eq(new MermaidWidget(FLOW, "light", false))).toBe(true);
    expect(new MermaidWidget(FLOW, "light", true).eq(new MermaidWidget(FLOW, "light", true))).toBe(true);
    expect(new MermaidWidget(FLOW, "light", true).eq(new MermaidWidget(FLOW, "light", false))).toBe(false);
    expect(new MermaidWidget(FLOW, "light", false).eq(new MermaidWidget(FLOW + "x", "light", false))).toBe(false);
    expect(new MermaidWidget(FLOW, "light", false).eq(new MermaidWidget(FLOW, "dark", false))).toBe(false);
  });
});

type Seen = { from: number; to: number; widget?: MermaidWidget; block: boolean; cls?: string };

function list(set: DecorationSet): Seen[] {
  const out: Seen[] = [];
  set.between(0, 1e9, (from, to, d: Decoration) => {
    const spec = d.spec as { widget?: MermaidWidget; block?: boolean; class?: string };
    out.push({ from, to, widget: spec.widget, block: !!spec.block, cls: spec.class });
  });
  return out;
}

describe("mermaidDecorations", () => {
  const doc = "a\n\n```mermaid\nA\n```\n\nb";
  const from = doc.indexOf("```mermaid");
  const to = doc.indexOf("```\n\nb") + 3;

  it("replaces a closed fence with one block widget", () => {
    const state = stateOf("^" + doc);
    const decos = list(mermaidDecorations(state, "t"));
    expect(decos).toHaveLength(1);
    expect(decos[0]).toMatchObject({ from, to, block: true });
    expect(decos[0].widget).toMatchObject({ source: "A\n", theme: "t", editing: false });
  });

  it("shows an open fence's source, styled, with the diagram below it", () => {
    const state = stateOf(doc.replace("A\n", "A^\n"));
    const decos = list(mermaidDecorations(state, "t"));
    const lines = decos.filter((d) => d.cls);
    expect(lines.map((d) => d.from)).toEqual([from, from + 11, from + 13]);
    const widget = decos.find((d) => d.widget)!;
    expect(widget).toMatchObject({ from: to, to, block: true });
    expect(widget.widget).toMatchObject({ source: "A\n", editing: true });
  });

  it("keeps its widgets when typing elsewhere, and rebuilds them on a theme change", () => {
    let state = stateOf("^" + doc, [liveMermaid(() => "light")]);
    const field = mermaidField;
    const before = list(state.field(field).decorations)[0].widget!;
    state = state.update({ changes: { from: 0, insert: "typing " } }).state;
    const after = list(state.field(field).decorations)[0].widget!;
    expect(after.eq(before)).toBe(true);
    state = state.update({ effects: mermaidThemeChanged.of("dark") }).state;
    const dark = list(state.field(field).decorations)[0].widget!;
    expect(dark.theme).toBe("dark");
    expect(dark.eq(before)).toBe(false);
  });
});
