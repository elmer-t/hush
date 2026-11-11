import { describe, expect, it } from "vitest";
import { licenceSummary } from "$lib/notices";

const NOTICES = `# Third-party notices

Intro.

## Summary

| Licence | Rust crates | npm packages |
|---|---:|---:|
| MIT | 391 | 59 |
| MPL-2.0 | 5 | 0 |
| OFL-1.1 | 0 | 3 |

Native libraries are listed in the next section.

## Native libraries, fonts and models

| Not | a | summary |
|---|---|---|
| X | 1 | 2 |
`;

describe("licenceSummary", () => {
  it("reads the licence families from the Summary table", () => {
    expect(licenceSummary(NOTICES)).toEqual([
      { licence: "MIT", rust: 391, npm: 59 },
      { licence: "MPL-2.0", rust: 5, npm: 0 },
      { licence: "OFL-1.1", rust: 0, npm: 3 },
    ]);
  });

  it("is empty when the file has no Summary table", () => {
    expect(licenceSummary("# Third-party notices\n\nNothing here.\n")).toEqual([]);
  });

  it("handles Windows line endings", () => {
    expect(licenceSummary(NOTICES.replace(/\n/g, "\r\n"))[0]).toEqual({ licence: "MIT", rust: 391, npm: 59 });
  });
});
