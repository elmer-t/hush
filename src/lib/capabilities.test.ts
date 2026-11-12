import { describe, expect, it } from "vitest";
import capability from "../../src-tauri/capabilities/default.json";

/**
 * The main window's Tauri permissions (#154). The opener plugin may only open the URLs
 * Hush's links open (`http(s):`, `mailto:`; see `canOpen` in linkOpen.ts), nothing else
 * of the plugin is granted.
 */

type Permission = string | { identifier: string; allow?: { url?: string }[] };

const permissions = capability.permissions as Permission[];
const ids = permissions.map((p) => (typeof p === "string" ? p : p.identifier));

function openerUrlPatterns(): string[] {
  const entry = permissions.find((p) => typeof p !== "string" && p.identifier === "opener:allow-open-url");
  if (!entry || typeof entry === "string") return [];
  return (entry.allow ?? []).map((a) => a.url ?? "");
}

/**
 * The opener plugin matches with `glob::Pattern::matches` (tauri-plugin-opener 2.7.2,
 * src/scope.rs): `*` matches any run of characters, including `/`, and matching is
 * case-sensitive. Only `*` is used in our patterns, so this models it exactly.
 */
function globMatches(pattern: string, url: string): boolean {
  const re = new RegExp("^" + pattern.split("*").map((s) => s.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*") + "$");
  return re.test(url);
}

const allowed = (url: string) => openerUrlPatterns().some((p) => globMatches(p, url));

describe("default capability", () => {
  it("grants no opener permission set beyond opening URLs", () => {
    expect(ids).not.toContain("opener:default");
    expect(ids).not.toContain("opener:allow-default-urls");
    expect(ids.filter((id) => id.startsWith("opener:"))).toEqual(["opener:allow-open-url"]);
  });

  it("scopes open-url to http, https and mailto", () => {
    expect(openerUrlPatterns().sort()).toEqual(["http://*", "https://*", "mailto:*"]);
  });

  it("allows the links Hush opens", () => {
    expect(allowed("https://example.com/a/b?c=d#e")).toBe(true);
    expect(allowed("http://example.com")).toBe(true);
    expect(allowed("mailto:someone@example.com")).toBe(true);
    expect(allowed("https://huggingface.co/BAAI/bge-small-en-v1.5")).toBe(true);
  });

  it("refuses tel:, file: and other schemes", () => {
    expect(allowed("tel:+31201234567")).toBe(false);
    expect(allowed("file:///C:/Windows/System32/calc.exe")).toBe(false);
    expect(allowed("javascript:alert(1)")).toBe(false);
    expect(allowed("ms-settings:")).toBe(false);
  });

  it("lets the window follow the theme setting (#173)", () => {
    expect(ids).toContain("core:window:allow-set-theme");
  });
});
