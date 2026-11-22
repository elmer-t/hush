import { describe, expect, it } from "vitest";
import { dragFraction, handleGeometry, MIN_HANDLE_PX, pageScrollTop, scrollTopAt } from "$lib/scrollbar";

describe("handleGeometry", () => {
  it("is null when the content fits: no handle", () => {
    expect(handleGeometry({ scrollTop: 0, scrollHeight: 500, clientHeight: 500 }, 500)).toBeNull();
    expect(handleGeometry({ scrollTop: 0, scrollHeight: 300, clientHeight: 500 }, 500)).toBeNull();
    // Half a pixel of rounding is not something to scroll.
    expect(handleGeometry({ scrollTop: 0, scrollHeight: 500.4, clientHeight: 500 }, 500)).toBeNull();
  });

  it("sizes the handle as the shown share of the content", () => {
    expect(handleGeometry({ scrollTop: 0, scrollHeight: 2000, clientHeight: 500 }, 500)).toEqual({ size: 125, offset: 0 });
  });

  it("places the handle at the scrolled share of the free track", () => {
    // Free track 375 px, scroll range 1500 px.
    expect(handleGeometry({ scrollTop: 750, scrollHeight: 2000, clientHeight: 500 }, 500)).toEqual({ size: 125, offset: 187.5 });
    expect(handleGeometry({ scrollTop: 1500, scrollHeight: 2000, clientHeight: 500 }, 500)).toEqual({ size: 125, offset: 375 });
  });

  it("keeps a minimum handle in a very long note, still reaching both ends", () => {
    const long = { scrollHeight: 400_000, clientHeight: 500 };
    expect(handleGeometry({ ...long, scrollTop: 0 }, 500)).toEqual({ size: MIN_HANDLE_PX, offset: 0 });
    expect(handleGeometry({ ...long, scrollTop: 399_500 }, 500)).toEqual({ size: MIN_HANDLE_PX, offset: 500 - MIN_HANDLE_PX });
  });

  it("stays on the track when the scroll position is past an end (overscroll, a shrinking note)", () => {
    expect(handleGeometry({ scrollTop: -40, scrollHeight: 2000, clientHeight: 500 }, 500)!.offset).toBe(0);
    expect(handleGeometry({ scrollTop: 1900, scrollHeight: 2000, clientHeight: 500 }, 500)!.offset).toBe(375);
  });

  it("is null on a track too short to hold a handle", () => {
    expect(handleGeometry({ scrollTop: 0, scrollHeight: 2000, clientHeight: 20 }, 20)).toBeNull();
  });
});

describe("dragFraction", () => {
  // A 100 px handle on a 500 px track that starts at y = 50, grabbed 30 px below the handle's top.
  const drag = (pointerY: number) => dragFraction(pointerY, 30, { top: 50, height: 500 }, 100);

  it("is where the handle's top is on the free track", () => {
    expect(drag(80)).toBe(0);
    expect(drag(280)).toBe(0.5);
    expect(drag(480)).toBe(1);
  });

  it("stops at the ends when the pointer goes past them", () => {
    expect(drag(-200)).toBe(0);
    expect(drag(2000)).toBe(1);
  });

  it("is 0 when the handle fills the track", () => {
    expect(dragFraction(100, 0, { top: 0, height: 500 }, 500)).toBe(0);
  });
});

describe("scrollTopAt", () => {
  it("maps a fraction onto the scroll range as it is now", () => {
    expect(scrollTopAt(0.5, { scrollHeight: 2000, clientHeight: 500 })).toBe(750);
    // The same fraction after the editor measured more of the note and it got taller.
    expect(scrollTopAt(0.5, { scrollHeight: 2400, clientHeight: 500 })).toBe(950);
    expect(scrollTopAt(1, { scrollHeight: 2400, clientHeight: 500 })).toBe(1900);
    expect(scrollTopAt(0, { scrollHeight: 2400, clientHeight: 500 })).toBe(0);
  });

  it("is 0 when the content fits", () => {
    expect(scrollTopAt(0.7, { scrollHeight: 400, clientHeight: 500 })).toBe(0);
  });
});

describe("pageScrollTop", () => {
  const box = { scrollTop: 1000, scrollHeight: 4000, clientHeight: 800 };

  it("moves most of a page, keeping a strip of the old page in view", () => {
    expect(pageScrollTop(box, 1)).toBe(1700);
    expect(pageScrollTop(box, -1)).toBe(300);
  });

  it("stops at the ends", () => {
    expect(pageScrollTop({ ...box, scrollTop: 200 }, -1)).toBe(0);
    expect(pageScrollTop({ ...box, scrollTop: 3000 }, 1)).toBe(3200);
  });
});
