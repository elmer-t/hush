# Hush MVP – Decisions

Outcome of the design grilling session (2026-10-22). Refines [project-brief.md](project-brief.md); where they differ, this document wins.

## Platform & stack
- **Windows only** for the MVP. Keep code free of platform-specific shortcuts so macOS/Linux can follow.
- Tauri 2 + **Svelte 5** + TypeScript + Vite, CodeMirror 6, **npm**.
- rusqlite with **`bundled-sqlcipher-vendored-openssl`** (OpenSSL libcrypto compiled from source and statically linked; needs Strawberry Perl at build time). SQLCipher needs a crypto provider; vendoring avoids system OpenSSL on Windows.
- Bundle identifier `com.redheadit.hush`. No licence file for now.

## Vaults & security
- Multiple vaults (one SQLCipher file each, extension `.hush`). Unlock screen offers "Open existing…" / "Create new…" and remembers recent vault **paths** in an app config file — paths only, never content or key material.
- SQLCipher default KDF (PBKDF2). Passphrase/key never written to disk. **No recovery**: the create screen states this plainly.
- Change passphrase via `PRAGMA rekey`.
- **Multiple instances** (2026-10-24): several Hush processes may run at once, each on a **different** vault (no single-instance plugin; each process has its own connection, key and auto-lock timer). The same vault is never open in two processes: opening takes an exclusive OS lock on `<vault>.hush-lock` next to the vault (not on the vault file itself, which on Windows would block SQLite), before the key is derived. A second open fails with "already open in another Hush window". The lock is released on lock, close and crash. The shared app config is re-read on every use and updated under an OS lock (`config.json.lock`), so concurrent recent-vault writes are not lost.
- Auto-lock after N minutes idle (default 10, 1–1440, stored inside the vault) and on demand (Lock button, Ctrl+L). Enforced by a backend timer; the frontend only reports activity. Idle time uses both the monotonic and wall clock so a machine waking from sleep locks immediately. OS sleep / screen-lock hooks deferred to v2.
- On lock: close the connection, zeroize the key in Rust, wipe frontend state.
- Preview: Markdown rendered with raw HTML **disabled**, output sanitised with DOMPurify. Strict CSP blocking all remote loads (images, scripts, fonts). External links open in the system browser, on click only.
- ~~Delete is a hard delete with confirmation.~~ Replaced by a **trash** (2026-10-24, see Notes). `PRAGMA secure_delete=ON` stays for every real delete. Trade-off accepted: deleted text stays in the (encrypted) vault file until purged, so anyone who later gets the passphrase can read notes deleted within the retention window; that person can already read every live note, and the trash protects against accidental permanent loss.
- No export in the MVP.
- No note content in logs; devtools disabled in release builds.

## Notes
- Identity: **`(folder, title)` is unique**, not the title alone. `folder` is the full path (e.g. `archive/2019/march`), indexed.
- Title is a separate field (from the filename on import). Renaming happens in the title field.
- Autosave 500 ms after typing stops; no save button.
- **Multi-select** (2026-10-24): in both the Tree and Recent views, Ctrl/Cmd+click toggles a note, Shift+click selects a range of visible notes, Ctrl/Cmd+A selects every visible note, Escape clears; a plain click opens the note. Only notes can be selected, never folders. An action bar offers Move to… and Delete. Bulk move is one transaction with one link-preserving pass, all or nothing on any `(folder, title)` clash. Bulk delete moves to the trash, also all or nothing.
- **Bulk metadata edit** (2026-10-24): the multi-select action bar's Edit… adds or removes a tag, or sets the topic or author, on every selected note. Each edit rewrites the note's frontmatter and re-derives it like a save, in one all-or-nothing transaction; notes it wouldn't change are left alone, changed ones get `updated_at` bumped. Remove tag only edits frontmatter, never inline `#tags`, and reports the notes that still have the tag inline. A singular `tag:` key is folded into `tags`. Notes whose frontmatter can't be read are skipped and reported rather than rewritten.
- **Trash** (2026-10-24): Delete moves a note to the trash, with no confirmation since it can be undone. Trashed notes are hidden everywhere: note list, folder tree, search, filters, facets, link resolution, backlinks and outgoing links (links to them show as unresolved). They keep their row but lose their tag, attribute and link rows, which are derived again on restore.
  - *Delete forever* on a note skips the trash (with confirmation). The Trash view lists trashed notes with their original folder and deletion date: Restore, Delete forever, Empty trash (both with confirmation, real deletes).
  - Restore goes back to the original folder; if `(folder, title)` is taken by then, the note becomes `Title (restored)` (then `(restored 2)`, …). It runs the same link-preserving rewrite as create.
  - Auto-purge on unlock: trashed notes older than N days are deleted for good. Default 30, 0 = never, stored in vault `settings` (`trash_days`) and editable in Settings.
