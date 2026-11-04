import { describe, expect, it } from "vitest";
import { activeFilterCount, emptyFilters, searchSummary, toQuery } from "./search";

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

describe("read status filter", () => {
  it("counts as a filter when set", () => {
    expect(activeFilterCount(emptyFilters())).toBe(0);
    for (const readStatus of ["unread", "partlyRead", "read", "notFinished"] as const) {
      expect(activeFilterCount({ ...emptyFilters(), readStatus })).toBe(1);
      expect(activeFilterCount({ ...emptyFilters(), readStatus, minRating: 3 })).toBe(2);
    }
  });

  it("is sent with the query, also without text", () => {
    expect(toQuery("", "a", emptyFilters(), 0, 50).readStatus).toBeNull();
    const q = toQuery("", "a", { ...emptyFilters(), readStatus: "notFinished" }, 0, 50);
    expect(q.readStatus).toBe("notFinished");
    expect(q.folder).toBe("a");
  });
});
