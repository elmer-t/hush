import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { searchScheduler, TYPING_DELAY_MS } from "$lib/searchTiming";

describe("searchScheduler", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  /** A scheduler that has already run its first search, with no filters and no folder. */
  function started() {
    const run = vi.fn();
    const s = searchScheduler(run);
    s.change({ text: "", scope: "" });
    vi.advanceTimersByTime(0);
    run.mockClear();
    return { run, s };
  }

  it("searches at once on the first run", () => {
    const run = vi.fn();
    searchScheduler(run).change({ text: "", scope: "" });
    vi.advanceTimersByTime(0);
    expect(run).toHaveBeenCalledOnce();
  });

  it("typing a word at normal speed starts one search after the pause, not one per letter", () => {
    const { run, s } = started();
    let text = "";
    for (const letter of "meeting") {
      text += letter;
      s.change({ text, scope: "" });
      vi.advanceTimersByTime(200);
    }
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(TYPING_DELAY_MS - 200);
    expect(run).toHaveBeenCalledOnce();
  });

  it("waits about 300 ms after the last keystroke", () => {
    expect(TYPING_DELAY_MS).toBe(300);
    const { run, s } = started();
    s.change({ text: "a", scope: "" });
    vi.advanceTimersByTime(299);
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledOnce();
  });

  it("searches at once when a filter or the folder changes", () => {
    const { run, s } = started();
    s.change({ text: "", scope: '{"tags":["work"]}' });
    vi.advanceTimersByTime(0);
    expect(run).toHaveBeenCalledOnce();
    s.change({ text: "", scope: '{"tags":["work"]}|Projects' });
    vi.advanceTimersByTime(0);
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("a filter change while typing searches at once, with the typed text", () => {
    const { run, s } = started();
    s.change({ text: "mee", scope: "" });
    s.change({ text: "mee", scope: "Projects" });
    vi.advanceTimersByTime(0);
    expect(run).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledOnce();
  });

  it("Enter searches at once and skips the wait", () => {
    const { run, s } = started();
    s.change({ text: "meeting", scope: "" });
    s.now();
    expect(run).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledOnce();
  });

  it("Enter with nothing waiting does not search again", () => {
    const { run, s } = started();
    s.now();
    expect(run).not.toHaveBeenCalled();
  });

  it("clearing the search updates at once", () => {
    const { run, s } = started();
    s.change({ text: "meeting", scope: "" });
    vi.advanceTimersByTime(TYPING_DELAY_MS);
    run.mockClear();
    s.change({ text: "", scope: "" });
    vi.advanceTimersByTime(0);
    expect(run).toHaveBeenCalledOnce();
  });

  it("cancel drops a waiting search", () => {
    const { run, s } = started();
    s.change({ text: "meeting", scope: "" });
    s.cancel();
    vi.advanceTimersByTime(1000);
    s.now();
    expect(run).not.toHaveBeenCalled();
  });
});
