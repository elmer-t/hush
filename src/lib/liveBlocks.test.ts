import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState, type SelectionRange, type Transaction } from "@codemirror/state";
import { history, undo } from "@codemirror/commands";
import { deleteMarkupBackward, insertNewlineContinueMarkup } from "@codemirror/lang-markdown";
import { buildLive, inlineHandlers, type LiveSpec } from "$lib/liveDecorations";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";
import { blockHandlers, cursorInBlock, findBlocks, shownBlocks, tableCellPos, toggleTask } from "$lib/liveBlocks";

/** A state for `doc` with the cursor at `^` (or a selection between two `^`), parsed in full. */
function stateOf(marked: string, extra = false) {
  const at = [...marked.matchAll(/\^/g)].map((m, i) => m.index! - i);
  const doc = marked.replaceAll("^", "");
  const selection = at.length === 2 ? EditorSelection.single(at[0], at[1]) : EditorSelection.cursor(at[0] ?? 0);
  return parsed(EditorState.create({ doc, selection, extensions: extra ? [liveLanguage(), history()] : [liveLanguage()] }));
}

const handlers = { ...inlineHandlers, ...blockHandlers };

function specs(marked: string) {
  const state = stateOf(marked);
  const all = buildLive(state, [{ from: 0, to: state.doc.length }], handlers);
  return { state, specs: all.sort((a, b) => a.from - b.from) };
}

const text = (state: EditorState, s: LiveSpec) => state.sliceDoc(s.from, s.kind === "line" ? s.from : s.to);
const hidden = (marked: string) => {
  const { state, specs: all } = specs(marked);
  return all.filter((s) => s.kind === "hide").map((s) => text(state, s));
};
const widgets = (marked: string) => {
  const { state, specs: all } = specs(marked);
  return all.flatMap((s) => (s.kind === "widget" ? [{ text: text(state, s), ...s.widget }] : []));
};
/** The line classes, by line number. */
const lineClasses = (marked: string) => {
  const { state, specs: all } = specs(marked);
  const out: Record<number, string[]> = {};
  for (const s of all) if (s.kind === "line") (out[state.doc.lineAt(s.from).number] ??= []).push(s.class);
  return out;
};

describe("cursor inside a block", () => {
  const doc = "para\n\n| a |\n|---|\n| 1 |\n\nafter";
  const state = (sel: EditorSelection | SelectionRange) =>
    EditorState.create({ doc, selection: sel, extensions: EditorState.allowMultipleSelections.of(true) });
  const from = doc.indexOf("| a |");
  const to = doc.indexOf("| 1 |") + 5;

  it("is inside when a cursor is on any of the block's lines, ends included", () => {
    for (const at of [from, from + 2, doc.indexOf("|---|"), to - 1, to]) {
      expect(cursorInBlock(state(EditorSelection.cursor(at)), from, to), String(at)).toBe(true);
    }
  });

  it("is outside on the lines around it", () => {
    for (const at of [0, from - 1, to + 1, doc.length]) {
      expect(cursorInBlock(state(EditorSelection.cursor(at)), from, to), String(at)).toBe(false);
    }
  });

  it("counts a selection that crosses the block, and any of several ranges", () => {
    expect(cursorInBlock(state(EditorSelection.single(0, doc.length)), from, to)).toBe(true);
    expect(cursorInBlock(state(EditorSelection.single(0, from - 1)), from, to)).toBe(false);
    const multi = EditorSelection.create([EditorSelection.cursor(0), EditorSelection.cursor(to)]);
    expect(cursorInBlock(state(multi), from, to)).toBe(true);
  });

  it("measures from the start of the first line, so an indented block counts its indent", () => {
    const s = EditorState.create({ doc: "x\n    code", selection: EditorSelection.cursor(2) });
    expect(cursorInBlock(s, 6, 10)).toBe(true);
  });
});

