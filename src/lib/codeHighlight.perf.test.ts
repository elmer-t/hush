// Frame-budget checks, run with `npm run test:perf`, not `npm test`: a 16 ms budget
// measured on a busy or shared machine says little. See vitest.perf.config.js.
import { describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
import { classHighlighter, highlightTree } from "@lezer/highlight";
import { loadGrammar } from "$lib/codeHighlight";
import { buildLive, inlineHandlers } from "$lib/liveDecorations";
import { blockHandlers, findBlocks, shownBlocks } from "$lib/liveBlocks";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";

vi.mock("dompurify", () => ({ default: { sanitize: (html: string) => html } }));
const { renderMarkdown } = await import("$lib/markdown");

const LANGS = ["rust", "js", "python", "sql", "json", "bash"];
const CODE: Record<string, string[]> = {
  rust: ["fn main() {", "    let n = 1;", '    let s = "hi"; // note', "    println!(\"{}\", s);", "}"],
  js: ['const a = "x"; // note', "function f(x) {", "  return x + 1;", "}"],
  python: ["def f(x):", '    return "s"  # note', "", "print(f(1))"],
  sql: ["SELECT name, id", "FROM notes", "WHERE id = 1 -- note", "ORDER BY name;"],
  json: ["{", '  "a": [1, true, null],', '  "b": "s"', "}"],
  bash: ['if [ -f "$1" ]; then', '  echo "yes" # note', "fi", "ls -la"],
};

/** A note of about `lines` lines: a paragraph, then a code block, in each language in turn (`plain`: without the language). */
function note(lines: number, plain = false): string {
  const out: string[] = [];
  for (let i = 0; out.length < lines; i++) {
    const lang = LANGS[i % LANGS.length];
    // Each block differs (its first `1` is the block's number), so none comes from the preview's cache.
    out.push(`Paragraph ${i} with **bold** text.`, "", "```" + (plain ? "" : lang), ...CODE[lang].join("\n").replace("1", String(i)).split("\n"), "```", "");
  }
  return out.join("\n");
}

const median = (times: number[]) => times.sort((a, b) => a - b)[times.length >> 1];

describe("code highlighting performance (#180)", () => {
  it("types into a 5,000-line note of highlighted code blocks in Live quickly", async () => {
    for (const lang of LANGS) await loadGrammar(lang);
    const handlers = { ...inlineHandlers, ...blockHandlers };
    let state = parsed(EditorState.create({ doc: note(5000), extensions: [liveLanguage()] }));
    // Into a code block in the middle of the note.
    const at0 = state.doc.line(state.doc.lineAt(state.doc.toString().indexOf("```js", state.doc.length >> 1)).number + 1).to;
    state = state.update({ selection: { anchor: at0 } }).state;
    const visible = () => {
      const line = state.doc.lineAt(state.selection.main.head);
      return { from: state.doc.line(line.number - 30).from, to: state.doc.line(line.number + 30).to };
    };
    let tokens = 0;
    const times: number[] = [];
    for (let i = 0; i < 50; i++) {
      const start = performance.now();
      const at = state.selection.main.head;
      state = parsed(state.update({ changes: { from: at, insert: "x" }, selection: { anchor: at + 1 } }).state);
      const range = visible();
      buildLive(state, [range], handlers);
      shownBlocks(state, findBlocks(state));
      // What the editor's highlighter does for the visible lines.
      highlightTree(syntaxTree(state), classHighlighter, () => tokens++, range.from, range.to);
      times.push(performance.now() - start);
    }
    expect(tokens).toBeGreaterThan(50 * 20);
    const m = median(times);
    console.log(`5,000-line code note, Live: median ${m.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms per keystroke`);
    expect(m).toBeLessThan(16);
  });

  it("renders the preview of a note with 600 code blocks quickly while typing", async () => {
    for (const lang of LANGS) await loadGrammar(lang);
    let body = note(5000);
    const blocks = body.split("```").length >> 1;
    const time = (src: string) => {
      const t = performance.now();
      const html = renderMarkdown(src);
      return { ms: performance.now() - t, html };
    };
    const plain = median([1, 2, 3, 4, 5].map(() => time(note(5000, true)).ms));
    const first = time(body);
    expect(first.html).toContain('<span class="tok-keyword">');
    // Typing in one code block: that block is parsed again, the others come from the cache.
    const at = body.indexOf("function f(x)", body.length >> 1);
    const times: number[] = [];
    for (let i = 0; i < 50; i++) {
      body = body.slice(0, at) + "x" + body.slice(at);
      const t = performance.now();
      renderMarkdown(body);
      times.push(performance.now() - t);
    }
    const m = median(times);
    console.log(`preview, ${blocks} code blocks: ${plain.toFixed(1)} ms without highlighting, first render ${first.ms.toFixed(1)} ms, then median ${m.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms per keystroke`);
    expect(m).toBeLessThan(50);
  });
});
