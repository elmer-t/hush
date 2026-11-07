import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { liveLanguage } from "$lib/livePreview";
import { clickAction, linkRangeAt, linkRevealed } from "$lib/liveLinks";
import type { Link } from "$lib/linkOpen";

function stateOf(doc: string, cursor = 0) {
  const state = EditorState.create({ doc, selection: EditorSelection.cursor(cursor), extensions: [liveLanguage()] });
  ensureSyntaxTree(state, doc.length, 5000);
  return state;
}

/** The source of the link around the `^` in `marked` (the `^` is removed). */
function linkSource(marked: string) {
  const pos = marked.indexOf("^");
  const doc = marked.replace("^", "");
  const state = stateOf(doc);
  const r = linkRangeAt(state, pos);
  return r && doc.slice(r.from, r.to);
}

describe("linkRangeAt", () => {
  it("finds a [text](url) link from its text", () => expect(linkSource("a [te^xt](https://x.com) b")).toBe("[text](https://x.com)"));
  it("finds it from formatted text inside it", () =>
    expect(linkSource("a [**b^old** t](https://x.com) b")).toBe("[**bold** t](https://x.com)"));
  it("finds an autolink", () => expect(linkSource("a <https://x^.com> b")).toBe("<https://x.com>"));
  it("finds a bare URL", () => expect(linkSource("see https://x^.com now")).toBe("https://x.com"));
  it("finds a wikilink from its alias", () => expect(linkSource("a [[Note|Al^ias]] b")).toBe("[[Note|Alias]]"));
  it("finds nothing in plain text", () => expect(linkSource("plain te^xt")).toBeNull());
});

describe("linkRevealed", () => {
  const doc = "ab [t](https://x.com) cd";
  const range = { from: 3, to: 21 };
  it("is revealed while the cursor is in it or at its ends", () => {
    for (const at of [3, 5, 21]) expect(linkRevealed(stateOf(doc, at), range)).toBe(true);
  });
  it("isn't while the cursor is elsewhere", () => {
    for (const at of [0, 2, 22]) expect(linkRevealed(stateOf(doc, at), range)).toBe(false);
  });
});

describe("clickAction", () => {
  const wiki: Link = { kind: "wiki", target: "Note" };
  const web: Link = { kind: "href", href: "https://a.com", text: "a" };
  const mail: Link = { kind: "href", href: "mailto:a@b.c", text: "a" };
  const other: Link = { kind: "href", href: "file:///x", text: "x" };
  const click = (link: Link | null, mod: boolean, revealed: boolean, shift = false) =>
    clickAction(link, { mod, shift, revealed });

  it("Ctrl/Cmd+click opens a wikilink, a web link and a mail link, revealed or not", () => {
    for (const link of [wiki, web, mail]) for (const revealed of [false, true]) expect(click(link, true, revealed)).toBe("open");
  });
  it("Ctrl/Cmd+click on another scheme does nothing, and leaves the cursor", () => {
    expect(click(other, true, false)).toBe("ignore");
    expect(click(other, true, true)).toBe("ignore");
  });
  it("a plain click on a link that isn't being edited opens it", () => {
    for (const link of [wiki, web, mail]) expect(click(link, false, false)).toBe("open");
  });
  it("a plain click on a revealed link places the cursor, to edit it", () => {
    for (const link of [wiki, web, mail, other]) expect(click(link, false, true)).toBe("cursor");
  });
  it("a plain click on a link that can't open places the cursor", () => expect(click(other, false, false)).toBe("cursor"));
  it("Shift+click extends the selection, as in text", () => {
    expect(click(web, false, false, true)).toBe("cursor");
    expect(click(web, true, false, true)).toBe("cursor");
  });
  it("a click off a link places the cursor", () => {
    expect(click(null, false, false)).toBe("cursor");
    expect(click(null, true, false)).toBe("cursor");
  });
});
