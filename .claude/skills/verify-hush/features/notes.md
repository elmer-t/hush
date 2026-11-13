# Write a note

A user creates a note, names it and writes Markdown in the editor, as source or formatted (Live; `Ctrl E` switches). Edits autosave to the encrypted vault, `#tags` in the body show up in the details panel, and the note is still there after locking and unlocking.

## Sub-features

- `note-create` makes an `Untitled` note in the current folder with its title selected.
- `note-title` renames it from the title field (commit on Enter or blur).
- `note-body` autosaves body edits (`Edited` → `Saving…` → `Saved`).
- `note-tags` lists `#tags` from the body in the details panel.
- `note-folder` moves it by typing a folder in the folder field.
- `note-persist` keeps title and body across lock and unlock.

## How to get to it (user POV)

- Header **New note** button, or `Ctrl N` anywhere in the workspace.
- **New note** button in the empty editor area ("Select a note, or create one.").
- Click a note row in the sidebar tree to open an existing note.

## Driving it with hush.mjs

Preconditions:

- Baseline: the scratch vault is unlocked and empty.

- **Create.** Run `node $h press Control+N` and `node $h wait "input[aria-label=Title]"`. `node $h eval "[document.activeElement.getAttribute('aria-label'), document.activeElement.value]"` prints `["Title","Untitled"]`.
- **Title.** Run `node $h type "input[aria-label=Title]" "Verify note"` (replaces the selected `Untitled`) and `node $h press Enter`. `node $h text "[role=tree]"` contains `Verify note`.
- **Body.** Run `node $h type ".cm-content" "Hello from verify-hush #proof"` and `node $h wait "text=Saved" --timeout 8000`. `node $h text ".cm-content"` shows the text (Source shows the Markdown as typed, one pane; there is no `.rendered` pane since #192).
- **Tags.** `node $h text "aside[aria-label=Details]"` contains `#proof`.
- **Rename existing.** Run `node $h click "input[aria-label=Title]"`, `node $h press Control+A`, `node $h type "input[aria-label=Title]" "Renamed"` and `node $h press Enter`. The tree row `[role=treeitem][title=Renamed]` exists.
- **Persist.** Run `node $h press Control+L`, unlock through the UI, `node $h click "[role=treeitem][title=Renamed]"` and `node $h text ".cm-content"`. It prints `Hello from verify-hush #proof`.
- **Proof.** Run `node $h shot note-saved` after **Body**, and `node $h shot note-after-relock` after **Persist**.

## Gotchas

- `wait "text=Saved"` can match the status from *before* the edit. Read `node $h text ".note .status"` right after typing to see `Edited`, then wait for `Saved`.
- Clicking into the title field drops the app's select-all on a new note, and typing then appends (`UntitledVerify note`). Either type immediately after `Control+N`, or press `Control+A` first.
- Tree rows are `title="<folder>/<title>"` for notes in a folder and `title="<title>"` at the root.
- **Delete forever** in the editor's **More actions** menu asks through a native dialog, which is unreachable. Use the Trash page's **Delete forever** instead ([trash.md](./trash.md)).