describe("task checkbox", () => {
  /** Toggles the task whose marker is at `[` in `doc`, returning the new doc. */
  function toggle(doc: string, marker = doc.indexOf("[")) {
    const state = stateOf(doc, true);
    const spec = toggleTask(state, marker);
    expect(spec).not.toBeNull();
    const tr = state.update(spec!);
    return { state, tr, doc: tr.state.doc.toString() };
  }

  it("ticks and unticks, changing exactly one character", () => {
    expect(toggle("- [ ] buy milk").doc).toBe("- [x] buy milk");
    expect(toggle("- [x] buy milk").doc).toBe("- [ ] buy milk");
    expect(toggle("- [X] buy milk").doc).toBe("- [ ] buy milk");
    expect(toggle("1. [ ] step [ ] two").doc).toBe("1. [x] step [ ] two");
    const { tr } = toggle("text\n  - [ ] nested **b** [x]\n", 9);
    expect(tr.state.doc.toString()).toBe("text\n  - [x] nested **b** [x]\n");
    let changed = 0;
    tr.changes.iterChanges((fa, ta, fb, tb) => {
      changed++;
      expect([ta - fa, tb - fb]).toEqual([1, 1]);
    });
    expect(changed).toBe(1);
  });

  it("leaves the selection where it was", () => {
    const state = stateOf("- [ ] a\n\npara^", true);
    const tr = state.update(toggleTask(state, 2)!);
    expect(tr.state.selection.main.head).toBe(state.selection.main.head);
  });

  it("is one undoable step, even right after typing", () => {
    let state = stateOf("- [ ] a\n\npara^", true);
    state = state.update({ changes: { from: state.doc.length, insert: "x" }, userEvent: "input.type" }).state;
    state = state.update(toggleTask(state, 2)!).state;
    state = state.update(toggleTask(state, 2)!).state;
    state = state.update(toggleTask(state, 2)!).state;
    expect(state.doc.toString()).toBe("- [x] a\n\nparax");
    const run = (s: EditorState) => {
      let out = s;
      undo({ state: s, dispatch: (tr: Transaction) => (out = tr.state) });
      return out;
    };
    state = run(state);
    expect(state.doc.toString()).toBe("- [ ] a\n\nparax");
    state = run(state);
    expect(state.doc.toString()).toBe("- [x] a\n\nparax");
    state = run(state);
    expect(state.doc.toString()).toBe("- [ ] a\n\nparax");
    state = run(state);
    expect(state.doc.toString()).toBe("- [ ] a\n\npara");
  });

  it("refuses anything that is not a task marker", () => {
    for (const [doc, at] of [
      ["- [ ] a", 0],
      ["[ ] a", 0],
      ["- [y] a", 2],
      ["```\n- [ ] a\n```", 6],
    ] as const) {
      expect(toggleTask(stateOf(doc), at), doc).toBeNull();
    }
  });
});

