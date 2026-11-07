import { describe, expect, it, vi } from "vitest";
import { parser } from "@lezer/markdown";
import { GFM } from "@lezer/markdown";
import { parseWikilink, wikilinkMarkdown } from "$lib/wikilink";

// The markdown sanitizer needs a DOM, which the tests don't have; it isn't what's tested here.
vi.mock("dompurify", () => ({ default: { sanitize: (html: string) => html } }));
const { renderMarkdown } = await import("$lib/markdown");

type Link = { target: string; text: string };

/** Cases both parsers must agree on: the source and the wikilinks it holds. */
const CASES: [string, Link[]][] = [
  ["[[Note]]", [{ target: "Note", text: "Note" }]],
  ["[[Note|Alias]]", [{ target: "Note", text: "Alias" }]],
  ["[[Note#Heading]]", [{ target: "Note", text: "Note#Heading" }]],
  ["[[ Note # H |  Al  ]]", [{ target: "Note", text: "Al" }]],
  ["[[  spaced  ]]", [{ target: "spaced", text: "spaced" }]],
  ["[[Note|]]", [{ target: "Note", text: "Note" }]],
  ["[[a]] and [[b|c]]", [{ target: "a", text: "a" }, { target: "b", text: "c" }]],
  ["**[[Bold]]** text", [{ target: "Bold", text: "Bold" }]],
  ["[[a]]]", [{ target: "a", text: "a" }]],
  ["folder/[[x/y]]", [{ target: "x/y", text: "x/y" }]],
  // An empty target stays text.
  ["[[#Heading]]", []],
  ["[[|alias]]", []],
  ["[[]]", []],
  // Embeds and code are not links.
  ["![[embed]]", []],
  ["`[[code]]`", []],
  ["``a [[code]] b``", []],
  // Unclosed, nested brackets or a line break.
  ["[[open", []],
  ["[[a[b]]", []],
  ["[[a]b]]", []],
  ["x [[a\nb]]", []],
  ["\\[[a]]", []],
];

function markdownItLinks(src: string): Link[] {
  const html = renderMarkdown(src);
  return [...html.matchAll(/<a href="#" class="wikilink" data-target="([^"]*)">([^<]*)<\/a>/g)].map((m) => ({
    target: m[1],
    text: m[2],
  }));
}

function lezerLinks(src: string): Link[] {
  const tree = parser.configure([GFM, wikilinkMarkdown]).parse(src);
  const links: Link[] = [];
  tree.iterate({
    enter(node) {
      if (node.name !== "Wikilink") return;
      const link = parseWikilink(src.slice(node.from, node.to), 0);
      expect(link, src).not.toBeNull();
      links.push({ target: link!.target, text: src.slice(node.from + link!.textFrom, node.from + link!.textTo) });
    },
  });
  return links;
}

describe("parseWikilink", () => {
  it("reads the target, the shown text and where that text is", () => {
    const src = "see [[Note#H|Alias]] here";
    const link = parseWikilink(src, 4);
    expect(link).toEqual({ from: 4, to: 20, target: "Note", text: "Alias", textFrom: 13, textTo: 18 });
    expect(src.slice(link!.textFrom, link!.textTo)).toBe("Alias");
  });

  it("finds nothing where there is no [[", () => {
    expect(parseWikilink("a [b]", 2)).toBeNull();
    expect(parseWikilink("[[a]]", 1)).toBeNull();
  });

  it("is not an embed", () => {
    expect(parseWikilink("![[a]]", 1)).toBeNull();
  });

  it("shows text that is a slice of the source, trimmed", () => {
    for (const src of ["[[ a ]]", "[[a | b ]]", "[[ a#h ]]"]) {
      const link = parseWikilink(src, 0)!;
      expect(src.slice(link.textFrom, link.textTo)).toBe(link.text);
      expect(link.text).toBe(link.text.trim());
    }
  });
});

describe("wikilinks in markdown-it and Lezer", () => {
  it.each(CASES)("%j", (src, expected) => {
    expect(markdownItLinks(src)).toEqual(expected);
    expect(lezerLinks(src)).toEqual(expected);
  });
});
