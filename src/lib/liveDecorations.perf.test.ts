// Frame-budget check, run with `npm run test:perf`, not `npm test`: a 16 ms budget
// measured on a busy or shared machine says little. See vitest.perf.config.js.
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { buildLive } from "$lib/liveDecorations";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";

describe("performance", () => {
  it("types into a 5,000-line note quickly", () => {
    const lines = Array.from({ length: 5000 }, (_, i) =>
      i % 10 === 0 ? `## Heading ${i}` : `Line ${i} with **bold**, *em*, \`code\`, [[Link ${i}|alias]] and [a](https://x.com/${i}).`,
    );
    let state = parsed(EditorState.create({ doc: lines.join("\n"), extensions: [liveLanguage()] }));
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
      state = parsed(state, visible()[0].to, 1000);
      buildLive(state, visible());
      times.push(performance.now() - start);
    }
    times.sort((a, b) => a - b);
    const median = times[times.length >> 1];
    console.log(`5,000-line note: median ${median.toFixed(2)} ms, max ${times[times.length - 1].toFixed(2)} ms per keystroke`);
    expect(median).toBeLessThan(16);
  });
});
