This vault is one encrypted file. A few things to know before you fill it.

## Passphrase

There is no recovery. Without the passphrase the notes are gone. Keep your own backup of the `.hush` file.

## Choose the model before a large import

Hush indexes every note with a local model. Search by meaning, related notes and tag suggestions depend on it. Switching model later indexes every note again: at roughly 2.5 notes a second, 10,000 notes take over an hour.

Choose in Settings → Model first:

- **BGE small English v1.5 (int8)**, the default: notes in English.
- **Multilingual E5 small (int8)**: notes in another language, or in several.
- **MiniLM L6 v2, fast English (int8)**: English, about twice as fast. For a very large vault.
- **BGE small English v1.5 (fp16, GPU)**: English, on a machine with a usable GPU.

All but the default are a one-time download.

## Importing

Import takes a folder of Markdown files. Run *Preview import* first: it shows what would happen without changing the vault.

## Writing

- `[[Note title]]` links to a note. `#tag` tags it.
- Ctrl+N new note, Ctrl+K search, Ctrl+L lock.
- Deleted notes go to the trash (Settings → Trash).

Delete this note when you no longer need it.
