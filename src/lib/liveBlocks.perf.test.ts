// Frame-budget check, run with `npm run test:perf`, not `npm test`: a 16 ms budget
// measured on a busy or shared machine says little. See vitest.perf.config.js.
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { buildLive, inlineHandlers } from "$lib/liveDecorations";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";
import { blockHandlers, findBlocks, shownBlocks } from "$lib/liveBlocks";

const handlers = { ...inlineHandlers, ...blockHandlers };

describe("performance", () => {
  it("types into a 5,000-line note with lists, quotes, code, tables and images quickly", () => {
    const chunk = [
      "## Heading",
      "- item **b**",
      "  - [ ] task",
      "> quote [[link]]",
      "```js",
      "code()",
      "```",
      "| a | b |",
      "|---|---|",
      "| 1 | 2 |",
      "",
      "![img](i.png)",
      "Text with ![inline](x.png) and *em*.",
      "",
      "---",
      "",
    ];
    const lines: string[] = [];
    while (lines.length < 5000) lines.push(...chunk);
    let state = parsed(EditorState.create({ doc: lines.join("\n"), extensions: [liveLanguage()] }));
    state = state.update({ selection: { anchor: state.doc.line(2501).to } }).state;
    const visible = () => {
      const line = state.doc.lineAt(state.selection.main.head);
      return [{ from: state.doc.line(line.number - 30).from, to: state.doc.line(line.number + 30).to }];
    };
    const times: number[] = [];
    for (let i = 0; i < 50; i++) {
      const start = performance.now();
      const at = state.selection.main.head;
      state = parsed(state.update({ changes: { from: at, insert: "x" }, selection: { anchor: at + 1 } }).state);
      buildLive(state, visible(), handlers);
      shownBlocks(state, findBlocks(state));
      times.push(performance.now() - start);
    }
    times.sort((a, b) => a - b);
    const median = times[times.length >> 1];
    console.log(`5,000-line block note: median ${median.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms per keystroke`);
    expect(median).toBeLessThan(16);
  });
});
