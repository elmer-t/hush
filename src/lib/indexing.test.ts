import { describe, expect, it } from "vitest";
import { indexText, indexing } from "./indexing";

const at = (done: number, complete: number, total: number) =>
  ({ state: "indexing", done, complete, total, etaSecs: null }) as const;

describe("indexing", () => {
  it("covers both phases and nothing else", () => {
    expect(indexing({ state: "idle" })).toBeNull();
    expect(indexing(at(5000, 5000, 5000))).toBeNull();
    expect(indexing(at(1200, 300, 5000))).not.toBeNull();
    expect(indexing(at(5000, 3000, 5000))).not.toBeNull();
  });

  it("says how many notes are indexed and how many complete", () => {
    expect(indexText(at(1200, 300, 5000))).toBe(
      `${(1200).toLocaleString()} of ${(5000).toLocaleString()} notes indexed, 300 of ${(1200).toLocaleString()} complete`,
    );
    const n = (x: number) => x.toLocaleString();
    expect(indexText(at(5000, 3000, 5000))).toBe(`${n(5000)} of ${n(5000)} notes indexed, ${n(3000)} of ${n(5000)} complete`);
    expect(indexText(at(10, 10, 40))).toBe("10 of 40 notes indexed");
  });
});
