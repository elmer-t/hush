// Builds the folder hierarchy from the flat folder list, for the explorer tree and the
// Recent view's folder tree. The list only holds folders with notes directly in them;
// their ancestors are added here. Paths are case-insensitive: the first spelling of a
// path wins. Linear in the number of path segments, apart from sorting each folder's
// children, so tens of thousands of folders under one parent stay quick.

export type FolderChild = { path: string; name: string };
export type FolderNode = FolderChild & { children: FolderNode[] };

/** Folder paths are case-insensitive, so the hierarchy is keyed by the lowercased path. */
const key = (folder: string) => folder.toLowerCase();

// One collator for every comparison: `localeCompare` sets one up per call, which made
// sorting large folders several times slower.
export const collator = new Intl.Collator();

/** Lowercased folder path ("" is the top level) -> its direct subfolders, sorted by name. */
export function folderChildren(folders: readonly string[]): Map<string, FolderChild[]> {
  const map = new Map<string, FolderChild[]>();
  const seen = new Set<string>();
  for (const folder of folders) {
    let path = "";
    let parent = "";
    for (const name of folder.split("/")) {
      if (!name) continue;
      path = path ? `${path}/${name}` : name;
      const k = key(path);
      if (!seen.has(k)) {
        seen.add(k);
        let siblings = map.get(parent);
        if (!siblings) map.set(parent, (siblings = []));
        siblings.push({ path, name });
      }
      parent = k;
    }
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
