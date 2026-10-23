const time = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });
const day = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const full = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** The time for today, the date otherwise. */
export function shortDate(ms: number): string {
  const d = new Date(ms);
  return d.toDateString() === new Date().toDateString() ? time.format(d) : day.format(d);
}

export function longDate(ms: number): string {
  return full.format(new Date(ms));
}
