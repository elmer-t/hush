# Hush verification map

The maintained source for verifying Hush's user-facing behaviour. Read this index before driving the app, then use the matching feature file as the recipe. Commands use `$h = ".claude/skills/verify-hush/scripts/hush.mjs"` from the repo root in PowerShell (see [../SKILL.md](../SKILL.md)).

## Baseline preconditions

- `node $h build` is current (`node $h doctor` reports `ok build is current`).
- `node $h launch` and `node $h seed` have run for this `--run`, and the scratch vault is unlocked through the UI: `node $h type "#passphrase" "verify-hush passphrase"`, `node $h click "text=Unlock"`, `node $h wait "button[aria-label=Settings]"`.
- `node $h doctor` passes every check, including `only the scratch vault is in use`.
- Never drive an instance that this run did not launch, and never unlock a vault outside `%TEMP%\hush-verify\`.

## Driving conventions

- Start every recipe from the baseline (an empty, unlocked scratch vault) unless its preconditions say otherwise.
- Prefer `aria-label`, `role`, `id` and visible text (`text=…`) handles over classes and position.
- `type` clicks the field first unless it already has focus. To replace a pre-filled value, `click` the field, `press Control+A`, then `type`.
- Wait for an observable state (`wait "text=Saved"`), never a fixed sleep.
- Commands are literal. Keep quoted names and selectors unchanged.

## Proof and skip reporting

- Capture the action and the result: `shot` before and after, plus `text` of the region that changed.
- Mutation proof includes a second view: lock (`Control+L`), unlock through the UI and read the value back, or check the other view that lists it (tree vs. Trash, search vs. tree).
- `node $h note "<feature-id>: <what you saw>"` records each assertion in `evidence\steps.log`.
- Native dialogs (Open existing…, Create new…, Choose…, Delete forever's confirm in the editor) are unreachable. Report them as skipped with the reason. Never report them as verified through another path.

## Feature entry contract

Each feature file starts with an H1 and one paragraph of user-visible behaviour, then exactly four H2s in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with hush.mjs` (starting with `Preconditions:`, then labelled bullets pairing an action, its exact command and the observable result), and `Gotchas`.

## Features

- [Unlock and lock a vault](./unlock-lock.md): unlock from the recent list, wrong passphrase, manual and keyboard lock.
- [Write a note](./notes.md): create, title, body autosave, rename, tags from the body, persistence across lock.
- [Search notes](./search.md): full-text search from the sidebar and `Control+K`, open a hit, clear.
- [Import a folder](./import.md): dry-run preview, import into a target folder, source left untouched.
- [Trash](./trash.md): move to trash, list, restore, delete forever.
