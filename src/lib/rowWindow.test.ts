import { describe, expect, it } from "vitest";
import { rowWindow, scrollTopFor } from "$lib/rowWindow";

describe("rowWindow", () => {
  it("covers the rows in view and the overscan around them", () => {
    expect(rowWindow(0, 310, 10_000, 31, 5)).toEqual({ first: 0, last: 15 });
    expect(rowWindow(3100, 310, 10_000, 31, 5)).toEqual({ first: 95, last: 115 });
    // A row cut by an edge counts as in view.
    expect(rowWindow(3110, 310, 10_000, 31, 0)).toEqual({ first: 100, last: 111 });
  });

  it("stays inside the list", () => {
    expect(rowWindow(0, 310, 4, 31, 5)).toEqual({ first: 0, last: 4 });
    expect(rowWindow(0, 310, 0, 31, 5)).toEqual({ first: 0, last: 0 });
    // Scrolled past the end, as right after folders collapsed.
    expect(rowWindow(31_000, 310, 20, 31, 5)).toEqual({ first: 20, last: 20 });
    // Not measured yet: nothing in view, only the overscan.
    expect(rowWindow(0, 0, 100, 31, 5)).toEqual({ first: 0, last: 5 });
  });

  it("does not grow with the list", () => {
    const { first, last } = rowWindow(155_000, 900, 1_000_000, 31);
    expect(last - first).toBeLessThan(60);
  });
});

describe("scrollTopFor", () => {
  it("leaves a row in view where it is", () => {
    expect(scrollTopFor(5, 100, 310, 31, 30)).toBe(100);
  });

  it("moves a row above the view to the top and one below it to the bottom", () => {
    expect(scrollTopFor(2, 100, 310, 31, 30)).toBe(62);
    expect(scrollTopFor(50, 100, 310, 31, 30)).toBe(50 * 31 + 30 - 310);
    // A row cut by the bottom edge moves fully into view.
    expect(scrollTopFor(13, 100, 310, 31, 30)).toBe(13 * 31 + 30 - 310);
  });
});
