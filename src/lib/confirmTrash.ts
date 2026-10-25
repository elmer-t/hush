import { ask } from "@tauri-apps/plugin-dialog";
import { api, type FolderContents } from "$lib/api";

/**
 * Asks before moving notes to the trash, unless that is turned off in Settings.
 * `title` names a single note; without it the dialog counts. Resolves to whether to go ahead.
 */
export async function confirmTrash(count: number, title?: string): Promise<boolean> {
  if (count === 0) return false;
  if (!(await api.confirmTrash())) return true;
  const what = count === 1 && title ? `“${title}”` : count === 1 ? "this note" : `${count.toLocaleString()} notes`;
  return ask(`Move ${what} to the trash? You can restore from the trash.`, {
    title: "Move to trash",
    kind: "warning",
    okLabel: "Move to trash",
    cancelLabel: "Cancel",
  });
}

const plural = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/**
 * Asks before moving a folder and everything in it to the trash. Always asks, even when
 * "Confirm before deleting" is off: one click can trash a large part of the vault.
 */
export async function confirmFolderTrash(folder: string, contents: FolderContents): Promise<boolean> {
  const notes = plural(contents.notes, "note", "notes");
  const where = contents.subfolders > 0 ? ` in ${plural(contents.subfolders, "subfolder", "subfolders")}` : "";
  return ask(
    `Move “${folder}” and everything in it to the trash? ${notes}${where}. You can restore it from the trash.`,
    { title: "Move folder to trash", kind: "warning", okLabel: "Move to trash", cancelLabel: "Cancel" },
  );
}
