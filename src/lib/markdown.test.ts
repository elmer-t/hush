import { describe, expect, it, vi } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
import { buildLive, inlineHandlers } from "$lib/liveDecorations";
import { blockHandlers } from "$lib/liveBlocks";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";

// The markdown sanitizer needs a DOM, which the tests don't have; it isn't what's tested here.
vi.mock("dompurify", () => ({ default: { sanitize: (html: string) => html } }));
const { renderMarkdown } = await import("$lib/markdown");

/** The Live editor's specs for `src`, with the cursor on a line added after it. */
function liveSpecs(src: string) {
  const doc = `${src}\n\nend`;
  const state = parsed(EditorState.create({ doc, selection: EditorSelection.cursor(doc.length), extensions: [liveLanguage()] }));
  return { state, specs: buildLive(state, [{ from: 0, to: doc.length }], { ...inlineHandlers, ...blockHandlers }) };
}

describe("task lists (#179)", () => {
  /** Cases the preview and the Live editor must agree on: the source and its checkboxes, in order. */
  const CASES: [string, boolean[]][] = [
    ["- [ ] todo", [false]],
    ["- [x] done", [true]],
    ["- [X] done", [true]],
    ["* [ ] a\n+ [x] b", [false, true]],
    ["1. [ ] a\n2. [x] b", [false, true]],
    ["1) [x] a", [true]],
    ["- [ ] a\n  - [x] b\n    1. [ ] c", [false, true, false]],
    ["- plain\n  - [x] nested", [true]],
    ["- [x] a\n\n- [ ] loose", [true, false]],
    ["> - [x] quoted", [true]],
    ["- [x]\ttab", [true]],
    ["- [ ]  two spaces", [false]],
    ["- [x] **bold** and `code`", [true]],
    ["- [ ] a\n  [ ] continued", [false]],
    // Nothing after the marker: a task only with the space that the Lezer parser asks for.
    ["- [ ] ", [false]],
    ["- [x]", []],
    // `[x]` is the task marker even when a reference of that name exists.
    ["- [x] a\n\n[x]: https://example.com", [true]],
    // Not at the start of a list item's first line.
    ["-[ ] a", []],
    ["[ ] a", []],
    ["- a [ ] b", []],
    ["- [ ]a", []],
    ["- [  ] a", []],
    ["- [y] a", []],
    ["- \\[ ] a", []],
    ["- [ ]\n  next line", []],
    ["- para\n\n  [ ] second paragraph", []],
    ["- # [ ] heading", []],
    ["- > [ ] quote in item", []],
    // Code.
    ["```\n- [ ] a\n```", []],
    ["    - [ ] a", []],
    ["- `[ ] a`", []],
    ["- ```\n  [ ] a\n  ```", []],
  ];

  const previewBoxes = (src: string) =>
    [...renderMarkdown(src).matchAll(/<span class="task-box[^"]*" role="checkbox" aria-checked="(true|false)"/g)].map((m) => m[1] === "true");

  const liveBoxes = (src: string) =>
    liveSpecs(src)
      .specs.filter((s) => s.kind === "widget" && s.widget.type === "task")
      .sort((a, b) => a.from - b.from)
      .map((s) => s.kind === "widget" && s.widget.type === "task" && s.widget.checked);

  it("the preview shows a checkbox for each task", () => {
    for (const [src, boxes] of CASES) expect(previewBoxes(src), src).toEqual(boxes);
  });

  it("the Live editor shows the same checkboxes", () => {
    for (const [src, boxes] of CASES) expect(liveBoxes(src), src).toEqual(boxes);
  });

  it("drops the marker and the bullet, and keeps the text", () => {
    expect(renderMarkdown("- [x] **done** thing\n- plain")).toBe(
      '<ul>\n<li class="task-item"><span class="task-box task-done" role="checkbox" aria-checked="true" aria-disabled="true" aria-label="Done"></span><strong>done</strong> thing</li>\n<li>plain</li>\n</ul>\n',
    );
    expect(renderMarkdown("1. [ ] a")).toBe(
      '<ol>\n<li class="task-item"><span class="task-box" role="checkbox" aria-checked="false" aria-disabled="true" aria-label="Not done"></span>a</li>\n</ol>\n',
    );
  });

  it("puts the box inside the paragraph of a loose item", () => {
    expect(renderMarkdown("- [ ] a\n\n- b")).toContain('<li class="task-item">\n<p><span class="task-box"');
  });

  it("renders no input element, which the sanitizer forbids", () => {
    expect(renderMarkdown("- [ ] a\n- [x] b")).not.toContain("<input");
  });

  it("leaves text that is not a task as it was", () => {
    expect(renderMarkdown("- a [ ] b")).toBe("<ul>\n<li>a [ ] b</li>\n</ul>\n");
    expect(renderMarkdown("- [x]")).toBe("<ul>\n<li>[x]</li>\n</ul>\n");
    expect(renderMarkdown("- `[ ] a`")).toBe("<ul>\n<li><code>[ ] a</code></li>\n</ul>\n");
  });
});

/** The source text of every `name` node the Live editor's parser finds in `src`. */
function liveNodes(src: string, name: string): string[] {
  const { state } = liveSpecs(src);
  const out: string[] = [];
  syntaxTree(state).iterate({ enter: (n) => void (n.name === name && out.push(state.sliceDoc(n.from, n.to))) });
  return out;
}

/** `html` as the text it shows. */
const shown = (html: string) =>
  html.replace(/<[^>]*>/g, "").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&amp;/g, "&");

