import { describe, expect, it } from "vitest";
import { folderChildren, folderNodes, sameFolders } from "$lib/folderTree";

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
