import { describe, expect, it } from "vitest";
import { MIN_PASSPHRASE_CHARS, passphraseStrength } from "$lib/passphrase";
import rustDb from "../../src-tauri/src/db/mod.rs?raw";

const level = (p: string) => passphraseStrength(p).level;

describe("MIN_PASSPHRASE_CHARS", () => {
  it("matches the minimum Rust enforces", () => {
    expect(rustDb).toContain(`pub const MIN_PASSPHRASE_CHARS: usize = ${MIN_PASSPHRASE_CHARS};`);
  });
});

describe("passphraseStrength", () => {
  it("has nothing to say about an empty field", () => {
    expect(passphraseStrength("")).toEqual({ level: "empty", bits: 0, warning: null });
  });

  it("calls anything under the minimum too short, counting characters not bytes", () => {
    expect(MIN_PASSPHRASE_CHARS).toBe(8);
    expect(level("a")).toBe("tooShort");
    expect(level("Xq7#mLp")).toBe("tooShort");
    expect(level("ééééééé")).toBe("tooShort");
    expect(passphraseStrength("abc").warning).toBe("Too short: use at least 8 characters.");
    expect(level("éééééééé")).not.toBe("tooShort");
  });

  it("warns about common passwords, also with leet spelling, capitals and digits added", () => {
    for (const p of ["password", "Password1", "p@ssw0rd!", "iloveyou", "sunshine123", "wachtwoord", "Welcome2024"]) {
      const s = passphraseStrength(p);
      expect(s.level, p).toBe("weak");
      expect(s.warning, p).toBe("This is one of the most common passwords.");
    }
  });

  it("warns about keyboard patterns and sequences", () => {
    for (const p of ["qwertyuiop", "asdfghjkl;", "12345678", "abcdefgh", "87654321", "1qaz2wsx3edc", "azertyuiop"]) {
      const s = passphraseStrength(p);
      expect(s.level, p).toBe("weak");
      expect(s.warning, p).toBe("Keyboard patterns and sequences are easy to guess.");
    }
  });

  it("warns about repeated characters", () => {
    for (const p of ["aaaaaaaa", "11111111", "abababab", "xyzxyzxyz", "horse horse horse horse"]) {
      const s = passphraseStrength(p);
      expect(s.level, p).toBe("weak");
      expect(s.warning, p).toBe("Repeated characters or words are easy to guess.");
    }
  });

  it("warns about a few common words or a word and a year", () => {
    for (const p of ["summer2024", "ilovemydog", "happy family", "garden1987"]) {
      const s = passphraseStrength(p);
      expect(s.level, p).toBe("weak");
      expect(s.warning, p).toBe("Common words and dates are easy to guess. Add more words.");
    }
  });

  it("calls a short random string fair, not weak", () => {
    expect(level("kq7#mLp2")).toBe("fair");
  });

  it("calls four or more random words strong", () => {
    for (const p of ["correct horse battery staple", "velvet orbit pickle tundra", "lantaarn fiets regen kaas molen"]) {
      const s = passphraseStrength(p);
      expect(s.level, p).toBe("strong");
      expect(s.warning, p).toBeNull();
    }
  });

  it("calls a long random string strong", () => {
    expect(level("G7v!q2Lz#p9Wm4Rx")).toBe("strong");
  });

  it("stays fast on long input", () => {
    const started = performance.now();
    for (const p of ["a".repeat(500), "ab".repeat(250), "velvet orbit pickle tundra ".repeat(20)]) {
      expect(passphraseStrength(p).level).not.toBe("tooShort");
    }
    expect(performance.now() - started).toBeLessThan(1000);
  });

  it("grows with length", () => {
    const a = passphraseStrength("velvet orbit").bits;
    const b = passphraseStrength("velvet orbit pickle").bits;
    const c = passphraseStrength("velvet orbit pickle tundra").bits;
    expect(a).toBeLessThan(b);
    expect(b).toBeLessThan(c);
  });
});
