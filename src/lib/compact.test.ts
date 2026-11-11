import { describe, expect, it } from "vitest";
import { elapsedText, compactNote } from "$lib/compact";
import type { VaultSpace } from "$lib/api";

const space = (extra: Partial<VaultSpace> = {}): VaultSpace => ({
  fileBytes: 15e9,
  reclaimableBytes: 9e9,
  trashNotes: 0,
  trashTextBytes: 0,
  neededBytes: 6.6e9,
  incremental: false,
  worthCompacting: true,
  ...extra,
});

describe("elapsedText", () => {
  it("shows minutes and seconds, then hours", () => {
    expect(elapsedText(0)).toBe("0:00");
    expect(elapsedText(7_400)).toBe("0:07");
    expect(elapsedText(725_000)).toBe("12:05");
    expect(elapsedText(3_723_000)).toBe("1:02:03");
    expect(elapsedText(-5)).toBe("0:00");
  });
});

describe("compactNote", () => {
  it("says how much compacting gives back", () => {
    expect(compactNote(space())).toBe("Compacting gives back 9.0 GB.");
  });

  it("says why there is nothing to compact", () => {
    expect(compactNote(space({ reclaimableBytes: 4e6, worthCompacting: false }))).toBe(
      "Only 4.0 MB can be given back; compacting starts to pay off from 10 MB and 1% of the file.",
    );
    expect(compactNote(space({ reclaimableBytes: 0, worthCompacting: false }))).toBe("Nothing to give back.");
  });

  it("says when the vault gives space back by itself", () => {
    expect(compactNote(space({ reclaimableBytes: 0, worthCompacting: false, incremental: true }))).toBe(
      "Nothing to give back. Space freed by emptying the trash is given back right away.",
    );
  });
});
