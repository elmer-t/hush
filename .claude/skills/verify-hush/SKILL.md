---
name: verify-hush
description: Drive the real Hush desktop app (Tauri 2 + WebView2 on Windows) the way a user does, to prove a change works. Launches an isolated instance with its own scratch vault, clicks and types through the UI over CDP, and captures screenshots plus a step log. Use after changing anything in src/ or src-tauri/src/ that a user can see, before claiming a UI, vault, note, search, import or trash change works.
---

# Verify Hush

Hush is a Windows desktop app: a Rust/Tauri backend and a SvelteKit UI rendered by WebView2. You drive it with `scripts/hush.mjs`, a dependency-free Node 24 script that talks to WebView2's remote-debugging port (CDP). It sends real mouse clicks and key events, so you exercise the same path as a user.

All commands below run from the repo root in **PowerShell** (not Git Bash, whose `perl` breaks the OpenSSL build). `$h` is shorthand:

```powershell
$h = ".claude/skills/verify-hush/scripts/hush.mjs"
node $h help
```

Every command takes `--run NAME` (default `default`). Each run is fully separate: its own debug port, WebView2 profile, scratch vault and evidence folder under `%TEMP%\hush-verify\<run>\`. Two runs can drive two windows side by side, and they never touch the user's own Hush window or vaults.

## Launch

1. **Build** whenever `src/`, `src-tauri/src/`, `tauri.conf.json`, `Cargo.toml` or `package.json` changed since the last build:

   ```powershell
   node $h build
   ```

   It runs `npm run tauri build -- --debug --no-bundle` into `src-tauri/target/verify/` (a separate target dir, because a running `tauri dev` locks `target/debug/hush.exe`). The first build compiles OpenSSL and takes several minutes; later builds are incremental. The frontend is embedded, so no Vite server is needed. `build` refuses to run when Git's `perl` comes first on `PATH`.

2. **Start** the instance:

   ```powershell
   node $h launch
   ```

   It prints `{"ready":true,"pid":…,"port":…,"screen":"unlock",…}` once the unlock card is rendered and usable. With no vault in the run yet, the unlock card may preselect the user's own most recent vault. **Never type into it.** Seed first.

3. **Seed** the scratch vault:

   ```powershell
   node $h seed
   ```

   This creates `%TEMP%\hush-verify\<run>\instance\verify.hush` with passphrase `verify-hush passphrase` (or `--passphrase P`), locks it and reloads, so the unlock screen preselects it. Seeding calls the `create_vault` command directly, because the UI's **Create new…** opens a native Save dialog that CDP cannot reach. Seeding is setup, not proof. Unlocking through the UI is the user path:

   ```powershell
   node $h type "#passphrase" "verify-hush passphrase"
   node $h click "text=Unlock"
   node $h wait "button[aria-label=Settings]"
   ```

   The vault starts empty. For realistic content, import `src-tauri/tests/fixtures/vault` through the Import screen (see [features/import.md](features/import.md)).

## Doctor

Run this first whenever anything looks off, and before trusting a result:

```powershell
node $h doctor
```

It is read-only and exits non-zero on any `FAIL`. It checks that the process is alive, the debug port is this run's, the page is the embedded build (`tauri.localhost`, not `localhost:1420`), the UI is usable, **the only vault selected or open is this run's scratch vault**, the WebView2 profile is the isolated one, and `hush.exe` is newer than every source file. A stale build means you are testing old code: `stop`, `build`, `launch`.

## Drive

`<target>` is a CSS selector or `text=Label` (the visible element whose whole text is `Label`; when a button or other control contains it, the control is used). `click` and `type` wait up to 10 s for the element to be visible, enabled and not covered, then click its centre with real mouse events.

```powershell
node $h click "text=New note"                       # or: node $h press Control+N
node $h type "input[aria-label=Title]" " - draft"   # click, then insert text at the caret
node $h type ".cm-content" "Body text #tag"         # CodeMirror body editor
node $h press Enter                                  # also Escape, Tab, Control+L, Control+K, Shift+ArrowDown…
node $h wait "text=Saved" --timeout 5000
node $h text ".sidebar"                              # print innerText, to assert on
node $h eval "document.querySelector('.status').textContent"   # read-only checks only
```

Stable handles in this app, by screen:

- **Unlock:** `#passphrase`, `text=Unlock`, recent vaults are `[role=radio]` whose `title` is the full path, the wrong-passphrase message is `[role=alert]`.
- **Header:** `text=New note`, `text=Import`, `button[aria-label=Settings]`, `text=Lock`, `button[aria-label="Hide note list"]`.
- **Sidebar:** `input[aria-label="Search notes"]`, `button[aria-label="Clear search"]`, `text=Tree`, `text=Recent`, `text=Filters`, the tree is `[role=tree]` with note rows `[role=treeitem][title="<folder>/<title>"]` (root notes: `title="<title>"`), and the trash entry `button.trash` (`text=Trash` stops matching once it shows a count).
- **Editor:** `input[aria-label=Title]`, folder `input[placeholder="No folder"]`, body `.cm-content`, save state `.note .status` (`Edited` → `Saving…` → `Saved`, or `Not saved`), `button[aria-label="More actions"]` then `text=Move to trash` / `text=Delete forever`, `text=Preview` / `text=Split`.
- **Details panel:** `aside[aria-label=Details]`, `input[aria-label="Add tag"]`.

