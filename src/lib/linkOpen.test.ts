import { describe, expect, it, vi } from "vitest";
import { canOpen, linkOfElement, openLink, type Link } from "$lib/linkOpen";

/** Just enough of an element for `linkOfElement`. */
function el(classes: string[], attrs: Record<string, string>, text = "") {
  const dataset: Record<string, string> = {};
  for (const [k, v] of Object.entries(attrs)) if (k.startsWith("data-")) dataset[k.slice(5)] = v;
  return {
    classList: { contains: (c: string) => classes.includes(c) },
    dataset,
    getAttribute: (name: string) => attrs[name] ?? null,
    textContent: text,
  } as unknown as Element;
}

describe("linkOfElement", () => {
  it("reads a preview wikilink", () => {
    expect(linkOfElement(el(["wikilink"], { href: "#", "data-target": "Note" }, "Alias"))).toEqual({
      kind: "wiki",
      target: "Note",
    });
  });
  it("reads a Live wikilink mark", () => {
    expect(linkOfElement(el(["cm-live-wikilink"], { "data-target": "Note" }, "Alias"))).toEqual({
      kind: "wiki",
      target: "Note",
    });
  });
  it("reads a preview link, with its text", () => {
    expect(linkOfElement(el([], { href: "https://a.com" }, "here"))).toEqual({
      kind: "href",
      href: "https://a.com",
      text: "here",
    });
  });
  it("reads a diagram link's xlink:href", () => {
    expect(linkOfElement(el([], { "xlink:href": "https://a.com" }, "x"))).toMatchObject({ href: "https://a.com" });
  });
  it("reads a Live link mark", () => {
    expect(linkOfElement(el(["cm-live-link"], { "data-href": "mailto:a@b.c" }, "mail"))).toEqual({
      kind: "href",
      href: "mailto:a@b.c",
      text: "mail",
    });
  });
});

describe("openLink", () => {
  const run = (link: Link) => {
    const onwikilink = vi.fn();
    const open = vi.fn();
    const opened = openLink(link, 10, 20, onwikilink, open);
    return { opened, onwikilink, open };
  };

  it("opens a wikilink's note at the pointer", () => {
    const r = run({ kind: "wiki", target: "Note" });
    expect(r.opened).toBe(true);
    expect(r.onwikilink).toHaveBeenCalledWith("Note", 10, 20);
    expect(r.open).not.toHaveBeenCalled();
  });

  for (const href of ["https://a.com/x", "http://a.com", "HTTPS://A.COM", "mailto:a@b.c"]) {
    it(`opens ${href} in the system browser`, () => {
      const r = run({ kind: "href", href, text: "" });
      expect(r.opened).toBe(true);
      expect(r.open).toHaveBeenCalledWith(href);
    });
  }

  for (const href of ["file:///etc/passwd", "javascript:alert(1)", "note.md", "#top", "ftp://a.com", ""]) {
    it(`does nothing for ${JSON.stringify(href)}`, () => {
      const r = run({ kind: "href", href, text: "" });
      expect(r.opened).toBe(false);
      expect(r.open).not.toHaveBeenCalled();
      expect(r.onwikilink).not.toHaveBeenCalled();
    });
  }

  it("says which links can open", () => {
    expect(canOpen({ kind: "wiki", target: "x" })).toBe(true);
    expect(canOpen({ kind: "href", href: "https://a", text: "" })).toBe(true);
    expect(canOpen({ kind: "href", href: "note.md", text: "" })).toBe(false);
  });
});
