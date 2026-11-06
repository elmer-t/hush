import { describe, expect, it } from "vitest";
import { describeHref, describeWikilink, truncateMiddle, type LinkTarget } from "./linkTarget";

const text = (t: LinkTarget) => t.parts.map((p) => p.text).join("");

describe("truncateMiddle", () => {
  it("leaves short strings alone", () => expect(truncateMiddle("abc", 5)).toBe("abc"));
  it("cuts the middle to the limit", () => {
    const s = truncateMiddle("abcdefghij", 7);
    expect(s).toBe("abc…hij");
    expect(s.length).toBe(7);
  });
});

describe("describeHref", () => {
  it("shows an external URL in full", () => {
    expect(text(describeHref("https://example.com/a/b?x=1", "here", 80))).toBe("https://example.com/a/b?x=1");
  });

  it("shows the href, not the link text, and warns when the text names another site", () => {
    const t = describeHref("https://evil.example/login", "https://mybank.com", 80);
    expect(text(t)).toBe("https://evil.example/login");
    expect(t.warning).toBe("Link text says mybank.com");
  });

  it("warns for a bare domain as link text", () => {
    expect(describeHref("https://evil.example/", "mybank.com/login", 80).warning).toBe("Link text says mybank.com");
  });

  it("doesn't warn when the text names the same site, with or without www", () => {
    expect(describeHref("https://www.example.com/x", "example.com", 80).warning).toBeUndefined();
    expect(describeHref("https://example.com/x", "https://www.Example.com/x", 80).warning).toBeUndefined();
  });

  it("doesn't warn for ordinary link text", () => {
    expect(describeHref("https://example.com", "the docs", 80).warning).toBeUndefined();
    expect(describeHref("https://example.com", "version 1.2", 80).warning).toBeUndefined();
    expect(describeHref("https://example.com", "v1.20", 80).warning).toBeUndefined();
  });

  it("cuts a long URL in the middle and keeps the host whole", () => {
    const long = "https://docs.example.com/" + "segment/".repeat(30) + "page?id=42";
    const t = describeHref(long, long, 60);
    const s = text(t);
    expect(s.length).toBeLessThanOrEqual(60);
    expect(s.startsWith("https://docs.example.com/")).toBe(true);
    expect(s.endsWith("page?id=42")).toBe(true);
    expect(s).toContain("…");
  });

  it("never cuts a long host", () => {
    const host = "a".repeat(50) + ".example.com";
    const s = text(describeHref(`https://${host}/some/long/path/here`, "", 40));
    expect(s).toContain(host);
  });

  it("shows a look-alike host in punycode and keeps the path readable", () => {
    // Cyrillic "а" in place of the Latin "a".
    const s = text(describeHref("https://аpple.com/caf%C3%A9", "apple.com", 80));
    expect(s).toBe("https://xn--pple-43d.com/café");
  });

  it("warns for a look-alike host behind a real-looking text", () => {
    expect(describeHref("https://аpple.com/", "apple.com", 80).warning).toBe("Link text says apple.com");
  });

  it("mutes userinfo so the real host stands out", () => {
    const t = describeHref("https://mybank.com@evil.example/", "", 80);
    expect(text(t)).toBe("https://mybank.com@evil.example");
    expect(t.parts.find((p) => p.muted)?.text).toBe("mybank.com@");
    expect(t.parts.find((p) => p.text === "evil.example")?.muted).toBeFalsy();
  });

  it("shows a mailto address", () => {
    expect(text(describeHref("mailto:someone@example.com", "mail", 80))).toBe("mailto:someone@example.com");
  });

  it("marks links that clicking ignores", () => {
    expect(text(describeHref("#top", "top", 80))).toBe("#top (won't open)");
    expect(text(describeHref("javascript:alert(1)", "x", 80))).toBe("javascript:alert(1) (won't open)");
    expect(text(describeHref("file:///etc/passwd", "x", 80))).toBe("file:///etc/passwd (won't open)");
  });
});

describe("describeWikilink", () => {
  it("names the note a unique link opens", () => {
    expect(text(describeWikilink("Ideas", { kind: "unique", folder: "Projects/Hush", title: "Ideas" }, 80))).toBe(
      "Note: Projects/Hush/Ideas",
    );
    expect(text(describeWikilink("Ideas", { kind: "unique", folder: "", title: "Ideas" }, 80))).toBe("Note: Ideas");
  });

  it("says how many notes an ambiguous link matches", () => {
    expect(text(describeWikilink("Ideas", { kind: "ambiguous", count: 3 }, 80))).toBe("3 notes match Ideas");
  });

  it("says when no note exists", () => {
    expect(text(describeWikilink("Foo", { kind: "missing" }, 80))).toBe("No note Foo (click to create)");
  });
});
