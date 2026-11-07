// Human-readable sizes for the folder tree. Decimal units (1 kB = 1000 B), as file
// managers on macOS and GNOME show them. Always a "." decimal point: the column is
// short and lines up the same in every locale.

const UNITS = ["kB", "MB", "GB"] as const;

/**
 * `512 B`, `4.2 kB`, `42 kB`, `1.0 MB`. Under 10 of a unit one decimal, from 10 whole
 * numbers, rounded half up; a value that would round to 1000 of a unit shows in the
 * next one (`999_500` is `1.0 MB`, not `1000 kB`). GB is the largest unit.
 */
export function formatSize(bytes: number): string {
  const n = Number.isFinite(bytes) && bytes > 0 ? Math.floor(bytes) : 0;
  if (n < 1000) return `${n} B`;
  for (let i = 0; i < UNITS.length; i++) {
    const unit = 1000 ** (i + 1);
    // Tenths of the unit, from integers so `1_150` is exactly 11.5 tenths.
    const tenths = Math.round(n / (unit / 10));
    if (tenths < 100) return `${Math.floor(tenths / 10)}.${tenths % 10} ${UNITS[i]}`;
    const whole = Math.round(n / unit);
    if (whole < 1000 || i === UNITS.length - 1) return `${whole} ${UNITS[i]}`;
  }
  throw new Error("unreachable");
}
