import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState, Text } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { buildLive, frontmatterRange, frontmatterView, type LiveSpec } from "$lib/liveDecorations";
import { liveLanguage } from "$lib/livePreview";
import { FRONTMATTER } from "$lib/markdown";

/** A state for `doc` with the cursor at `^` (or a selection between two `^`), parsed in full. */
function stateOf(marked: string) {
  const at = [...marked.matchAll(/\^/g)].map((m, i) => m.index! - i);
  const doc = marked.replaceAll("^", "");
  const selection = at.length === 2 ? EditorSelection.single(at[0], at[1]) : EditorSelection.cursor(at[0] ?? 0);
  const state = EditorState.create({ doc, selection, extensions: [liveLanguage()] });
  ensureSyntaxTree(state, doc.length, 5000);
  return state;
}

function specs(marked: string, ranges?: { from: number; to: number }[]) {
  const state = stateOf(marked);
  const all = buildLive(state, ranges ?? [{ from: 0, to: state.doc.length }]);
  return { state, specs: all.sort((a, b) => a.from - b.from) };
}

const slice = (state: EditorState, s: LiveSpec) => state.sliceDoc(s.from, s.kind === "line" ? s.from : s.to);

/** The source text hidden. */
function hidden(marked: string) {
  const { state, specs: all } = specs(marked);
  return all.filter((s) => s.kind === "hide").map((s) => slice(state, s));
}

/** The text styled with `cls`. */
function styled(marked: string, cls: string) {
  const { state, specs: all } = specs(marked);
  return all.filter((s) => s.kind === "mark" && s.class === cls).map((s) => slice(state, s));
}

describe("inline marks", () => {
  it("hides the marks of a span the cursor is not on, and styles it", () => {
    expect(hidden("^a **b** c")).toEqual(["**", "**"]);
    expect(styled("^a **b** c", "cm-live-strong")).toEqual(["**b**"]);
    expect(hidden("^a *b* _c_")).toEqual(["*", "*", "_", "_"]);
    expect(styled("^a *b* _c_", "cm-live-em")).toEqual(["*b*", "_c_"]);
    expect(hidden("^a ~~b~~")).toEqual(["~~", "~~"]);
    expect(styled("^a ~~b~~", "cm-live-strike")).toEqual(["~~b~~"]);
    expect(hidden("^a `b` c")).toEqual(["`", "`"]);
    expect(styled("^a `b` c", "cm-live-code")).toEqual(["`b`"]);
  });

  it("shows the marks, muted, while the cursor is on or next to the span", () => {
    for (const doc of ["a ^**b** c", "a **^b** c", "a **b^** c", "a **b**^ c"]) {
      expect(hidden(doc), doc).toEqual([]);
      expect(styled(doc, "cm-live-syntax"), doc).toEqual(["**", "**"]);
      expect(styled(doc, "cm-live-strong"), doc).toEqual(["**b**"]);
    }
    expect(hidden("a **b** ^c")).toEqual(["**", "**"]);
  });

  it("reveals each span the selection touches, and only those", () => {
    expect(hidden("^a^ **b** *c*")).toEqual(["**", "**", "*", "*"]);
    expect(hidden("^a **b**^ *c*")).toEqual(["*", "*"]);
    expect(hidden("^a **b** *c*^")).toEqual([]);
  });

  it("reveals nested spans together when the cursor is in the inner one", () => {
    expect(hidden("**a *b* c** ^")).toEqual(["**", "*", "*", "**"]);
    expect(hidden("**a *^b* c**")).toEqual([]);
    // In the outer span but away from the inner one.
    expect(hidden("**a^ *b* c**")).toEqual(["*", "*"]);
  });

  it("leaves code content alone", () => {
    expect(hidden("`**not bold** [[nor link]]` ^")).toEqual(["`", "`"]);
    expect(styled("`**x**` ^", "cm-live-strong")).toEqual([]);
  });

  it("leaves raw HTML and escaped marks as text", () => {
    expect(hidden("^<b>x</b> \\*y\\*")).toEqual([]);
    expect(specs("^<b>x</b>").specs).toEqual([]);
  });
});

