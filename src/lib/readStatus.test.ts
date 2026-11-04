import { beforeEach, describe, expect, it, vi } from "vitest";

// What `api.setReadStatus` was called with, and what it does next.
const calls: unknown[][] = [];
let reply: () => Promise<number[]> = async () => [];
vi.mock("$lib/api", () => ({
  api: {
    setReadStatus: (...a: unknown[]) => {
      calls.push(a);
      return reply();
    },
  },
}));

const { markRead, readMark, readStatusOf } = await import("$lib/readStatus.svelte");

beforeEach(() => {
  calls.length = 0;
});

describe("readMark", () => {
  it("marks partly read and read notes, not unread ones", () => {
    expect(readMark("unread")).toBeNull();
    expect(readMark("partlyRead")?.label).toBe("Partly read");
    expect(readMark("read")?.label).toBe("Read");
    expect(readMark("partlyRead")?.glyph).not.toBe(readMark("read")?.glyph);
  });
});

describe("markRead", () => {
  it("shows the new status in lists loaded before it", async () => {
    reply = async () => [1];
    const loaded = { id: 1, readStatus: "unread" as const };
    expect(readStatusOf(loaded)).toBe("unread");
    expect(await markRead([1, 2], "read")).toEqual([1]);
    expect(calls).toEqual([[[1, 2], "read"]]);
    expect(readStatusOf(loaded)).toBe("read");
    expect(readStatusOf({ id: 3, readStatus: "partlyRead" })).toBe("partlyRead");
  });

  it("changes nothing when saving fails", async () => {
    reply = () => Promise.reject(new Error("locked"));
    await expect(markRead([5], "read")).rejects.toThrow("locked");
    expect(readStatusOf({ id: 5, readStatus: "unread" })).toBe("unread");
  });
});
