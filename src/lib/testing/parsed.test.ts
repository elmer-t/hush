import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import { liveLanguage } from "$lib/livePreview";
import { parsed } from "$lib/testing/parsed";

// Longer than the 3,000 characters `EditorState.create` parses up front, so the state's
// tree is partial however fast the machine is.
const doc = "# T\n\n" + "Some *text* with [[a link]].\n\n".repeat(400) + "```mermaid\ngraph TD\n```\n";

describe("parsed", () => {
  it("is needed: ensureSyntaxTree does not update the tree the state holds", () => {
    const state = EditorState.create({ doc, extensions: [liveLanguage()] });
    expect(ensureSyntaxTree(state, doc.length, 5000)?.length).toBe(doc.length);
    expect(syntaxTree(state).length).toBeLessThan(doc.length);
  });

  it("gives a state whose syntax tree covers the whole document", () => {
    const state = parsed(EditorState.create({ doc, selection: { anchor: 7 }, extensions: [liveLanguage()] }));
    expect(syntaxTree(state).length).toBe(doc.length);
    expect(state.doc.toString()).toBe(doc);
    expect(state.selection.main.head).toBe(7);
  });
});
