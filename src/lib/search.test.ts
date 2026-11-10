import { describe, expect, it } from "vitest";
import type { IndexStatus, ModelStatus } from "$lib/api";
import {
  activeFilterCount,
  emptyFilters,
  meaningCoverage,
  meaningOff,
  scopeLabel,
  searchFolder,
  searchKey,
  searchPlaceholder,
  searchSummary,
  shortPath,
  toQuery,
  whyLabel,
} from "./search";

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

describe("search by meaning", () => {
  it("says the mode in the summary, and the cut", () => {
    expect(searchSummary(42, false, "stuck", false, "Projects/Work", { byMeaning: true, top: null })).toBe(
      "42 results in Projects/Work · by meaning",
    );
    expect(searchSummary(200, true, "stuck", false, null, { byMeaning: true, top: 1000 })).toBe(
      "200+ results in whole vault · by meaning, top 1,000",
    );
    // Searched by words after all (a quoted phrase): the words order as before.
    expect(searchSummary(3, false, '"stuck"', false, undefined, { byMeaning: false, top: null })).toBe(
      "3 results · best match first",
    );
  });

  it("sends the mode and treats it as part of the search", () => {
    expect(toQuery("x", null, emptyFilters(), 0, 50).mode).toBe("words");
    expect(toQuery("x", null, emptyFilters(), 0, 50, "meaning").mode).toBe("meaning");
    const f = emptyFilters();
    expect(searchKey(f, "a", "folder", "words")).not.toBe(searchKey(f, "a", "folder", "meaning"));
  });

  it("names what found each hit", () => {
    expect(whyLabel({ words: true, cosine: null })).toEqual({ text: "words", title: "Found by its words" });
    expect(whyLabel({ words: false, cosine: 0.7341 })).toEqual({ text: "meaning", title: "Found by meaning, cosine 0.734" });
    expect(whyLabel({ words: true, cosine: 0.8 })).toEqual({ text: "both", title: "Found by its words and by meaning, cosine 0.800" });
  });

  it("is off with the reason while features that need the model are", () => {
    const status = (reduced: ModelStatus["reduced"]): ModelStatus => ({ model: "m", readyModel: "m", load: { state: "ready" }, reduced });
    expect(meaningOff(null)).toBeNull();
    expect(meaningOff(status(null))).toBeNull();
    expect(meaningOff(status({ reason: "missing", message: "model file m.onnx is missing" }))).toBe(
      "Search by meaning is off: model file m.onnx is missing",
    );
  });

  it("says how much of the vault meaning covers while indexing runs", () => {
    const ready: ModelStatus = { model: "m", readyModel: "m", load: { state: "ready" }, reduced: null };
    const indexing = (done: number, complete: number, total: number): IndexStatus => ({ state: "indexing", done, complete, total, etaSecs: null });
    expect(meaningCoverage(ready, indexing(412000, 412000, 1000000))).toBe("meaning covers 412,000 of 1,000,000 notes");
    expect(meaningCoverage(ready, indexing(1000000, 400000, 1000000))).toBe(
      "meaning covers 1,000,000 of 1,000,000 notes (first part only for 600,000)",
    );
    expect(meaningCoverage(ready, indexing(10, 10, 10))).toBeNull();
    expect(meaningCoverage(ready, { state: "idle" })).toBeNull();
    // During a switch the counts are the new model's; the old one serves every note.
    expect(meaningCoverage({ ...ready, model: "new" }, indexing(1, 1, 10))).toBeNull();
  });
});
