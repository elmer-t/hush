import { describe, expect, it } from "vitest";
// @ts-expect-error type error without @types/node package
import { readdirSync, readFileSync } from "node:fs";

// Selected text (#245): every text colour the editor can show reaches WCAG AA (4.5:1) on the
// selection background, in both themes. Computed from the tokens in app.css.

const css: string = readFileSync(new URL("../app.css", import.meta.url), "utf8");

/** The `#rrggbb` tokens of the rule that starts at `selector`. */
function tokens(selector: string): Map<string, string> {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`no rule ${selector} in app.css`);
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
  return new Map([...body.matchAll(/(--[a-z0-9-]+):\s*(#[0-9a-f]{6})\s*;/g)].map((m) => [m[1], m[2]]));
}

/** Relative luminance of `#rrggbb` (WCAG 2.1). */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// Body text, links, code and the callout titles (`--callout-color` is one of the last six).
const TEXT = ["--text", "--text-2", "--muted", "--accent", "--callout-blue", "--callout-purple", "--ok", "--warn", "--danger"];

describe("contrast", () => {
  it("computes the WCAG ratio", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
  });
});

// The dark tokens are in app.css twice; theme.test.ts keeps the copies equal, so one is read here.
describe.each([
  ["light", ":root {"],
  ["dark", ':root[data-theme="dark"]'],
])("selected text in the %s theme", (_, selector) => {
  const theme = tokens(selector);
  const background = theme.get("--text-selection");
  const colours = [...TEXT, ...[...theme.keys()].filter((k) => k.startsWith("--syn-"))];

  it("has a selection colour of its own, apart from selected rows", () => {
    expect(background).toBeDefined();
    expect(background).not.toBe(theme.get("--selection"));
    // The list above and the five code colours.
    expect(colours.length).toBe(TEXT.length + 5);
  });

  it.each(colours)("%s reaches 4.5:1 on the selection", (name) => {
    const colour = theme.get(name);
    expect(colour, name).toBeDefined();
    expect(contrast(colour!, background!)).toBeGreaterThanOrEqual(4.5);
  });
});

// Outside the editor the selection keeps each text's own colour. Text in an inverse colour
// (light on a filled element in the light theme, dark in the dark theme) has no contrast on
// the selection colour, so the rules that use one must keep their text from being selected.
describe("text in an inverse colour", () => {
  const INVERSE = /(?<![-\w])color:\s*var\(--(accent-text|danger-text|bg)\)/;
  const lib = new URL("./", import.meta.url);
  const sources: [string, string][] = [
    ["app.css", css],
    ...(readdirSync(lib) as string[])
      .filter((name) => name.endsWith(".svelte"))
      .map((name): [string, string] => [name, readFileSync(new URL(name, lib), "utf8")]),
  ];
  const rules = sources.flatMap(([file, text]) =>
    [...text.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter((m) => INVERSE.test(m[2]))
      .map((m) => ({ rule: `${file}: ${m[1].trim().split("\n").pop()!.trim()}`, body: m[2] })),
  );

  it("is found in the rules this test knows of", () => {
    expect(rules.length).toBeGreaterThanOrEqual(8);
  });

  it.each(rules)("$rule can't be selected", ({ body }) => {
    expect(body).toMatch(/user-select:\s*none/);
  });
});
