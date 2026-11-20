import { afterEach, describe, expect, it } from "vitest";
import { clockTime, dateTime, longDate, setDateLocale, shortDate } from "$lib/dates";

// 20 November 2026, 20:50:07 local time, whatever zone the tests run in.
const at = new Date(2026, 10, 20, 20, 50, 7).getTime();
const noon = new Date(2026, 10, 20, 12, 0, 0).getTime();
const nextDay = new Date(2026, 10, 21, 9, 0, 0).getTime();

// ICU puts a narrow no-break space before AM/PM; the tests don't care which space it is.
const plain = (s: string) => s.replace(/\s/g, " ");

afterEach(() => setDateLocale(null));

describe("with the operating system's format set to Dutch (Netherlands)", () => {
  it("shows a date and time day-first with a 24-hour clock", () => {
    setDateLocale("nl-NL");
    expect(plain(longDate(at))).toBe("20 nov 2026, 20:50");
  });

  it("shows only the time for today and only the date for another day", () => {
    setDateLocale("nl-NL");
    expect(plain(shortDate(at, noon))).toBe("20:50");
    expect(plain(shortDate(at, nextDay))).toBe("20 nov 2026");
  });

  it("shows a clock time with seconds", () => {
    setDateLocale("nl-NL");
    expect(plain(clockTime(at))).toBe("20:50:07");
  });

  it("shows a numeric date and time with seconds", () => {
    setDateLocale("nl-NL");
    expect(plain(dateTime(at))).toBe("20-11-2026, 20:50:07");
  });
});

describe("with the operating system's format set to English (United States)", () => {
  it("shows month-first dates and a 12-hour clock", () => {
    setDateLocale("en-US");
    expect(plain(longDate(at))).toBe("Nov 20, 2026, 8:50 PM");
    expect(plain(shortDate(at, noon))).toBe("8:50 PM");
    expect(plain(clockTime(at))).toBe("08:50:07 PM");
    expect(plain(dateTime(at))).toBe("11/20/2026, 8:50:07 PM");
  });
});

describe("the operating system's locale as it arrives", () => {
  it("accepts the form Linux uses", () => {
    for (const name of ["nl_NL", "nl_NL.UTF-8", "nl_NL@euro"]) {
      setDateLocale(name);
      expect(plain(longDate(at))).toBe("20 nov 2026, 20:50");
    }
  });

  it("switches format when the locale changes", () => {
    setDateLocale("en-US");
    longDate(at);
    setDateLocale("de-DE");
    expect(plain(longDate(at))).toBe("20.11.2026, 20:50");
  });

  it("keeps working when the name is not a locale", () => {
    for (const bad of ["", "C", "not a locale", "x-custom"]) {
      setDateLocale(bad);
      expect(longDate(at)).toBe(new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(at));
    }
  });
});
