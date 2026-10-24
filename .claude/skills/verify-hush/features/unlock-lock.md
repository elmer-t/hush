# Unlock and lock a vault

A user picks a vault from the recent list, types its passphrase and lands in the workspace. A wrong passphrase is refused with a message. Locking (button, `Ctrl L`, or idle timeout) closes the vault and returns to the unlock screen with the same vault preselected.

## Sub-features

- `unlock-recent` unlocks the preselected recent vault with the right passphrase.
- `unlock-wrong` refuses a wrong passphrase, clears the field and says which vault.
- `lock-button` locks from the header **Lock** button.
- `lock-key` locks with `Ctrl L`, even while typing in a note.
- `lock-idle` locks after the auto-lock period and says so on the unlock screen.

## How to get to it (user POV)

- Start Hush: the most recent existing vault is preselected on the unlock card.
- Click a vault row in **Recent vaults** to select another.
- Header **Lock** button, or `Ctrl L` anywhere in the workspace.
- Leave the app idle for the auto-lock period (Settings, default 10 minutes).

## Driving it with hush.mjs

Preconditions:

- Launched and seeded, **not yet unlocked** (`node $h doctor` shows `"screen":"unlock"` with the scratch vault selected).

- **Wrong passphrase.** Run `node $h type "#passphrase" "nope"` and `node $h click "text=Unlock"`. `node $h text "[role=alert]"` prints `Wrong passphrase for verify.hush.`, and `node $h eval "document.querySelector('#passphrase').value"` prints `""`.
- **Unlock.** Run `node $h type "#passphrase" "verify-hush passphrase"`, `node $h click "text=Unlock"` and `node $h wait "button[aria-label=Settings]"`. `node $h text ".workspace header .vault"` shows `verify` and the scratch path.
- **Lock with key.** Run `node $h press Control+L` and `node $h wait "#passphrase"`. `node $h doctor` shows `"screen":"unlock"` with the scratch vault selected again, and the page URL ends in `?locked=manual`.
- **Lock with button.** Unlock again, then run `node $h click "text=Lock"` and `node $h wait "#passphrase"`.
- **Idle lock.** Unlock, open **Settings** (`node $h click "button[aria-label=Settings]"`), set auto-lock to `1` minute, then wait about 70 s without input (`node $h wait "#passphrase" --timeout 90000`). `node $h text "main .card"` contains `The vault was locked after a period of inactivity.`
- **Proof.** Run `node $h shot unlock-wrong`, `node $h shot workspace` and `node $h shot locked` at the matching steps.

## Gotchas

- The unlock card can preselect the **user's own** vault when no scratch vault is seeded. Run `doctor` before typing a passphrase.
- Lock reloads the page, so any `eval`-read state from before the lock is gone. Re-read after unlocking.
- Idle time comes from the backend's check every 5 s, so allow up to the period plus 5 s.
- Pointer moves and keys count as activity. Don't run `click` or `type` while waiting for idle lock.
- **Open existing…** and **Create new…** open native file dialogs and are unreachable. Report them as skipped.
