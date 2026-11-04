import { describe, expect, it } from "vitest";
import { searchSummary } from "./search";

describe("searchSummary", () => {
  it("says the order the results are in", () => {
    expect(searchSummary(3, false, "mailto", false)).toBe("3 results · best match first");
    expect(searchSummary(50, true, "mailto", true)).toBe("50+ results · newest first");
  });

  it("names no order without search text", () => {
    expect(searchSummary(1, false, "  ", false)).toBe("1 result");
    expect(searchSummary(1, true, "", false)).toBe("1+ results");
  });
});
