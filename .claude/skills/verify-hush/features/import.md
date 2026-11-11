# Import a folder

A user points Import at a folder of Markdown, text, HTML and RTF files, previews what will happen (a dry run with counts, warnings and samples), then imports into a folder of the vault. The source folder is never changed.

## Sub-features

- `import-preview` runs a dry run and shows counts without writing anything.
- `import-run` imports and refreshes the tree.
- `import-merge` skips notes whose folder and title already exist when importing into the same target again.
- `import-rules` edits label and attribute rules (**Edit rules**).
- `import-source-untouched` leaves the source folder byte-for-byte unchanged.

## How to get to it (user POV)

- Header **Import** button. Type the source path into **Source folder**, or use **Choose…** (a native dialog, unreachable here).

## Driving it with hush.mjs

Preconditions:

- Baseline. The fixture folder `src-tauri\tests\fixtures\vault` is clean in git (`git status --short src-tauri/tests/fixtures` prints nothing).

- **Open.** Run `node $h click "text=Import"` and `node $h wait "#import-source"`.
- **Source and target.** Run `node $h type "#import-source" "$PWD\src-tauri\tests\fixtures\vault"` (PowerShell expands `$PWD` to the repo root, so the app gets an absolute path). Then `node $h click "#import-target"`, `node $h press Control+A` and `node $h type "#import-target" "fixtures"`.
- **Preview.** Run `node $h click "text=Preview import"` and `node $h wait "text=Dry run" --timeout 30000`. `node $h text ".report .stats"` shows `Will be imported 25`, `Not supported yet 6`, `Folders 9` and `Warnings 10`. `node $h text "[role=tree]"` is unchanged.
- **Import.** Run `node $h click "text=Import 25 notes"` and `node $h wait "text=Import finished" --timeout 60000`. `node $h text "[role=tree]"` now shows the `fixtures` folder.
- **Merge.** Repeat **Source and target** and **Preview**. The stats show `Already in vault 25`.
- **Source untouched.** `git status --short src-tauri/tests/fixtures` still prints nothing.
- **Proof.** Run `node $h shot import-preview` after **Preview**, and `node $h shot import-done` after **Import**.

## Gotchas

- **Target folder in vault** is pre-filled with a suggestion derived from the source (`vault` for the fixtures). Typing appends to it (`vaultfixtures`), so select-all first.
- Counts above are for the fixture folder at the commit this map was written. If fixtures change, update the numbers from the insta snapshots in `src-tauri/tests/snapshots/`, not from a run you have not checked.
- The import runs in the background. While it runs, the Source and Target fields are disabled and a **Cancel** button shows.
