// State of the explorer tree: which folders are expanded and the notes loaded per
// folder. Module state, so it survives the sidebar being hidden and switching between
// Tree and Recent; lock reloads the page, which clears it.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, type NoteSummary } from "$lib/api";

export const PAGE_SIZE = 500;

export type FolderNotes = { notes: NoteSummary[]; hasMore: boolean };

/** Folder paths are case-insensitive, so state is keyed by the lowercased path. */
export const key = (folder: string) => folder.toLowerCase();

/** Expanded folders, by key. The root ("") is always shown. */
export const expanded = new SvelteSet<string>();
/** Loaded notes per folder, by key. A folder is loaded on its first expand. */
export const loaded = new SvelteMap<string, FolderNotes>();

// Bumped per folder on every load, so a slow, stale answer is dropped.
const generations = new Map<string, number>();

/** Loads the first page of a folder's notes, or at least as many as are shown now. */
export async function loadFolder(folder: string): Promise<void> {
  const k = key(folder);
  const run = (generations.get(k) ?? 0) + 1;
  generations.set(k, run);
  const limit = Math.max(PAGE_SIZE, loaded.get(k)?.notes.length ?? 0);
  const notes = await api.folderNotes(folder, 0, limit);
  if (generations.get(k) !== run) return;
  loaded.set(k, { notes, hasMore: notes.length === limit });
}

/** Loads the next page of a folder that has more. */
export async function loadMore(folder: string): Promise<void> {
  const k = key(folder);
  const current = loaded.get(k);
  if (!current?.hasMore) return;
  const run = (generations.get(k) ?? 0) + 1;
  generations.set(k, run);
  const page = await api.folderNotes(folder, current.notes.length, PAGE_SIZE);
  if (generations.get(k) !== run) return;
  const seen = new Set(current.notes.map((n) => n.id));
  loaded.set(k, { notes: [...current.notes, ...page.filter((n) => !seen.has(n.id))], hasMore: page.length === PAGE_SIZE });
}

/**
 * Reloads the given folders' notes after a save, rename/move, create or delete changed
 * them. Folders that were never loaded are skipped; they load on expand. A move passes
 * both the old and the new folder.
 */
export async function refreshFolders(folders: string[]): Promise<void> {
  const unique = [...new Set(folders.map(key))];
  await Promise.all(
    unique.filter((k) => loaded.has(k)).map((k) => loadFolder(folders.find((f) => key(f) === k) ?? k)),
  );
}

/** A loaded note's summary, if any folder holding it is loaded. */
export function findNote(id: number): NoteSummary | undefined {
  for (const folder of loaded.values()) {
    const note = folder.notes.find((n) => n.id === id);
    if (note) return note;
  }
  return undefined;
}

/** Forgets every loaded folder (after an import, which can touch any of them). */
export function clearLoaded() {
  loaded.clear();
  generations.clear();
}

/** Updates a note's summary in place, e.g. after an autosave. */
export function updateNote(summary: NoteSummary) {
  const k = key(summary.folder);
  const current = loaded.get(k);
  if (!current) return;
  const i = current.notes.findIndex((n) => n.id === summary.id);
  if (i < 0) return;
  const notes = current.notes.slice();
  notes[i] = summary;
  loaded.set(k, { ...current, notes });
}

/**
 * `path` with its `from` prefix swapped for `to`, if it is `from` or below it; else null.
 * Case-insensitive, like folder paths.
 */
export function rebase(path: string, from: string, to: string): string | null {
  if (key(path) === key(from)) return to;
  return key(path).startsWith(`${key(from)}/`) ? to + path.slice(from.length) : null;
}

/**
 * After folder `from` moved to `to`: the folder and its subfolders stay expanded at
 * their new paths, and every loaded or expanded folder there is reloaded (`to` may have
 * existed already and been merged into). The folder it was dropped in is expanded, so
 * it shows.
 */
export async function folderMoved(from: string, to: string): Promise<void> {
  // All removed before any is added: `b/b` moved to the top level becomes `b`, and
  // `b/b/b` becomes `b/b`.
  const moved = [...expanded].flatMap((k) => {
    const path = rebase(k, from, to);
    return path === null ? [] : [{ k, path }];
  });
  for (const { k } of moved) expanded.delete(k);
  for (const { path } of moved) expanded.add(key(path));
  for (const k of [...loaded.keys()]) if (rebase(k, from, to) !== null) loaded.delete(k);
  const stale = new Set([...expanded, ...loaded.keys()].filter((k) => rebase(k, to, to) !== null));
  await Promise.all([...stale].map(loadFolder));
  await expandTo(to.split("/").slice(0, -1).join("/"));
}

/** Expands every ancestor of `folder` and the folder itself, loading them as needed. */
export async function expandTo(folder: string): Promise<void> {
  const parts = folder.split("/").filter(Boolean);
  const paths = parts.map((_, i) => parts.slice(0, i + 1).join("/"));
  for (const p of paths) expanded.add(key(p));
  await Promise.all(["", ...paths].filter((p) => !loaded.has(key(p))).map(loadFolder));
}

/**
 * Makes sure an opened note shows in its folder's list, even if it lies beyond the
 * pages loaded so far.
 */
export function ensureNote(summary: NoteSummary) {
  const k = key(summary.folder);
  const current = loaded.get(k);
  if (!current || current.notes.some((n) => n.id === summary.id)) return;
  loaded.set(k, { ...current, notes: [...current.notes, summary] });
}
