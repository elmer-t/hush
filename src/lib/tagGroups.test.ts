import { describe, expect, it } from "vitest";
import { groupTags, PAGE_SIZE, shownCount } from "$lib/tagGroups";
import { fastestOf } from "$lib/testing/timing";

const info = (tag: string, notes: number, description = "") => ({ tag, notes, description });

describe("groupTags", () => {
  it("sorts by note count, highest first, then by name ignoring case", () => {
    const tags = [info("b", 3), info("Alpha", 3), info("zed", 9), info("apple", 3)];
    const { main } = groupTags(tags, (t) => t);
    expect(main.map((t) => t.tag)).toEqual(["zed", "Alpha", "apple", "b"]);
  });

  it("puts tags on fewer than 2 notes in the rare group, unless they have a description", () => {
    const tags = [info("once", 1), info("twice", 2), info("described", 0, "a sentence"), info("none", 0)];
    const { main, rare } = groupTags(tags, (t) => t);
    expect(main.map((t) => t.tag)).toEqual(["twice", "described"]);
    expect(rare.map((t) => t.tag)).toEqual(["once", "none"]);
  });

  it("groups other rows by the tag info they belong to", () => {
    const byTag = new Map([info("often", 5), info("rare", 1)].map((t) => [t.tag, t]));
    const rows = [{ tag: "rare" }, { tag: "often" }, { tag: "unknown" }];
    const { main, rare } = groupTags(rows, (r) => byTag.get(r.tag));
    expect(main.map((r) => r.tag)).toEqual(["often"]);
    expect(rare.map((r) => r.tag)).toEqual(["rare", "unknown"]);
  });

  it("handles 10,000 tags quickly", () => {
    const tags = Array.from({ length: 10_000 }, (_, i) => info(`tag${i}`, i % 7));
    // About 7 ms idle, so 200 ms leaves a busy CI runner 30x; the fastest of 3 runs
    // drops the ones another process or a GC pause slowed down.
    const { ms, result: { main, rare } } = fastestOf(3, () => groupTags(tags, (t) => t));
    expect(main.length + rare.length).toBe(10_000);
    expect(ms).toBeLessThan(200);
  });
});

describe("shownCount", () => {
  it("shows a page at a time, never more than there are", () => {
    expect(shownCount(10_000, 1)).toBe(PAGE_SIZE);
    expect(shownCount(10_000, 3)).toBe(3 * PAGE_SIZE);
    expect(shownCount(150, 3)).toBe(150);
    expect(shownCount(0, 1)).toBe(0);
  });
});
