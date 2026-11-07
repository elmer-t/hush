import { describe, expect, it } from "vitest";
import { folderChildren, folderNodes, sameFolders, subtreeSizes } from "$lib/folderTree";

describe("folderChildren", () => {
  it("adds ancestors that hold no notes and sorts each folder's children by name", () => {
    const map = folderChildren(["b/x", "a", "b/c/d"]);
    expect(map.get("")).toEqual([
      { path: "a", name: "a" },
      { path: "b", name: "b" },
    ]);
    expect(map.get("b")).toEqual([
      { path: "b/c", name: "c" },
      { path: "b/x", name: "x" },
    ]);
    expect(map.get("b/c")).toEqual([{ path: "b/c/d", name: "d" }]);
  });

  it("treats paths case-insensitively, keeping the first spelling", () => {
    const map = folderChildren(["Projects", "projects/Plan", "PROJECTS/plan"]);
    expect(map.get("")).toEqual([{ path: "Projects", name: "Projects" }]);
    expect(map.get("projects")).toEqual([{ path: "projects/Plan", name: "Plan" }]);
  });

  it("ignores the top level and empty segments", () => {
    const map = folderChildren(["", "a//b"]);
    expect(map.get("")).toEqual([{ path: "a", name: "a" }]);
    expect(map.get("a")).toEqual([{ path: "a/b", name: "b" }]);
  });

  it("sorts names the way the locale does, not by code point", () => {
    const map = folderChildren(["b", "a", "B", "Ä"]);
    expect(map.get("")!.map((c) => c.name)).toEqual(["a", "Ä", "b"]);
  });

  it("builds 50,000 folders under one parent quickly", () => {
    const folders = Array.from({ length: 50_000 }, (_, i) => `Archive/Item ${i}`);
    // The quadratic sibling lookup this replaces took 80 s here.
    const start = performance.now();
    const nodes = folderNodes(folders);
    const elapsed = performance.now() - start;
    expect(nodes[0].children).toHaveLength(50_000);
    expect(elapsed).toBeLessThan(200);
  });
});

describe("folderNodes", () => {
  it("nests the hierarchy", () => {
    expect(folderNodes(["a/b", "c"])).toEqual([
      { path: "a", name: "a", children: [{ path: "a/b", name: "b", children: [] }] },
      { path: "c", name: "c", children: [] },
    ]);
  });
});

describe("sameFolders", () => {
  it("compares lists in order", () => {
    expect(sameFolders(["a", "b"], ["a", "b"])).toBe(true);
    expect(sameFolders(["a", "b"], ["b", "a"])).toBe(false);
    expect(sameFolders(["a"], ["a", "b"])).toBe(false);
    expect(sameFolders([], [])).toBe(true);
  });
});

describe("subtreeSizes", () => {
  it("rolls each folder's bytes up to every ancestor, implied ones too", () => {
    const totals = subtreeSizes([
      { path: "", bytes: 1 },
      { path: "a", bytes: 2 },
      { path: "a/b/c", bytes: 5 },
      { path: "d/e/f", bytes: 7 },
    ]);
    expect(Object.fromEntries(totals)).toEqual({
      "": 15,
      a: 7,
      "a/b": 5,
      "a/b/c": 5,
      d: 7,
      "d/e": 7,
      "d/e/f": 7,
    });
  });

  it("keys paths case-insensitively and skips empty segments, like the tree", () => {
    const totals = subtreeSizes([
      { path: "Projects", bytes: 3 },
      { path: "projects/Plan", bytes: 4 },
      { path: "x//y/", bytes: 6 },
    ]);
    expect(totals.get("projects")).toBe(7);
    expect(totals.get("projects/plan")).toBe(4);
    expect(totals.get("x")).toBe(6);
    expect(totals.get("x/y")).toBe(6);
    expect(totals.get("")).toBe(13);
  });

  it("gives an empty vault a zero total", () => {
    expect(Object.fromEntries(subtreeSizes([]))).toEqual({ "": 0 });
  });

  it("rolls up 50,000 deep folders quickly", () => {
    const sizes = Array.from({ length: 50_000 }, (_, i) => ({ path: `A/B ${i % 100}/C ${i}`, bytes: 10 }));
    const start = performance.now();
    const totals = subtreeSizes(sizes);
    const elapsed = performance.now() - start;
    expect(totals.get("")).toBe(500_000);
    expect(totals.get("a")).toBe(500_000);
    expect(totals.get("a/b 7")).toBe(5_000);
    expect(elapsed).toBeLessThan(200);
  });
});
