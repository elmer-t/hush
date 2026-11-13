import { describe, expect, it, vi } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
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
