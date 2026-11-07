import { describe, expect, it } from "vitest";
import { formatSize } from "$lib/size";

describe("formatSize", () => {
  it("shows whole bytes under 1000", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(1)).toBe("1 B");
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(999)).toBe("999 B");
  });

  it("uses decimal units: 1 kB is 1000 B", () => {
    expect(formatSize(1000)).toBe("1.0 kB");
    expect(formatSize(1024)).toBe("1.0 kB");
    expect(formatSize(4_200)).toBe("4.2 kB");
    expect(formatSize(1_500_000)).toBe("1.5 MB");
  });

  it("shows one decimal under 10 of a unit and whole numbers from 10", () => {
    expect(formatSize(9_949)).toBe("9.9 kB");
    expect(formatSize(9_950)).toBe("10 kB");
    expect(formatSize(42_000)).toBe("42 kB");
    expect(formatSize(420_000)).toBe("420 kB");
  });

  it("rounds half up on the shown digit", () => {
    expect(formatSize(1_150)).toBe("1.2 kB");
    expect(formatSize(1_149)).toBe("1.1 kB");
    expect(formatSize(10_500)).toBe("11 kB");
  });

  it("never shows 1000 of a unit: it moves to the next one", () => {
    expect(formatSize(999_499)).toBe("999 kB");
    expect(formatSize(999_500)).toBe("1.0 MB");
    expect(formatSize(999_499_999)).toBe("999 MB");
    expect(formatSize(999_500_000)).toBe("1.0 GB");
  });

  it("stays in GB above that, with no larger unit", () => {
    expect(formatSize(1_000_000_000)).toBe("1.0 GB");
    expect(formatSize(9_949_999_999)).toBe("9.9 GB");
    expect(formatSize(42_000_000_000)).toBe("42 GB");
    expect(formatSize(2_500_000_000_000)).toBe("2500 GB");
  });

  it("treats nonsense as nothing", () => {
    expect(formatSize(-5)).toBe("0 B");
    expect(formatSize(Number.NaN)).toBe("0 B");
  });
});
