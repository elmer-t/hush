# Project: Hush (working title) – MVP

## Goal
Build a cross-platform desktop app for private notes, journals and thoughts, with basic Obsidian-style features. Everything is stored in a single encrypted SQLite database.

## Stack
- Tauri 2 (Rust backend, TypeScript frontend)
- Frontend: Vite + TypeScript, plus a light UI framework of your choice (Svelte or React)
- Editor: CodeMirror 6 with Markdown support
- Database: rusqlite with the `bundled-sqlcipher` feature (SQLCipher, AES-256)
- Targets: Windows, macOS, Linux (no mobile for the MVP)

## MVP features
1. **Unlock screen:** create or open a vault with a passphrase. Use the SQLCipher defaults (PBKDF2). Never store the passphrase or key on disk.
2. **Notes:** create, edit, delete and rename. Markdown editor with live preview or split view.
3. **Organisation:** tags/labels, topics, author and created/updated dates, each indexed.
4. **Links:** `[[wikilinks]]` between notes, plus a backlinks panel.
5. **Search:** FTS5 full-text search over titles and bodies, with filters for tag, topic and date.
6. **Auto-lock:** lock after N minutes idle, and on demand.
7. **Import:** import an existing folder of Markdown files (an Obsidian vault) into the database.
	-. **Clean:** many files contain html and line endings at roughly 80 characters which need to be cleaned during import, resulting in clean markdown.
	-. **Index:** files may contain indexable attributes; author names, author email, label tags (jr=Journal, dr=Dream, etc.) which need to be stored in a searchable way. THIS NEEDS FURTHER RESEARCH

## Data model (starting point)
- `notes` (id, title, body, author, topic, created_at, updated_at)
- `tags` (id, name), `note_tags` (note_id, tag_id)
- `links` (from_note_id, to_note_id)
- `notes_fts` (FTS5 over title and body)
- Turn on WAL mode and add indexes on topic, author, updated_at and note_tags.

## Constraints
- All note content lives only inside the encrypted database. No plaintext temp files, caches or logs containing note content.
- The Rust side owns all database access. The frontend talks to it only through Tauri commands.
- Keep attachments out of scope for the MVP.
- Free and open-source dependencies only.

## Deliverables
- A working app that builds on macOS and Windows
- A README with build steps
- Basic tests for the database layer (create, search, link, lock/unlock)