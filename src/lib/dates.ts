// Every date and time the app shows goes through here, in the operating system's regional
// format (#202). The webview's own default is not that: WebView2 follows the Windows region
// only when its language matches the display language, so English Windows with a Dutch
// region got US dates. The OS locale is read on the Rust side (`os_locale`) and set here
// before anything renders (routes/+layout.ts).

/** `undefined` is the webview's default, used until the OS locale is known or when it is unusable. */
let locale: string | undefined;

type Kind = "time" | "day" | "full" | "clock" | "stamp";

const OPTIONS: Record<Kind, Intl.DateTimeFormatOptions> = {
  time: { timeStyle: "short" },
  day: { dateStyle: "medium" },
  full: { dateStyle: "medium", timeStyle: "short" },
  clock: { hour: "2-digit", minute: "2-digit", second: "2-digit" },
  // What `toLocaleString()` gives: a four-digit year, where `dateStyle: "short"` has two in en-US.
  stamp: { year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit" },
};

let formats: Partial<Record<Kind, Intl.DateTimeFormat>> = {};

const format = (kind: Kind, ms: number) => (formats[kind] ??= new Intl.DateTimeFormat(locale, OPTIONS[kind])).format(ms);

/**
 * Use the operating system's locale, as the OS names it: `nl-NL` on Windows,
 * `nl_NL.UTF-8` on Linux. A name `Intl` has no formats for (`C`, a custom locale)
 * leaves the webview's default in place.
 */
export function setDateLocale(name: string | null | undefined): void {
  locale = supported(name);
  formats = {};
}

function supported(name: string | null | undefined): string | undefined {
  const tag = name?.split(/[.@]/)[0].replaceAll("_", "-");
  if (!tag) return undefined;
  try {
    return Intl.DateTimeFormat.supportedLocalesOf(tag)[0];
  } catch {
    return undefined;
  }
}

/** The time for today, the date otherwise. */
export function shortDate(ms: number, now: number = Date.now()): string {
  const today = new Date(ms).toDateString() === new Date(now).toDateString();
  return format(today ? "time" : "day", ms);
}

/** Date and time to the minute: `20 nov 2026, 20:50`. */
export function longDate(ms: number): string {
  return format("full", ms);
}

/** Time of day with seconds: `20:50:07`. */
export function clockTime(ms: number): string {
  return format("clock", ms);
}

/** Numeric date and time with seconds: `20-11-2026, 20:50:07`. */
export function dateTime(ms: number): string {
  return format("stamp", ms);
}