describe("lists", () => {
  it("shows bullet markers as bullets off the cursor's line, and keeps numbers", () => {
    expect(widgets("- a\n* b\n+ c\n1. d\n^")).toEqual([
      { text: "-", type: "bullet" },
      { text: "*", type: "bullet" },
      { text: "+", type: "bullet" },
    ]);
    expect(widgets("- a\n- b^")).toEqual([{ text: "-", type: "bullet" }]);
    expect(widgets("^1. a\n2) b")).toEqual([]);
  });

  it("marks each line with its nesting depth", () => {
    expect(lineClasses("- a\n  - b\n    1. c\n- d\n^")).toEqual({
      1: ["cm-live-li", "cm-live-li-1"],
      2: ["cm-live-li", "cm-live-li-2"],
      3: ["cm-live-li", "cm-live-li-3"],
      4: ["cm-live-li", "cm-live-li-1"],
    });
  });

  it("shows tasks as checkboxes in place of the bullet and marker", () => {
    expect(widgets("- [ ] a\n- [x] b\n1. [X] c\n^")).toEqual([
      { text: "- [ ]", type: "task", checked: false, marker: 2 },
      { text: "- [x]", type: "task", checked: true, marker: 2 },
      { text: "[X]", type: "task", checked: true, marker: 0 },
    ]);
    expect(widgets("- [ ] a^")).toEqual([]);
  });

  it("continues a list on Enter and ends it on an empty item", () => {
    const enter = (marked: string) => {
      let state = stateOf(marked);
      insertNewlineContinueMarkup({ state, dispatch: (tr: Transaction) => (state = tr.state) });
      return state.doc.toString();
    };
    expect(enter("- a^")).toBe("- a\n- ");
    expect(enter("1. a^")).toBe("1. a\n2. ");
    expect(enter("- [x] a^")).toBe("- [x] a\n- [ ] ");
    expect(enter("- a\n- b\n- ^")).toBe("- a\n- b\n");
    // CodeMirror's one exception: an empty second item makes a two-item list loose first.
    expect(enter("- a\n- ^")).toBe("- a\n\n- ");
    expect(enter("- a\n\n- ^")).toBe("- a\n\n");
    let state = stateOf("- a\n- ^");
    deleteMarkupBackward({ state, dispatch: (tr: Transaction) => (state = tr.state) });
    // Backspace turns the empty marker into the item's indent, as CodeMirror does.
    expect(state.doc.toString()).toBe("- a\n  ");
  });
});

describe("blockquotes", () => {
  it("hides the > markers off the cursor's line and marks the quote lines", () => {
    expect(hidden("> a\n> b\n\n^x")).toEqual(["> ", "> "]);
    expect(hidden("> a^\n> b")).toEqual(["> "]);
    expect(lineClasses("> a\nlazy\n\n^x")).toEqual({ 1: ["cm-live-quote"], 2: ["cm-live-quote"] });
  });

  it("hides nested markers and marks the nested lines", () => {
    expect(hidden("> a\n> > b\n\n^x")).toEqual(["> ", "> ", "> "]);
    expect(lineClasses("> a\n> > b\n\n^x")[2]).toEqual(["cm-live-quote", "cm-live-quote-nested"]);
  });
});

describe("code blocks", () => {
  it("hides the fences while the cursor is outside, and marks every line", () => {
    expect(hidden("```js\nlet a = 1;\n```\n\n^x")).toEqual(["```js", "```"]);
    expect(lineClasses("```js\nlet a = 1;\n```\n\n^x")).toEqual({
      1: ["cm-live-codeblock", "cm-live-codeblock-first", "cm-live-fence"],
      2: ["cm-live-codeblock"],
      3: ["cm-live-codeblock", "cm-live-codeblock-last", "cm-live-fence"],
    });
  });

  it("shows the fences, muted, while the cursor is anywhere in the block", () => {
    for (const doc of ["^```js\na\n```", "```js\na^\n```", "```js\na\n```^"]) {
      expect(hidden(doc), doc).toEqual([]);
      expect(lineClasses(doc)[1], doc).toEqual(["cm-live-codeblock", "cm-live-codeblock-first"]);
    }
  });

  it("keeps an unclosed fence that is being typed raw", () => {
    expect(hidden("```\nabc^")).toEqual([]);
    expect(hidden("x^\n\n```\nabc")).toEqual(["```"]);
  });

  it("never formats markdown in code", () => {
    expect(hidden("```\n**a** > b\n- c\n```\n^")).toEqual(["```", "```"]);
    expect(widgets("```\n- c\n---\n```\n^")).toEqual([]);
  });

  it("styles indented code", () => {
    expect(lineClasses("para\n\n    code\n    more\n\n^x")).toEqual({
      3: ["cm-live-codeblock", "cm-live-codeblock-first"],
      4: ["cm-live-codeblock", "cm-live-codeblock-last"],
    });
  });

  it("leaves top-level mermaid fences to the diagram widget (#136)", () => {
    expect(hidden("```mermaid\ngraph TD\n```\n\n^x")).toEqual([]);
    expect(lineClasses("``` Mermaid\ngraph TD\n```\n\n^x")).toEqual({});
  });
});

