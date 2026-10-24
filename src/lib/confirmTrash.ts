import { ask } from "@tauri-apps/plugin-dialog";
import { api } from "$lib/api";

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
