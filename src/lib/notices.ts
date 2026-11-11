/** One row of THIRD_PARTY_NOTICES.md's Summary table: how many packages use a licence. */
export type LicenceFamily = { licence: string; rust: number; npm: number };

/** The licence families in the notices' Summary table (written by scripts/notices/generate.mjs). */
export function licenceSummary(notices: string): LicenceFamily[] {
  const lines = notices.split(/\r?\n/);
  const start = lines.indexOf("## Summary");
  if (start < 0) return [];
  const rows: LicenceFamily[] = [];
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith("## ")) break;
    const cells = line.match(/^\|\s*([^|]+?)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|$/);
    if (cells) rows.push({ licence: cells[1], rust: Number(cells[2]), npm: Number(cells[3]) });
  }
  return rows;
}