/** Markdown source as the text it shows, for the inline syntax the cases below use. */
const plain = (source: string) => source.replace(/\*\*|~~|==|[*`]|\[\[|\]\]/g, "");

describe("highlights (#181)", () => {
  /** Cases the preview and the Live editor must agree on: the source and the text it highlights. */
  const CASES: [string, string[]][] = [
    ["==important==", ["important"]],
    ["a ==b== c", ["b"]],
    ["==a== and ==b==", ["a", "b"]],
    ["==two words==", ["two words"]],
    ["a==b==c", ["b"]],
    ["(==a==)", ["a"]],
    ["==a==.", ["a"]],
    ["==a\nb==", ["a\nb"]],
    // Other inline formatting, inside and around.
    ["==**bold**==", ["bold"]],
    ["==a **b** c==", ["a b c"]],
    ["**==a==**", ["a"]],
    ["*==a==*", ["a"]],
    ["~~==a==~~", ["a"]],
    ["==~~a~~==", ["a"]],
    ["==a `code` b==", ["a code b"]],
    ["==[[Note]]==", ["Note"]],
    ["- ==item==", ["item"]],
    ["> ==quote==", ["quote"]],
    ["# ==heading==", ["heading"]],
    ["| ==a== |\n|---|\n| b |", ["a"]],
    // Longer runs: the last two `=` are the delimiter.
    ["===a===", ["a="]],
    ["====", []],
    // Unclosed, or not next to text.
    ["==open", []],
    ["open==", []],
    ["a == b", []],
    ["a == b == c", []],
    ["== a ==", []],
    ["==a ==", []],
    ["== a==", []],
    ["x = y", []],
    ["a=b=c", []],
    ["==", []],
    ["\\==a==", []],
    ["==a\n\nb==", []],
    // Code.
    ["`==a==`", []],
    ["``x ==a== y``", []],
    ["==a `b==` c", []],
    ["```\n==a==\n```", []],
    ["    ==a==", []],
    // A line of `=` under text is a heading, not a highlight.
    ["title\n==", []],
  ];

  const previewMarks = (src: string) => [...renderMarkdown(src).matchAll(/<mark>([\s\S]*?)<\/mark>/g)].map((m) => shown(m[1]));
  const liveMarks = (src: string) => liveNodes(src, "Highlight").map((text) => plain(text));

  it("the preview highlights the text between the markers", () => {
    for (const [src, marks] of CASES) expect(previewMarks(src), src).toEqual(marks);
  });

  it("the Live editor highlights the same text", () => {
    for (const [src, marks] of CASES) expect(liveMarks(src), src).toEqual(marks);
  });

  it("renders a mark element and keeps what is not a highlight as text", () => {
    expect(renderMarkdown("a ==b **c**== d")).toBe("<p>a <mark>b <strong>c</strong></mark> d</p>\n");
    expect(renderMarkdown("a == b")).toBe("<p>a == b</p>\n");
    expect(renderMarkdown("==open")).toBe("<p>==open</p>\n");
    expect(renderMarkdown("`==a==`")).toBe("<p><code>==a==</code></p>\n");
  });

  it("works inside a link's text", () => {
    expect(renderMarkdown("[==a==](https://example.com)")).toBe('<p><a href="https://example.com"><mark>a</mark></a></p>\n');
    expect(liveMarks("[==a==](https://example.com)")).toEqual(["a"]);
  });

  it("hides the markers in Live off the cursor and shows them on it", () => {
    const at = (cursor: number) => {
      const doc = "x ==mark== y";
      const state = parsed(EditorState.create({ doc, selection: EditorSelection.cursor(cursor), extensions: [liveLanguage()] }));
      return buildLive(state, [{ from: 0, to: doc.length }], inlineHandlers).map((s) => `${s.kind}:${s.from}-${s.kind === "line" ? s.from : s.to}`);
    };
    expect(at(0)).toEqual(["mark:2-10", "hide:2-4", "hide:8-10"]);
    // On the span, its ends included, the markers are shown as syntax.
    for (const cursor of [2, 5, 10]) expect(at(cursor), String(cursor)).toEqual(["mark:2-10", "mark:2-4", "mark:8-10"]);
    expect(at(11)).toEqual(["mark:2-10", "hide:2-4", "hide:8-10"]);
  });
});

describe("subscript and superscript (#182)", () => {
  /** Cases the preview and the Live editor must agree on: the source and its `sub` and `sup` texts, in order. */
  const CASES: [string, string[]][] = [
    ["H~2~O", ["sub:2"]],
    ["x^2^", ["sup:2"]],
    ["x^2^ + y~i~", ["sup:2", "sub:i"]],
    ["~a~", ["sub:a"]],
    ["^a^", ["sup:a"]],
    ["E = mc^2^.", ["sup:2"]],
    ["a^b^c^d^", ["sup:b", "sup:d"]],
    ["x~i~^2^", ["sub:i", "sup:2"]],
    ["**H~2~O**", ["sub:2"]],
    ["- CO~2~", ["sub:2"]],
    ["| x^2^ |\n|---|\n| b |", ["sup:2"]],
    ["[x^2^](https://example.com)", ["sup:2"]],
    // The text between the markers is taken as it is.
    ["~*a*~", ["sub:*a*"]],
    ["^[[x]]^", ["sup:[[x]]"]],
    ["~a^b~", ["sub:a^b"]],
    // An escaped space, marker or backslash is text between the markers.
    ["~a\\ b~", ["sub:a b"]],
    ["^a\\^b^", ["sup:a^b"]],
    ["~a\\\\~", ["sub:a\\"]],
    // Strikethrough keeps working, alone and next to subscript.
    ["~~gone~~", []],
    ["~~a~~ H~2~O", ["sub:2"]],
    ["~~H~2~O~~", ["sub:2"]],
    ["H~2~O ~~gone~~", ["sub:2"]],
    ["~~a~b~~", []],
    ["~~a~~~b~", []],
    ["a ~~b~~ c ~d~", ["sub:d"]],
    // Lone markers in prose.
    ["~5 minutes", []],
    ["2^10", []],
    ["~5 minutes and ~6", []],
    ["2^10 and 3^4", []],
    ["a ~ b ~ c", []],
    ["~a b~", []],
    ["^a\tb^", []],
    ["~a\nb~", []],
    ["~a\\\nb~", []],
    ["~~", []],
    ["^^", []],
    ["^^a^", []],
    ["~a~~", []],
    ["\\~a~", []],
    ["\\^a^", []],
    // Code.
    ["`H~2~O`", []],
    ["``x^2^``", []],
    ["```\nH~2~O\n```", []],
    ["    x^2^", []],
    ["~~~\nx^2^\n~~~", []],
    // A bare link keeps its `~` and `^`.
    ["https://example.com/~user/~files", []],
    ["see www.example.com/~user/~files here", []],
    ["<https://example.com/~a~b>", []],
    ["[~a~](https://example.com/~u~v)", ["sub:a"]],
  ];

  const previewScripts = (src: string) =>
    [...renderMarkdown(src).matchAll(/<(sub|sup)>([\s\S]*?)<\/\1>/g)].map((m) => `${m[1]}:${shown(m[2])}`);

  function liveScripts(src: string): string[] {
    const { state } = liveSpecs(src);
    const out: string[] = [];
    syntaxTree(state).iterate({
      enter(n) {
        if (n.name !== "Subscript" && n.name !== "Superscript") return;
        out.push(`${n.name === "Subscript" ? "sub" : "sup"}:${state.sliceDoc(n.from + 1, n.to - 1).replace(/\\(.)/g, "$1")}`);
      },
    });
    return out;
  }

  it("the preview shows the text between the markers lowered or raised", () => {
    for (const [src, scripts] of CASES) expect(previewScripts(src), src).toEqual(scripts);
  });

  it("the Live editor reads the same text", () => {
    for (const [src, scripts] of CASES) expect(liveScripts(src), src).toEqual(scripts);
  });

  it("renders sub and sup elements and keeps strikethrough", () => {
    expect(renderMarkdown("H~2~O and x^2^")).toBe("<p>H<sub>2</sub>O and x<sup>2</sup></p>\n");
    expect(renderMarkdown("~~H~2~O~~")).toBe("<p><s>H<sub>2</sub>O</s></p>\n");
    expect(renderMarkdown("~~a~b~~")).toBe("<p><s>a~b</s></p>\n");
    expect(renderMarkdown("~5 minutes, 2^10")).toBe("<p>~5 minutes, 2^10</p>\n");
    expect(renderMarkdown("`H~2~O`")).toBe("<p><code>H~2~O</code></p>\n");
  });

  it("keeps a bare link whole", () => {
    expect(renderMarkdown("https://example.com/~user/~files")).toBe(
      '<p><a href="https://example.com/~user/~files">https://example.com/~user/~files</a></p>\n',
    );
  });

  it("keeps strikethrough in Live next to subscript", () => {
    expect(liveNodes("~~H~2~O~~", "Strikethrough")).toEqual(["~~H~2~O~~"]);
    expect(liveNodes("~~a~b~~", "Strikethrough")).toEqual(["~~a~b~~"]);
    expect(liveNodes("~~gone~~ H~2~O", "Strikethrough")).toEqual(["~~gone~~"]);
  });

  it("hides the markers in Live off the cursor and shows them on it", () => {
    const at = (cursor: number) => {
      const doc = "H~2~O x^2^";
      const state = parsed(EditorState.create({ doc, selection: EditorSelection.cursor(cursor), extensions: [liveLanguage()] }));
      return buildLive(state, [{ from: 0, to: doc.length }], inlineHandlers).map((s) => `${s.kind}:${s.from}-${s.kind === "line" ? s.from : s.to}`);
    };
    expect(at(0)).toEqual(["mark:1-4", "hide:1-2", "hide:3-4", "mark:7-10", "hide:7-8", "hide:9-10"]);
    expect(at(2)).toEqual(["mark:1-4", "mark:1-2", "mark:3-4", "mark:7-10", "hide:7-8", "hide:9-10"]);
    expect(at(10)).toEqual(["mark:1-4", "hide:1-2", "hide:3-4", "mark:7-10", "mark:7-8", "mark:9-10"]);
  });
});