describe("headings", () => {
  it("hides the # marker off the heading's line and styles the line", () => {
    const { state, specs: all } = specs("# Title\n\n^para");
    expect(all.filter((s) => s.kind === "hide").map((s) => slice(state, s))).toEqual(["# "]);
    expect(all.filter((s) => s.kind === "line")).toEqual([{ kind: "line", from: 0, class: "cm-live-h1" }]);
  });

  it("shows the marker while the cursor is anywhere on the line", () => {
    expect(hidden("# Ti^tle\n\npara")).toEqual([]);
    expect(hidden("#^ Title")).toEqual([]);
    expect(styled("# Ti^tle", "cm-live-syntax")).toEqual(["# "]);
  });

  it("styles each level and hides closing markers", () => {
    const { specs: all } = specs("## a\n### b ###\n###### c\n^");
    expect(all.filter((s) => s.kind === "line").map((s) => s.kind === "line" && s.class)).toEqual([
      "cm-live-h2",
      "cm-live-h3",
      "cm-live-h6",
    ]);
    expect(hidden("### b ###\n^")).toEqual(["### ", " ###"]);
  });

  it("formats inline marks inside headings", () => {
    expect(hidden("# a **b**\n^")).toEqual(["# ", "**", "**"]);
  });
});

describe("links", () => {
  it("shows the text of [text](url) as a link and hides the rest", () => {
    expect(hidden('|see [text](https://x.com "t") now')).toEqual(["[", '](https://x.com "t")']);
    const { specs: all } = specs("[text](https://x.com) ^");
    expect(all).toContainEqual({
      kind: "mark",
      from: 1,
      to: 5,
      class: "cm-live-link",
      attributes: { "data-href": "https://x.com" },
    });
  });

  it("shows the source of a link the cursor is on", () => {
    expect(hidden("[te^xt](u)")).toEqual([]);
    expect(styled("[te^xt](u)", "cm-live-link")).toEqual(["text"]);
  });

  it("styles bare URLs and autolinks", () => {
    expect(styled("^see https://x.com now", "cm-live-link")).toEqual(["https://x.com"]);
    expect(hidden("^see https://x.com now")).toEqual([]);
    expect(hidden("<https://x.com> ^")).toEqual(["<", ">"]);
    expect(styled("<https://x.com> ^", "cm-live-link")).toEqual(["https://x.com"]);
  });

  it("leaves images and links without a URL as text", () => {
    expect(hidden("^![alt *x*](a.png) [ref]")).toEqual([]);
    expect(styled("^![alt](a.png)", "cm-live-link")).toEqual([]);
  });

  it("hides per line, never a line break", () => {
    const { state, specs: all } = specs('|[a](u\n"title")');
    const hides = all.filter((s) => s.kind === "hide");
    expect(hides.map((s) => slice(state, s))).toEqual(["[", "](u", '"title")']);
  });
});

describe("wikilinks", () => {
  it("shows the alias, else the target part, and hides the rest", () => {
    expect(hidden("[[Note#H|Alias]] ^")).toEqual(["[[Note#H|", "]]"]);
    expect(styled("[[Note#H|Alias]] ^", "cm-live-wikilink")).toEqual(["Alias"]);
    expect(hidden("[[Note#H]] ^")).toEqual(["[[", "]]"]);
    expect(styled("[[ Note#H ]] ^", "cm-live-wikilink")).toEqual(["Note#H"]);
    expect(hidden("[[ Note#H ]] ^")).toEqual(["[[ ", " ]]"]);
  });

  it("carries the target for the link handling to come", () => {
    const { specs: all } = specs("[[Note|Alias]] ^");
    expect(all).toContainEqual({
      kind: "mark",
      from: 7,
      to: 12,
      class: "cm-live-wikilink",
      attributes: { "data-target": "Note" },
    });
  });

  it("shows the source while the cursor is on it", () => {
    expect(hidden("[[No^te]]")).toEqual([]);
    expect(hidden("[[Note]]^")).toEqual([]);
  });

  it("leaves embeds, empty targets and code as text", () => {
    expect(styled("^![[embed]] [[#h]] `[[c]]`", "cm-live-wikilink")).toEqual([]);
  });
});