- Editor: split view (source | preview) with a preview-only toggle. Live preview deferred to v2.
- **Frontmatter in the body is the single source of truth.** Author, topic, tags and attributes are derived from it on every save. The metadata panel edits the frontmatter.
- Tags = inline `#tags` ∪ frontmatter `tags`.
- **Tags are case-sensitive** (2026-10-24): `jr`, `Jr` and `jR` are different tags, everywhere (dedup, filters, nested-tag matching). Lists sort ignoring case. Folders, titles, author, topic and attributes stay case-insensitive. Import rule maps prefer an exact-case entry and fall back to ignoring case.
- Author: free text, defaults to a vault-level "default author" setting.
- Topic: free field, defaults on import to the top-level folder relative to the import source (not the target folder); rules can override.
- Folders exist implicitly through their notes. Move a note by editing its folder; empty folders can be created from the tree (they are not stored, so one without notes disappears on lock).
- New notes are titled "Untitled", "Untitled 2", … in the selected folder, with the default author in their frontmatter. Shortcuts: Ctrl+N new note, Ctrl+K search, Ctrl+L lock. Folder rename deferred to v2.

## Links
- Stored as `links(from_note_id, target_text)` and resolved at query time.
- Obsidian-style resolution: `[[Title]]` if the title is unique, otherwise the shortest unique path `[[archive/2019/Title]]`.
- `[[Title|alias]]` fully supported. `[[Title#Heading]]` links to the note (no heading jump yet). No embeds.
- Ambiguous link: warning style, click opens a picker. Unresolved link: distinct style, click creates the note.
- A note whose folder *equals* the link path wins outright (so `[[T]]` → root `T`, `[[a/T]]` → `a/T` over `x/a/T`); otherwise every folder *ending in* the path matches.
- Rename, move **and create** preserve meaning: any link that resolved uniquely before the change but not after is rewritten to the shortest unique link of its original target (keeping `#heading` and `|alias`), in the same transaction. Bulk import skips this and reports ambiguities instead.
- Backlinks and outgoing-links panels.

## Data model
- `notes` (id, folder, title, body, author, topic, created_at, updated_at, deleted_at), `(folder, title)` unique among live notes (partial index `WHERE deleted_at IS NULL`); `deleted_at` set = in the trash
- `tags` (id, name), `note_tags` (note_id, tag_id)
- `note_attributes` (note_id, key, value), indexed on `(key, value)`
- `links` (from_note_id, target_text)
- `notes_fts` (FTS5 over title and body)
- `settings` (key, value) — auto-lock minutes, default author, trash retention days, …
- `import_rules` (see Import)
- `note_embeddings` (note_id, model, content_hash, vector), primary key `(note_id, model)` (see AI classification)
- WAL mode. Indexes on folder, topic, author, updated_at, created_at, note_tags.

## Search
- FTS5, bm25 ranking, highlighted snippets, search as you type (~150 ms debounce).
- Filters: tags (multi-select, AND), topic, author, folder, date range (created/updated toggle), attribute key = value.

## Import
- Source: a folder of notes (an Obsidian vault, or the archive: Markdown, text, HTML, RTF, Word, PDF, …). Real data is mostly an online archive with partial, malformed HTML. Real files are not available on the dev machine: develop against a **dummy fixture vault** (~20 files) that mimics them.
- Background task with progress and cancel; tens of thousands of files expected; transactions in batches of 500. Note list is virtualised.
- Dry-run preview (before/after samples, per-file warnings) before anything is written; the Import button is only enabled after a full preview of the same folder. Source folder never modified. Raw text is not kept.
- **Best effort: every readable file is imported**, judged by extension and content (2026-10-24):
  - Markdown and plain text (including unknown extensions, and files without one, that turn out to be text) are cleaned as below.
  - `.html`/`.htm` documents are converted whole; `.rtf` goes through a built-in converter (text, paragraphs, special characters, Unicode). A file with the wrong extension is recognised by its content (`{\rtf`, `<!DOCTYPE html`).
  - Data and code (`.csv`, `.json`, `.xml`, `.log`, `.ini`, scripts, …) are kept verbatim in a fenced code block, since joining "wrapped" lines would corrupt them.
  - Encodings: UTF-8, UTF-16 with a byte order mark (Notepad "Unicode"), otherwise Windows-1252 (a superset of Latin-1), with a warning. Line endings become `\n`.
  - Binary files (known extensions, file signatures, NUL bytes) are not imported yet and are counted per format in the report; **Word, PDF and other formats come later**. Text files over 10 MB are skipped and listed. `<page>_files` folders saved next to a web page are counted as page assets, not imported.
