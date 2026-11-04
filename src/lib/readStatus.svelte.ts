// Read status: marked by hand, stored in the vault. Lists loaded before a note was
// marked show the new status through `readStatusOf`, so marking needs no reload.
// Module state; lock reloads the page, which clears it.
import { SvelteMap } from "svelte/reactivity";
import { api, type ReadFilter, type ReadStatus } from "$lib/api";

export const READ_STATUSES: { value: ReadStatus; label: string }[] = [
  { value: "unread", label: "Unread" },
  { value: "partlyRead", label: "Partly read" },
  { value: "read", label: "Read" },
];

export const READ_FILTERS: { value: ReadFilter; label: string }[] = [
  ...READ_STATUSES,
  { value: "notFinished", label: "Not finished" },
];

/** The marker a list shows for `status`; none for unread, which nearly every note is. */
export function readMark(status: ReadStatus): { glyph: string; label: string } | null {
  if (status === "partlyRead") return { glyph: "◐", label: "Partly read" };
  if (status === "read") return { glyph: "✓", label: "Read" };
  return null;
}

// Statuses set in this session, by note id.
const marked = new SvelteMap<number, ReadStatus>();

/** The note's status, including any set since it was loaded. */
export function readStatusOf(note: { id: number; readStatus: ReadStatus }): ReadStatus {
  return marked.get(note.id) ?? note.readStatus;
}

/** Saves the status of `ids`; resolves to the ids whose status changed. */
export async function markRead(ids: number[], status: ReadStatus): Promise<number[]> {
  const changed = await api.setReadStatus(ids, status);
  for (const id of ids) marked.set(id, status);
  return changed;
}