describe("visible ranges", () => {
  it("builds only for the ranges given", () => {
    const doc = Array.from({ length: 200 }, (_, i) => `line ${i} **b**`).join("\n");
    const state = stateOf(doc);
    const line50 = state.doc.line(51);
    const all = buildLive(state, [{ from: line50.from, to: line50.to }]);
    expect(all.length).toBe(3);
    expect(all.every((s) => s.from >= line50.from && s.from <= line50.to)).toBe(true);
  });

  it("doesn't repeat a span that two ranges both reach", () => {
    const state = stateOf("# a **long** heading\n^");
    const all = buildLive(state, [
      { from: 0, to: 8 },
      { from: 10, to: state.doc.length },
    ]);
    expect(all.filter((s) => s.kind === "line")).toHaveLength(1);
    expect(all.filter((s) => s.kind === "hide")).toHaveLength(3);
  });
});

describe("frontmatter", () => {
  const SAMPLES = [
    "---\ntitle: a\n---\nbody",
    "---\ntitle: a\n...\nbody",
    "---\n---\nbody",
    "---\ntitle: a\n---",
    "---\na\n---x\n---\nb",
    "---\nno end",
    "--- \na\n---\n",
    "----\na\n---\n",
    "x\n---\na\n---\n",
    "",
    "---",
    "---\n",
  ];

  it("matches the same block as markdown.ts", () => {
    for (const src of SAMPLES) {
      const m = FRONTMATTER.exec(src);
      const range = frontmatterRange(Text.of(src.split("\n")));
      const expected = m ? m[0].replace(/\n$/, "").length : null;
      expect(range?.to ?? null, JSON.stringify(src)).toBe(expected);
    }
  });

  it("is collapsed while the cursor is outside it, with its keys", () => {
    const view = frontmatterView(stateOf("---\ntitle: a\ntags:\n  - x\n---\n^body"));
    expect(view).toEqual({ from: 0, to: 28, collapsed: true, keys: ["title", "tags"] });
    expect(frontmatterView(stateOf("---\nti^tle: a\n---\nbody"))?.collapsed).toBe(false);
    expect(frontmatterView(stateOf("^---\ntitle: a\n---\nbody"))?.collapsed).toBe(false);
    expect(frontmatterView(stateOf("---\ntitle: a\n---^\nbody"))?.collapsed).toBe(false);
    expect(frontmatterView(stateOf("^no frontmatter"))).toBeNull();
  });

  it("is never formatted as markdown", () => {
    const { state, specs: all } = specs("---\ntitle: **a** [[b]]\n# not a heading\n---\n# H\n^");
    expect(all.filter((s) => s.kind === "hide").map((s) => slice(state, s))).toEqual(["# "]);
    expect(all.filter((s) => s.kind === "line")).toHaveLength(1);
  });
});

describe("performance", () => {
  it("types into a 5,000-line note quickly", () => {
    const lines = Array.from({ length: 5000 }, (_, i) =>
      i % 10 === 0 ? `## Heading ${i}` : `Line ${i} with **bold**, *em*, \`code\`, [[Link ${i}|alias]] and [a](https://x.com/${i}).`,
    );
    let state = stateOf(lines.join("\n"));
    const visible = () => {
      const line = state.doc.lineAt(state.selection.main.head);
      const first = state.doc.line(Math.max(1, line.number - 30));
      const last = state.doc.line(Math.min(state.doc.lines, line.number + 30));
      return [{ from: first.from, to: last.to }];
    };
    state = state.update({ selection: { anchor: state.doc.line(2500).to } }).state;
    const times: number[] = [];
    for (let i = 0; i < 50; i++) {
      const start = performance.now();
      const at = state.selection.main.head;
      state = state.update({ changes: { from: at, insert: "x" }, selection: { anchor: at + 1 } }).state;
      ensureSyntaxTree(state, visible()[0].to, 1000);
      buildLive(state, visible());
      times.push(performance.now() - start);
    }
    times.sort((a, b) => a - b);
    const median = times[times.length >> 1];
    console.log(`5,000-line note: median ${median.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms per keystroke`);
    expect(median).toBeLessThan(16);
  });
});
