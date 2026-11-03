import type { IndexStatus } from "$lib/api";

type Indexing = Extract<IndexStatus, { state: "indexing" }>;

/** The progress while notes are still being indexed (either phase), else `null`. */
export function indexing(i: IndexStatus): Indexing | null {
  return i.state === "indexing" && (i.done < i.total || i.complete < i.total) ? i : null;
}

/** "1,200 of 5,000 notes indexed, 300 of 1,200 complete": every note first gets a vector
 * from its first chunk, then long notes get the rest (see embed/job.rs). */
export function indexText(i: Indexing): string {
  const n = (x: number) => x.toLocaleString();
  const indexed = `${n(i.done)} of ${n(i.total)} notes indexed`;
  return i.complete < i.done ? `${indexed}, ${n(i.complete)} of ${n(i.done)} complete` : indexed;
}
