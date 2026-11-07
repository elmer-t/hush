import { describe, expect, it } from "vitest";
import { activeFilterCount, emptyFilters, scopeLabel, searchFolder, searchPlaceholder, searchSummary, shortPath, toQuery } from "./search";

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

describe("search scope", () => {
  it("names the folder a search is limited to", () => {
    expect(searchSummary(42, false, "plan", false, "Projects/Work")).toBe("42 results in Projects/Work · best match first");
    expect(searchSummary(1, false, "", false, "Projects")).toBe("1 result in Projects");
  });

  it("says whole vault when nothing limits the search", () => {
    expect(searchSummary(42, false, "plan", false, null)).toBe("42 results in whole vault · best match first");
    expect(searchSummary(50, true, "mailto", true, null)).toBe("50+ results in whole vault · newest first");
    expect(searchSummary(1, true, "", false, null)).toBe("1+ results in whole vault");
    expect(scopeLabel(null)).toBe("whole vault");
  });

  it("keeps count, plural and order as before", () => {
    expect(searchSummary(1, false, "x", true, "A")).toBe("1 result in A · newest first");
    expect(searchSummary(0, false, "x", false, "A")).toBe("0 results in A · best match first");
    expect(searchSummary(1234, true, "x", false, "A")).toBe(`${(1234).toLocaleString()}+ results in A · best match first`);
  });

  it("shortens a long path from the middle, keeping the first and last segments", () => {
    expect(shortPath("Projects/Work", 24)).toBe("Projects/Work");
    expect(shortPath("Projects/Clients/Acme/Meetings/2026", 24)).toBe("Projects/…/Meetings/2026");
    expect(shortPath("Archive/a/b/c/Quarterly reports", 24)).toBe("…/Quarterly reports");
    const long = shortPath("Inbox/Some very long folder name indeed", 24);
    expect(long.endsWith("Some very long folder name indeed")).toBe(true);
    expect(long.startsWith("…/")).toBe(true);
    expect(scopeLabel("Projects/Clients/Acme/Meetings/2026")).toBe("Projects/…/Meetings/2026");
  });

  it("queries the selected folder, or the whole vault with folder null", () => {
    expect(searchFolder("Projects/Work", "folder", true)).toBe("Projects/Work");
    expect(searchFolder("Projects/Work", "vault", true)).toBeNull();
    expect(searchFolder(null, "folder", true)).toBeNull();
    expect(toQuery("plan", searchFolder("Projects/Work", "vault", true), emptyFilters(), 0, 50).folder).toBeNull();
    expect(toQuery("plan", searchFolder("Projects/Work", "folder", true), emptyFilters(), 0, 50).folder).toBe("Projects/Work");
  });

  it("keeps browsing the selected folder while not searching", () => {
    expect(searchFolder("Projects", "vault", false)).toBe("Projects");
  });

  it("shows the scope in the idle search box", () => {
    expect(searchPlaceholder("Projects/Work", "folder")).toBe("Search Projects/Work");
    expect(searchPlaceholder("Projects/Work", "vault")).toBe("Search notes");
    expect(searchPlaceholder(null, "folder")).toBe("Search notes");
  });
});
