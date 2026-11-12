import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { KEYS } from "$lib/layout";
// @ts-expect-error type error without @types/node package
import { readFileSync } from "node:fs";

/** A Map-backed localStorage; `broken` makes every access throw, as in a locked-down webview. */
function fakeStorage(broken = false) {
  const data = new Map<string, string>();
  const guard = () => {
    if (broken) throw new Error("storage unavailable");
  };
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => (guard(), data.get(k) ?? null),
    setItem: (k: string, v: string) => (guard(), void data.set(k, v)),
    removeItem: (k: string) => (guard(), void data.delete(k)),
  });
  return data;
}

/** A window whose OS scheme can be flipped with `setDark`, and a <html> to put data-theme on. */
function fakeWindow(dark: boolean) {
  const listeners = new Set<() => void>();
  const media = {
    get matches() {
      return dark;
    },
    addEventListener: (_: string, l: () => void) => listeners.add(l),
    removeEventListener: (_: string, l: () => void) => listeners.delete(l),
  };
  const root = { dataset: {} as Record<string, string> };
  vi.stubGlobal("window", { matchMedia: () => media });
  vi.stubGlobal("document", { documentElement: root });
  return {
    root,
    setDark(next: boolean) {
      dark = next;
      for (const l of listeners) l();
    },
  };
}

/** A fresh copy of the module, as its state is set once per app start. */
const freshTheme = async () => {
  vi.resetModules();
  return import("$lib/theme");
};

afterEach(() => vi.unstubAllGlobals());

describe("loadThemePreference", () => {
  it("is System on a fresh install", async () => {
    fakeStorage();
    const { loadThemePreference } = await freshTheme();
    expect(loadThemePreference()).toBe("system");
  });

  it("reads back each valid choice", async () => {
    const data = fakeStorage();
    const { loadThemePreference } = await freshTheme();
    for (const value of ["system", "light", "dark"]) {
      data.set(KEYS.theme, JSON.stringify(value));
      expect(loadThemePreference()).toBe(value);
    }
  });

  it("falls back to System on a broken or unknown value", async () => {
    const data = fakeStorage();
    const { loadThemePreference } = await freshTheme();
    for (const raw of ["{not json", '"Dark"', '"auto"', "1", "null", '["dark"]', ""]) {
      data.set(KEYS.theme, raw);
      expect(loadThemePreference(), raw).toBe("system");
    }
  });

  it("falls back to System when storage is unavailable", async () => {
    fakeStorage(true);
    const { loadThemePreference } = await freshTheme();
    expect(loadThemePreference()).toBe("system");
  });
});

describe("resolveScheme", () => {
  it("follows the OS for System and ignores it otherwise", async () => {
    const { resolveScheme } = await freshTheme();
    expect(resolveScheme("system", false)).toBe("light");
    expect(resolveScheme("system", true)).toBe("dark");
    expect(resolveScheme("light", true)).toBe("light");
    expect(resolveScheme("light", false)).toBe("light");
    expect(resolveScheme("dark", false)).toBe("dark");
    expect(resolveScheme("dark", true)).toBe("dark");
  });
});

describe("applying the theme", () => {
  beforeEach(() => fakeStorage());

  it("applies the stored choice on init, over the OS", async () => {
    localStorage.setItem(KEYS.theme, JSON.stringify("dark"));
    const win = fakeWindow(false);
    const theme = await freshTheme();
    theme.initTheme();
    expect(win.root.dataset.theme).toBe("dark");
    expect(theme.currentScheme()).toBe("dark");
    expect(theme.themePreference()).toBe("dark");
  });

  it("follows OS changes with System, and tells listeners", async () => {
    const win = fakeWindow(false);
    const theme = await freshTheme();
    theme.initTheme();
    const seen: string[] = [];
    theme.onSchemeChange((s) => seen.push(s));
    expect(win.root.dataset.theme).toBe("light");
    win.setDark(true);
    expect(win.root.dataset.theme).toBe("dark");
    win.setDark(false);
    expect(seen).toEqual(["dark", "light"]);
  });

  it("ignores OS changes once Light or Dark is chosen", async () => {
    const win = fakeWindow(false);
    const theme = await freshTheme();
    theme.initTheme();
    theme.setThemePreference("light");
    const seen: string[] = [];
    theme.onSchemeChange((s) => seen.push(s));
    win.setDark(true);
    expect(win.root.dataset.theme).toBe("light");
    expect(seen).toEqual([]);
  });

  it("switches and stores a manual choice, notifying only on a real change", async () => {
    const win = fakeWindow(true);
    const theme = await freshTheme();
    theme.initTheme();
    const seen: string[] = [];
    const stop = theme.onSchemeChange((s) => seen.push(s));
    theme.setThemePreference("dark"); // already dark via the OS
    theme.setThemePreference("light");
    expect(win.root.dataset.theme).toBe("light");
    expect(localStorage.getItem(KEYS.theme)).toBe('"light"');
    theme.setThemePreference("system");
    expect(win.root.dataset.theme).toBe("dark");
    stop();
    theme.setThemePreference("light");
    expect(seen).toEqual(["light", "dark"]);
  });
});

describe("app.css", () => {
  /** The declarations of the first rule whose selector is `selector`, normalised. */
  function declarations(css: string, selector: string): string[] {
    const start = css.indexOf(selector + " {");
    expect(start, selector).toBeGreaterThanOrEqual(0);
    const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
    return body
      .split(";")
      .map((d) => d.trim())
      .filter(Boolean);
  }

  it("has the same dark tokens for an explicit Dark and for a dark OS on System", () => {
    const css: string = readFileSync(new URL("../app.css", import.meta.url), "utf8");
    const explicit = declarations(css, ':root[data-theme="dark"]');
    const media = declarations(css, ':root:not([data-theme="light"])');
    expect(explicit.length).toBeGreaterThan(30);
    expect(media).toEqual(explicit);
  });
});
