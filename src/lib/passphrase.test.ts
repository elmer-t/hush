import { describe, expect, it } from "vitest";
import { PASSPHRASE_GUIDANCE, passphraseStrength, weakPassphraseQuestion } from "$lib/passphrase";

const level = (p: string) => passphraseStrength(p).level;

describe("passphraseStrength", () => {
  it("has nothing to say about an empty field", () => {
    expect(passphraseStrength("")).toEqual({ level: "empty", bits: 0, warning: null });
  });

  it("rates a passphrase under 8 characters like any other and advises a longer one", () => {
    const advice = "Shorter than the advised 8 characters.";
    for (const p of ["a", "kq", "1234", "ééééééé"]) {
      expect(passphraseStrength(p), p).toMatchObject({ level: "weak", warning: advice });
    }
    // Seven random characters are not weak, only shorter than advised.
    expect(passphraseStrength("Xq7#mLp")).toMatchObject({ level: "fair", warning: advice });
    // Characters, not bytes.
    expect(passphraseStrength("kq7#mLé2").warning).toBeNull();
    expect(PASSPHRASE_GUIDANCE).toBe("Use four or more random words. 8 characters or more is advised.");
  });

  it("keeps the common-password warning for a short common password", () => {
    for (const p of ["abc", "admin1"]) expect(passphraseStrength(p), p).toMatchObject({ level: "weak", warning: "This is one of the most common passwords." });
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
      expect(passphraseStrength(p).level).not.toBe("empty");
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

describe("weakPassphraseQuestion", () => {
  const time = (p: string) => weakPassphraseQuestion(p)?.match(/in (\w+)\./)?.[1];

  it("asks before a passphrase that falls within days, with the time it takes", () => {
    expect(weakPassphraseQuestion("1234")).toBe(
      "Someone with a copy of the vault file can guess this passphrase in seconds. Use it anyway?",
    );
    expect(time("password")).toBe("seconds");
    expect(time("Password1")).toBe("seconds");
    expect(time("kqzvw")).toBe("minutes");
    expect(time("kqzmtw")).toBe("hours");
  });

  it("asks nothing for an empty field or a passphrase that holds for longer", () => {
    for (const p of ["", "Xq7#mLp", "kq7#mLp2", "correct horse battery staple"]) {
      expect(weakPassphraseQuestion(p), p).toBeNull();
    }
  });
});