Don't drive the app with `eval` that changes state, or with `window.__TAURI_INTERNALS__.invoke`. That skips the UI you are trying to prove. `eval` is for reading.

Native dialogs (**Open existing…**, **Create new…**, the **Choose…** buttons) and native `ask` confirmations (e.g. NoteEditor's *Delete forever*) open outside the webview and cannot be driven. Where a text field exists next to **Choose…** (Import source), type the path instead. Otherwise report the path as unreachable, not as verified.

## Evidence

Everything lands in `%TEMP%\hush-verify\<run>\evidence\`:

- `steps.log`: every `launch`/`click`/`type`/`press`/`wait`/`shot`/`stop`, timestamped. Passphrases are masked.
- `<name>.png` from `node $h shot <name>`: a screenshot of the window.
- `node $h note "<text>"` appends your own assertion or observation to the log.

Proof standards:

- Drive the real user path: clicks and keys, not `invoke` or state-changing `eval`.
- Capture the action and the result: a `shot` before and after the step that matters, plus a `text` read of what changed.
- Prove persistence across a second view. Hush autosaves, so after an edit wait for `Saved`, then **Lock** (`press Control+L`), unlock again through the UI, reopen the note and read it back. A `Saved` badge alone is not proof the data reached the encrypted vault.
- Verify side effects: for import, check the source folder is unchanged. For trash, check the note is gone from the tree *and* listed in Trash.
- Mocks have no place here. Everything is local, so there is nothing to stub.

Report `evidence\` paths with the result.

## Cleanup

```powershell
node $h stop
```

It removes the scratch vault from Hush's recent-vault list, kills **only the process tree this run started** (by pid, never by name, so the user's own Hush survives) and deletes `instance\` (vault, WebView2 profile, app log). `evidence\` is kept. Run `stop` after every attempt, including failed ones. To discard old evidence, delete `%TEMP%\hush-verify\<run>\` yourself.

The recent-vault list lives in the user's real `%APPDATA%\com.redheadit.hush\config.json` and can't be redirected per instance. That is why `stop` must run even after a crash: when the app is already gone, it edits that file directly to drop any `hush-verify` paths.

## Features

[features/README.md](features/README.md) indexes one recipe per user-facing feature. When you change a feature, drive every entry point its file lists, not just the most convenient one.

## Gotchas

- Auto-lock defaults to 10 idle minutes. A run that pauses longer comes back to the unlock screen with "locked after a period of inactivity". Unlock again, or raise it in Settings first.
- Locking (`Control+L`, idle, or `lock_vault`) reloads the page, which drops the CDP session. Each helper command reconnects on its own, so just run the next command.
- Closing the window with a vault open waits up to 3 s for pending saves. `stop` kills instead, so wait for `Saved` before stopping if the save matters.
- Shortcuts (`Control+N/L/K`) are caught at window level and work even while a text field has focus.
- The app reads the shared recent list fresh on every unlock screen. The user's own vaults show up in the list too. Leave them alone.
