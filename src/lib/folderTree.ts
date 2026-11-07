// Builds the folder hierarchy from the flat folder list, for the explorer tree and the
// Recent view's folder tree. The list only holds folders with notes directly in them;
// their ancestors are added here. Paths are case-insensitive: the first spelling of a
// path wins. Linear in the number of folders, apart from sorting each folder's
// children, so tens of thousands of folders under one parent stay quick.

export type FolderChild = { path: string; name: string };
export type FolderNode = FolderChild & { children: FolderNode[] };

/** Folder paths are case-insensitive, so the hierarchy is keyed by the lowercased path. */
const key = (folder: string) => folder.toLowerCase();

// One collator for every comparison: `localeCompare` sets one up per call, which made
// sorting large folders several times slower.
export const collator = new Intl.Collator();

/** A path without empty segments: no leading, trailing or doubled slash. */
const CLEAN = /^[^/]+(\/[^/]+)*$/;

/** Lowercased folder path ("" is the top level) -> its direct subfolders, sorted by name. */
export function folderChildren(folders: readonly string[]): Map<string, FolderChild[]> {
  const map = new Map<string, FolderChild[]>();
  const seen = new Set<string>();
  // Adds `path` and any ancestors not seen yet, returning its key. A path already seen
  // costs one lookup, so each folder is usually one lowercase and one set check.
  const add = (path: string): string => {
    const k = key(path);
    if (seen.has(k)) return k;
    seen.add(k);
    const slash = path.lastIndexOf("/");
    const parent = slash < 0 ? "" : add(path.slice(0, slash));
    let siblings = map.get(parent);
    if (!siblings) map.set(parent, (siblings = []));
    siblings.push({ path, name: path.slice(slash + 1) });
    return k;
  };
  for (const folder of folders) {
    const path = CLEAN.test(folder) ? folder : folder.split("/").filter(Boolean).join("/");
    if (path) add(path);
  }
  for (const list of map.values()) list.sort((a, b) => collator.compare(a.name, b.name));
  return map;
}

/** The same hierarchy as nested nodes, from the top level down. */
export function folderNodes(folders: readonly string[]): FolderNode[] {
  const children = folderChildren(folders);
  const build = (parent: string): FolderNode[] =>
    (children.get(parent) ?? []).map((c) => ({ ...c, children: build(key(c.path)) }));
  return build("");
}

/** Whether two folder lists are the same, in the same order. */
export function sameFolders(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((f, i) => f === b[i]);
}

/** A folder's own byte total, as `folder_sizes` lists it: notes directly in it only. */
export type FolderSize = { path: string; bytes: number };

/**
 * Lowercased folder path ("" is the vault) -> the bytes of every note in it and below
 * it. Ancestors with no notes of their own get a total too. One pass over the list,
 * each folder adding to its ancestors, so the cost follows folders times depth.
 */
export function subtreeSizes(sizes: readonly FolderSize[]): Map<string, number> {
  const totals = new Map<string, number>([["", 0]]);
  for (const { path: folder, bytes } of sizes) {
    totals.set("", totals.get("")! + bytes);
    const path = CLEAN.test(folder) ? folder : folder.split("/").filter(Boolean).join("/");
    if (!path) continue;
    let k = key(path);
    for (;;) {
      totals.set(k, (totals.get(k) ?? 0) + bytes);
      const slash = k.lastIndexOf("/");
      if (slash < 0) break;
      k = k.slice(0, slash);
    }
  }
  return totals;
}
