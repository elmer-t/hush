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
- Auto-lock after N minutes idle (default 10, 1–1440, stored inside the vault) and on demand (Lock button, Ctrl+L). Enforced by a backend timer; the frontend only reports activity. Idle time uses both the monotonic and wall clock so a machine waking from sleep locks immediately. OS sleep / screen-lock hooks deferred to v2.
- On lock: close the connection, zeroize the key in Rust, wipe frontend state.
- Preview: Markdown rendered with raw HTML **disabled**, output sanitised with DOMPurify. Strict CSP blocking all remote loads (images, scripts, fonts). External links open in the system browser, on click only.
- Delete is a hard delete with confirmation; `PRAGMA secure_delete=ON`.
- No export in the MVP.
- No note content in logs; devtools disabled in release builds.

## Notes
- Identity: **`(folder, title)` is unique**, not the title alone. `folder` is the full path (e.g. `archive/2019/march`), indexed.
- Title is a separate field (from the filename on import). Renaming happens in the title field.
- Autosave 500 ms after typing stops; no save button.
- Editor: split view (source | preview) with a preview-only toggle. Live preview deferred to v2.
- **Frontmatter in the body is the single source of truth.** Author, topic, tags and attributes are derived from it on every save. The metadata panel edits the frontmatter.
- Tags = inline `#tags` ∪ frontmatter `tags`.
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
- `notes` (id, folder, title, body, author, topic, created_at, updated_at), unique `(folder, title)`
- `tags` (id, name), `note_tags` (note_id, tag_id)
- `note_attributes` (note_id, key, value), indexed on `(key, value)`
- `links` (from_note_id, target_text)
- `notes_fts` (FTS5 over title and body)
- `settings` (key, value) — auto-lock minutes, default author, …
- `import_rules` (see Import)
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

## UI
Three panes: left = search, filters, note list / folder tree (sorted by updated); centre = title + editor; right = metadata panel, backlinks, outgoing links. Side panes collapsible.

## Tests
- Rust integration tests on encrypted temp DBs: create, rename + link rewrite, search with filters, backlinks, lock/unlock, wrong passphrase, rekey.
- Importer built test-first with `insta` snapshot tests against the fixture vault.
- Frontend: `svelte-check` / type checking only.

## Build order
1. Scaffold  2. DB layer + tests  3. Unlock/lock  4. Notes + editor  5. Search  6. Import (test-first), rules editor last. Nice to have: links/backlinks
