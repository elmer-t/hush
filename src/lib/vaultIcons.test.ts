import { describe, expect, it } from "vitest";
// @ts-expect-error type error without @types/node package
import { readFileSync } from "node:fs";
import { gridMove, resolveVaultIcon, VAULT_COLORS, VAULT_ICONS } from "$lib/vaultIcons";

// The form `set_vault_icon` accepts (src-tauri/src/db/icon.rs).
const ID = /^[a-z0-9-]{1,32}$/;

describe("the vault icon set", () => {
  it("has 24 to 32 icons and 8 colours, each with an ID the vault accepts and a name", () => {
    expect(VAULT_ICONS.length).toBeGreaterThanOrEqual(24);
    expect(VAULT_ICONS.length).toBeLessThanOrEqual(32);
    expect(VAULT_COLORS.length).toBe(8);
    for (const { id, label } of [...VAULT_ICONS, ...VAULT_COLORS]) {
      expect(id).toMatch(ID);
      expect(label.trim()).not.toBe("");
    }
    expect(new Set(VAULT_ICONS.map((i) => i.id)).size).toBe(VAULT_ICONS.length);
    expect(new Set(VAULT_ICONS.map((i) => i.label)).size).toBe(VAULT_ICONS.length);
    expect(new Set(VAULT_COLORS.map((c) => c.id)).size).toBe(8);
    expect(new Set(VAULT_COLORS.map((c) => c.label)).size).toBe(8);
  });
});

describe("resolveVaultIcon", () => {
  it("gives the drawing and the colour token of a known icon", () => {
    expect(resolveVaultIcon({ icon: "anchor", color: "blue" })).toEqual({
      label: "Anchor",
      paths: VAULT_ICONS.find((i) => i.id === "anchor")!.paths,
      color: "var(--vault-blue)",
    });
  });

  it("is null without an icon, so the Hush mark is drawn", () => {
    expect(resolveVaultIcon(null)).toBeNull();
    expect(resolveVaultIcon(undefined)).toBeNull();
  });

  it("is null for an icon or a colour this version doesn't know", () => {
    expect(resolveVaultIcon({ icon: "submarine", color: "blue" })).toBeNull();
    expect(resolveVaultIcon({ icon: "anchor", color: "ultraviolet" })).toBeNull();
    // Names of object internals are not icons.
    expect(resolveVaultIcon({ icon: "constructor", color: "toString" })).toBeNull();
  });
});

describe("gridMove", () => {
  // 10 choices in rows of 4: 0-3, 4-7, 8-9.
  const move = (index: number, key: string) => gridMove(index, key, 10, 4);

  it("moves one choice left and right, stopping at the ends", () => {
    expect(move(4, "ArrowRight")).toBe(5);
    expect(move(4, "ArrowLeft")).toBe(3);
    expect(move(0, "ArrowLeft")).toBe(0);
    expect(move(9, "ArrowRight")).toBe(9);
  });

  it("moves one row up and down, staying put when there is no choice there", () => {
    expect(move(1, "ArrowDown")).toBe(5);
    expect(move(5, "ArrowUp")).toBe(1);
    expect(move(1, "ArrowUp")).toBe(1);
    expect(move(5, "ArrowDown")).toBe(9);
    expect(move(7, "ArrowDown")).toBe(7);
  });

  it("jumps to the first and the last choice", () => {
    expect(move(5, "Home")).toBe(0);
    expect(move(5, "End")).toBe(9);
  });

  it("is null for a key that doesn't move", () => {
    expect(move(5, "Enter")).toBeNull();
    expect(move(5, "a")).toBeNull();
  });

  it("works as a single row", () => {
    expect(gridMove(2, "ArrowDown", 8, 8)).toBe(2);
    expect(gridMove(2, "ArrowRight", 8, 8)).toBe(3);
  });
});

describe("the vault colour tokens", () => {
  const css: string = readFileSync(new URL("../app.css", import.meta.url), "utf8");

  function tokens(selector: string): Map<string, string> {
    const start = css.indexOf(selector + " {");
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, css.indexOf("}", start));
    return new Map([...block.matchAll(/(--[\w-]+):\s*(#[0-9a-f]{6});/g)].map((m) => [m[1], m[2]]));
  }

  /** WCAG relative luminance of `#rrggbb`. */
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

  it("measures contrast as WCAG does", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
  });

  // The top bar's background is --panel.
  it.each([
    ["light", ":root"],
    ["dark", ':root[data-theme="dark"]'],
  ])("each has at least 3:1 contrast against the top bar in the %s theme", (_, selector) => {
    const theme = tokens(selector);
    const panel = theme.get("--panel")!;
    for (const { id } of VAULT_COLORS) {
      const value = theme.get(`--vault-${id}`);
      expect(value, `--vault-${id}`).toBeDefined();
      expect(contrast(value!, panel), `--vault-${id} ${value} on ${panel}`).toBeGreaterThanOrEqual(3);
    }
  });
});