- **Target folder** (2026-10-24): after picking a source folder, an editable target folder is pre-filled with the source folder's name (`D:\Archive\OldVault` → `OldVault`); clearing it imports into the vault root. Note folder = target + the path relative to the source, normalised. If the target already holds notes the UI warns that the import merges into it, and the dry run shows how many notes would be skipped because their `(folder, title)` exists, plus every final folder path. Changing the source or the target requires a new full preview before Import is enabled.
- Hidden files and folders (`.obsidian`, `.trash`) are skipped. Folder = target folder + path relative to the source; title = file name (characters titles can't hold are replaced, with a warning). Files with the same name in one folder (`letter.rtf`, `letter.txt`) are kept apart: later ones get their extension added (`letter (txt)`). A note whose `(folder, title)` already exists in the vault is skipped, so re-importing a folder adds only what is new.
- **HTML cleanup per block**: split on blank lines; only blocks containing tags go through a tolerant HTML5 parser (`html5ever` via `htmd`); unclosed tags are closed at block end; unconvertible markup is dropped and reported. Blocks with block-level tags are converted whole; blocks with inline markup only keep their line structure (paragraphs via a line-break placeholder, lists line by line), so Markdown lists, `[[links]]` and `#tags` mixed with HTML survive. Code blocks are never touched.
- **Unwrap hard wraps** inside paragraphs; leave headings, list items, quotes, code blocks, tables and blank-line breaks alone; never join a line ending in two spaces (or `\`). A line is only joined to one of at least 50 characters, so short lines written on purpose (greetings, poems) stay as they are.
- **Label/attribute rules** (user-configurable, stored inside the vault, run at import only):
  - `source`: filename | frontmatter | first N lines | body
  - `pattern`: Rust `regex` (linear time; no lookarounds/backreferences)
  - `target`: tag | author | author_email | topic | created_at | attribute:<key>
  - `map`: optional value mapping (e.g. `jr → Journal`, `dr → Dream`)
  - `strip`: remove matched text from the body
  - `strip`: on the file-name source it removes the match from the title instead.
  - Single-valued targets take the first match of the first matching rule; tags and attributes collect every match.
  - Test button previews matches against a pasted sample. Frontmatter is read natively, so the defaults are the archive's mail-style headers (`From:` → author and `author_email`, `Date:` → created, `Subject:` → `subject` attribute, all stripped) and one label rule (`jr`/`dr` in the file name → `journal`/`dream` tags, stripped from the title).
  - Rule results are written into the note's frontmatter, as is the default topic when there is none: the top-level folder relative to the import source, ignoring the target prefix. Files directly in the source root get no default topic.
- `created_at` precedence: rule match → frontmatter `created`/`date` → date in filename → file mtime. `updated_at` = file mtime.
- Import report lists warnings and ambiguous links.

## AI classification
**Embeddings** (2026-10-24): every note gets a vector computed inside the Hush process by a local model and stored in the vault. It is the base for related notes, tag suggestions and tag discovery; nothing reads the vectors yet. No note content leaves the machine or the process; English only.
- **Model:** `BAAI/bge-small-en-v1.5` (MIT; 384 dimensions, 512-token context), the int8-quantized ONNX export from `Xenova/bge-small-en-v1.5` (`onnx/model_quantized.onnx`, 34 MB), run with ONNX Runtime through the `ort` crate and tokenized with `tokenizers`. Model ID `bge-small-en-v1.5-q8`.
- **Bundled, not downloaded at runtime.** `npm run fetch-model` (run before `tauri dev`/`tauri build`) fetches the ONNX file and `tokenizer.json` into `src-tauri/models/` (gitignored) and checks them against the SHA-256s pinned in `models/manifest.json`; a test keeps the manifest equal to the model descriptor. They ship as Tauri resources with the model licence. On load Hush reads each file into memory, checks its SHA-256 and loads the checked bytes, so a mismatch (or a missing file) stops indexing with a message in the left pane footer and nothing else changes.
- **No network at runtime:** `cargo tree -e normal --target x86_64-pc-windows-msvc` contains no HTTP client (`reqwest`, `ureq`, `hf-hub`, `hyper`). ONNX Runtime itself is downloaded at build time by `ort` (`download-binaries`, linked statically on Windows); its telemetry is turned off.
- Inference uses at most half the CPU cores (ONNX Runtime intra-op threads), so the UI stays responsive.
- **Ready for other models:** everything model-specific is in one `ModelDescriptor` (`src-tauri/src/embed/mod.rs`): ID, file names, URLs and SHA-256s, vector dimensions, max tokens per chunk, token pooling (CLS for bge), chunk pooling, text prefix, default suggestion threshold. No other code assumes a vector length or chunk size; a vector's length comes from its BLOB.
  - The active model is the vault setting `embedding_model` (default the bge ID). `embedding_model_ready` names the model whose vectors are complete; everything that reads vectors uses it, so after a switch the old vectors keep serving until the new model covers every note. The job then sets `embedding_model_ready` to the new model and deletes the other model's rows.
- **What is embedded:** title + body **without frontmatter** (it holds the tags, which would leak the labels into what later classifiers learn from). A note whose body is empty apart from frontmatter gets no vector. Long notes are split into chunks of at most 512 tokens (special tokens included); the note vector is the L2-normalised mean of the L2-normalised chunk vectors. Only the first 64 chunks (~32k tokens) of a note are embedded, so one huge note can't hold the job for minutes.
- **Storage:** `note_embeddings`, little-endian `f32` BLOBs, keyed by note and model so two models can coexist during a switch; `content_hash` is the SHA-256 of the embedded text, so a save that doesn't change it (or changes only frontmatter) doesn't re-embed. Vectors are as sensitive as the text, so they live only in the vault: no cache file, no temp files (SQLite `temp_store = MEMORY`, the model is loaded from memory). Trash drops a note's row; restore makes it dirty again.
- **Background job:** starts after unlock. It reads a note's text, embeds it and writes the vector in batches of 32, taking the vault lock only to read a batch and to write its results, never while the model runs. It uses a vault accessor that does **not** count as activity, so indexing never postpones auto-lock. It stops on lock (checked between notes) and never touches a vault unlocked later. When idle it checks once a second whether anything changed (SQLite's change counter) and then rescans, so saves, imports, bulk edits, renames and restores are picked up. While 10 or more notes are pending the left pane footer shows `Indexing 1,234 / 20,000`; fewer (a single save) are embedded without it. A loaded model holds no note data and is kept across lock/unlock.
- **Measured speed:** SPEED_PLACEHOLDER

## Extensibility
- **No plugins or third-party code** (2026-10-24). Features that would otherwise be plugins (templates, daily notes, custom CSS, …) are built into Hush and requested as separate issues.
- Why: a plugin runs after unlock and sees every decrypted note, and loading one would need the strict CSP relaxed.
- Revisit a sandboxed plugin API (WASM/WebWorker, no network, per-plugin permissions) only if the list of built-in features keeps growing.

## UI
Three panes: left = search, filters, note list / folder tree (sorted by updated); centre = title + editor; right = metadata panel, backlinks, outgoing links. Side panes collapsible.
- Left pane has a **Tree | Recent** toggle (2026-10-24), stored in vault `settings` (`sidebar_view`, default Tree). *Tree*: one explorer tree of folders and the notes directly in them (subfolders first, then notes, both by name), root notes at the top level, notes loaded per folder on first expand in pages of 500 ("Show more"). Opening a note any way expands and scrolls to it. Expanded folders survive view switches but not lock. Keyboard: Up/Down, Left/Right, Enter, Home/End. *Recent*: the folder tree plus the flat list by updated. While a search or filter is active, both views show the flat result list.

## Tests
- Rust integration tests on encrypted temp DBs: create, rename + link rewrite, search with filters, backlinks, lock/unlock, wrong passphrase, rekey.
- Importer built test-first with `insta` snapshot tests against the fixture vault.
- Frontend: `svelte-check` / type checking only.

## Build order
1. Scaffold  2. DB layer + tests  3. Unlock/lock  4. Notes + editor  5. Search  6. Import (test-first), rules editor last. Nice to have: links/backlinks
