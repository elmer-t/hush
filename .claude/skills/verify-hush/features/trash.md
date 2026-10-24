# Trash

A user moves a note to the trash, after which it is hidden from the tree and search. They can see it on the Trash page with its original folder and how long until it is removed, then restore it or delete it forever. Trash older than the retention period (Settings, default 30 days) is purged on unlock.

## Sub-features

- `trash-move` moves the open note to the trash from **More actions**.
- `trash-bulk` trashes several selected notes from the sidebar selection bar.
- `trash-list` shows title, original folder, deleted time and days left.
- `trash-restore` brings the note back and opens it.
- `trash-delete` deletes one note forever after an in-page confirm.
- `trash-empty` empties the trash after an in-page confirm.

## How to get to it (user POV)

- Editor **More actions** (`…`) → **Move to trash**.
- Select notes in the tree (Ctrl-click), then the selection bar's trash button.
- **Trash** at the bottom of the sidebar (shows a count when not empty).

## Driving it with hush.mjs

Preconditions:

- Baseline, plus two notes `Keep` and `Bin` created as in [notes.md](./notes.md).

- **Move.** Run `node $h click "[role=treeitem][title=Bin]"`, `node $h click "button[aria-label='More actions']"` and `node $h click "text=Move to trash"`. `node $h text "[role=tree]"` no longer contains `Bin`.
- **List.** Run `node $h click "button.trash"` and `node $h wait "text=Restore"`. `node $h text "main table"` shows `Bin`, `vault root` and `30 days`.
- **Restore.** Run `node $h click "text=Restore"`. The editor opens `Bin`. `node $h text "[role=tree]"` must contain `Bin` (**fails at 046fd3c**, see Gotchas).
- **Delete forever.** Trash `Bin` again, open **Trash**, then run `node $h click "td.actions button.danger"` and confirm with `node $h click "dialog button.danger-fill"`. `node $h text "main"` contains `The trash is empty.`
- **Proof.** Run `node $h shot trash-list` after **List**, and `node $h shot trash-restored` after **Restore**.

## Gotchas

- `text=Trash` misses once the sidebar button shows a count (`Trash1`). Use `button.trash`.
- Found at commit 046fd3c: after **Restore**, the restored note is missing from the tree until something refreshes the whole sidebar (e.g. an import). `restored()` in `src/lib/Workspace.svelte` calls `sidebar.refresh()` without the note's folder. Keep the journey failing until that is fixed. Don't weaken the assertion.
- The Trash page's confirms are in-page `<dialog>`s and drivable. The editor's **Delete forever** uses a native dialog and is not.
- Purge-on-unlock needs a note trashed more than the retention period ago, which the UI cannot fake. Rust tests cover it (`session::tests::unlock_purges_expired_trash`). Report it as not driven.
