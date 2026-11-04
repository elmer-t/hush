import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ABOVE_ACTIVITY_MIN,
  ACTIVITY,
  debounced,
  fitHeight,
  fitWidths,
  isBoolean,
  isSplit,
  jobsWidth,
  KEYS,
  LEFT,
  loadStored,
  loadWidth,
  oneOf,
  RIGHT,
  saveStored,
  SPLIT,
} from "$lib/layout";
import { loadDisplay } from "$lib/graph/settings.svelte";

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

afterEach(() => vi.unstubAllGlobals());

describe("loadStored", () => {
  it("reads back what was saved", () => {
    fakeStorage();
    saveStored(KEYS.showLeft, false);
    saveStored(KEYS.view, "graph");
    expect(loadStored(KEYS.showLeft, true, isBoolean)).toBe(false);
    expect(loadStored(KEYS.view, "note", oneOf("note", "graph"))).toBe("graph");
  });

  it("falls back on nothing stored, bad JSON, a wrong type or an unknown value", () => {
    const data = fakeStorage();
    expect(loadStored(KEYS.showLeft, true, isBoolean)).toBe(true);
    for (const raw of ["{not json", '"yes"', "1", "null", "[true]"]) {
      data.set(KEYS.showLeft, raw);
      expect(loadStored(KEYS.showLeft, true, isBoolean), raw).toBe(true);
    }
    // Settings, Import and Trash are never reopened.
    for (const view of ["settings", "import", "trash", "other"]) {
      data.set(KEYS.view, JSON.stringify(view));
      expect(loadStored(KEYS.view, "note", oneOf("note", "graph")), view).toBe("note");
    }
  });

  it("falls back when storage is unavailable, and saving doesn't throw", () => {
    fakeStorage(true);
    expect(loadStored(KEYS.previewOnly, false, isBoolean)).toBe(false);
    expect(() => saveStored(KEYS.previewOnly, true)).not.toThrow();
    expect(loadWidth(LEFT)).toBe(LEFT.default);
  });
});

describe("loadDisplay (graph controls)", () => {
  it("is today's defaults with nothing stored", () => {
    fakeStorage();
    expect(loadDisplay()).toMatchObject({ mode: "local", depth: 1, threshold: 0.75, meaning: { local: true, global: false } });
  });

  it("keeps valid stored controls and resets each invalid one to its default", () => {
    const data = fakeStorage();
    data.set(
      KEYS.graph,
      JSON.stringify({
        mode: "global",
        depth: "3",
        threshold: 0.5,
        cap: null,
        colorBy: "rainbow",
        meaning: { local: false },
        ghosts: true,
        scopeFolder: "should be ignored",
      }),
    );
    const d = loadDisplay();
    expect(d.mode).toBe("global");
    expect(d.threshold).toBe(0.5);
    expect(d.ghosts).toBe(true);
    expect(d.depth).toBe(1);
    expect(d.cap).toBe(5000);
    expect(d.colorBy).toBe("folder");
    expect(d.meaning).toEqual({ local: true, global: false });
    expect(d).not.toHaveProperty("scopeFolder");
    expect(d.rareTags).toBe(false);
  });

  it("is the defaults when the stored value isn't an object", () => {
    const data = fakeStorage();
    for (const raw of ["[]", "42", "garbage"]) {
      data.set(KEYS.graph, raw);
      expect(loadDisplay().mode, raw).toBe("local");
    }
  });
});

describe("debounced", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("runs once with the last value, and flush runs it at once", () => {
    const run = vi.fn();
    const d = debounced<number>(300, run);
    d.call(1);
    d.call(2);
    vi.advanceTimersByTime(299);
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledExactlyOnceWith(2);
    d.call(3);
    d.flush();
    expect(run).toHaveBeenLastCalledWith(3);
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledTimes(2);
    d.call(4);
    d.cancel();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledTimes(2);
  });
});

describe("fitWidths", () => {
  it("leaves no room for a hidden details panel", () => {
    expect(fitWidths(1280, 280, null)).toEqual({ left: 280, right: 0 });
    expect(fitWidths(2000, 280, 300)).toEqual({ left: 280, right: 300 });
  });

  it("shrinks the shown columns towards their minimums when the window is narrow", () => {
    const { left, right } = fitWidths(800, 400, 400);
    expect(left + right).toBe(800 - 360);
    expect(left).toBeGreaterThanOrEqual(LEFT.min);
    expect(right).toBeGreaterThanOrEqual(RIGHT.min);
  });
});

describe("fitHeight (Activity panel)", () => {
  it("shows the stored height when the window has room", () => {
    expect(fitHeight(1000, 300)).toBe(300);
  });

  it("gives a short window's header and editor their minimum, down to the panel's minimum", () => {
    expect(fitHeight(500, 400)).toBe(500 - ABOVE_ACTIVITY_MIN);
    expect(fitHeight(300, 400)).toBe(ACTIVITY.min);
  });

  it("stores and loads the height like a width", () => {
    const data = fakeStorage();
    expect(loadWidth(ACTIVITY)).toBe(ACTIVITY.default);
    data.set(ACTIVITY.storageKey, "5000");
    expect(loadWidth(ACTIVITY)).toBe(ACTIVITY.max);
  });
});

describe("jobsWidth (Activity panel split)", () => {
  it("is the stored share of the width when both sides have room", () => {
    expect(jobsWidth(1000, SPLIT.default)).toBe(600);
    expect(jobsWidth(1000, 0.4)).toBe(400);
  });

  it("keeps each side's minimum, the Event log's first", () => {
    expect(jobsWidth(1000, 0.1)).toBe(SPLIT.jobsMin);
    expect(jobsWidth(1000, 0.95)).toBe(1000 - SPLIT.logMin);
    expect(jobsWidth(500, 0.6)).toBe(500 - SPLIT.logMin);
    expect(jobsWidth(200, 0.6)).toBe(0);
  });

  it("only loads a share between 0 and 1", () => {
    const data = fakeStorage();
    expect(loadStored(KEYS.activitySplit, SPLIT.default as number, isSplit)).toBe(SPLIT.default);
    for (const raw of ["0", "1", "-0.2", '"0.5"', "null"]) {
      data.set(KEYS.activitySplit, raw);
      expect(loadStored(KEYS.activitySplit, SPLIT.default as number, isSplit), raw).toBe(SPLIT.default);
    }
    saveStored(KEYS.activitySplit, 0.35);
    expect(loadStored(KEYS.activitySplit, SPLIT.default as number, isSplit)).toBe(0.35);
  });
});