describe("horizontal rules", () => {
  it("shows a rule widget off the cursor's line", () => {
    expect(widgets("a\n\n---\n\n***\n\n___\n^")).toEqual([
      { text: "---", type: "rule" },
      { text: "***", type: "rule" },
      { text: "___", type: "rule" },
    ]);
    expect(widgets("---^")).toEqual([]);
  });

  it("is not confused with frontmatter delimiters or setext headings", () => {
    expect(widgets("---\ntitle: a\n---\n\nbody\n^")).toEqual([]);
    expect(widgets("Title\n---\n^")).toEqual([]);
  });
});

describe("tables and images", () => {
  const blocks = (marked: string) => {
    const state = stateOf(marked);
    return shownBlocks(state, findBlocks(state)).map((b) => ({ ...b, text: state.sliceDoc(b.from, b.to) }));
  };

  it("renders a table off the cursor, as whole lines", () => {
    const doc = "intro\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n^x";
    expect(blocks(doc)).toEqual([
      { kind: "table", block: true, from: 7, to: 36, text: "| a | b |\n|---|---|\n| 1 | 2 |" },
    ]);
    expect(blocks("| a |\n|---|\n| 1^ |")).toEqual([]);
  });

  it("leaves tables inside lists or quotes, and in the frontmatter, as text", () => {
    expect(blocks("> | a |\n> |---|\n\n^x")).toEqual([]);
    expect(blocks("- | a |\n  |---|\n\n^x")).toEqual([]);
    expect(blocks("---\n| a |\n|---|\n---\n^x")).toEqual([]);
  });

  it("renders an image alone on its line as a block, and inline images inline", () => {
    expect(blocks('![alt](a.png "t")\n\ntext ![b](b.png) more\n^')).toEqual([
      { kind: "image", block: true, from: 0, to: 17, text: '![alt](a.png "t")' },
      { kind: "image", block: false, from: 24, to: 35, text: "![b](b.png)" },
    ]);
    expect(blocks("- ![a](a.png)\n^").map((b) => b.block)).toEqual([false]);
  });

  it("shows an image's source while the cursor touches it", () => {
    expect(blocks("![a](a.png)^")).toEqual([]);
    expect(blocks("x ![a](a.png) ^y")).toHaveLength(1);
    expect(blocks("x ![a](a.png)^ y")).toEqual([]);
  });

  it("leaves reference images, embeds and images in code as text", () => {
    expect(blocks("![a][ref] ![[embed.png]] `![c](c.png)`\n\n    ![d](d.png)\n^")).toEqual([]);
  });

  it("finds a table's cell in the source", () => {
    const doc = "x\n\n| a | bb |\n|---|---|\n| 1 |  2 |\nc | d\n|e|\n\ny";
    const state = stateOf(doc);
    const table = doc.indexOf("| a");
    const at = (row: number, col: number) => {
      const pos = tableCellPos(state, table, row, col);
      return doc.slice(pos, pos + 2);
    };
    expect(at(0, 0)).toBe("a ");
    expect(at(0, 1)).toBe("bb");
    expect(at(1, 0)).toBe("1 ");
    expect(at(1, 1)).toBe("2 ");
    expect(at(2, 0)).toBe("c ");
    expect(at(2, 1)).toBe("d\n");
    expect(at(3, 0)).toBe("e|");
    // A cell markdown-it adds for a short row: the end of that row.
    expect(tableCellPos(state, table, 3, 1)).toBe(doc.indexOf("|e|") + 3);
    // Past the last row: the end of the table.
    expect(tableCellPos(state, table, 9, 0)).toBe(doc.indexOf("|e|") + 3);
  });
});
