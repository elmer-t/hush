//! Schema migrations, tracked with `PRAGMA user_version`.

use rusqlite::Connection;

use super::{Error, Result, rules};
use crate::import::rules::Rule;

/// Append-only: entry `i` migrates from version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE notes (
    id         INTEGER PRIMARY KEY,
    folder     TEXT NOT NULL DEFAULT '' COLLATE NOCASE,
    title      TEXT NOT NULL COLLATE NOCASE,
    body       TEXT NOT NULL DEFAULT '',
    author     TEXT COLLATE NOCASE,
    topic      TEXT COLLATE NOCASE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX notes_folder_title ON notes(folder, title);
CREATE INDEX notes_title   ON notes(title);
CREATE INDEX notes_topic   ON notes(topic);
CREATE INDEX notes_author  ON notes(author);
CREATE INDEX notes_created ON notes(created_at);
CREATE INDEX notes_updated ON notes(updated_at);

CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
CREATE TABLE note_tags (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX note_tags_tag ON note_tags(tag_id, note_id);

CREATE TABLE note_attributes (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    key     TEXT NOT NULL COLLATE NOCASE,
    value   TEXT NOT NULL COLLATE NOCASE
);
CREATE INDEX note_attributes_kv   ON note_attributes(key, value);
CREATE INDEX note_attributes_note ON note_attributes(note_id);

-- Links are stored by target text and resolved at query time.
CREATE TABLE links (
    from_note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    target_path  TEXT NOT NULL COLLATE NOCASE,
    target_title TEXT NOT NULL COLLATE NOCASE
);
CREATE INDEX links_target ON links(target_title);
CREATE INDEX links_from   ON links(from_note_id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE VIRTUAL TABLE notes_fts USING fts5(
    title, body,
    content = 'notes', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER notes_fts_insert AFTER INSERT ON notes BEGIN
    INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER notes_fts_delete AFTER DELETE ON notes BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TRIGGER notes_fts_update AFTER UPDATE OF title, body ON notes BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
    INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
"#, r#"
-- Label/attribute rules run during import, in `position` order.
CREATE TABLE import_rules (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL,
    name     TEXT NOT NULL,
    enabled  INTEGER NOT NULL,
    source   TEXT NOT NULL,
    lines    INTEGER NOT NULL,
    pattern  TEXT NOT NULL,
    target   TEXT NOT NULL,
    map      TEXT NOT NULL,   -- JSON array of [from, to] pairs
    strip    INTEGER NOT NULL
);
"#];

/// Vaults created before this version get the default import rules.
const IMPORT_RULES_VERSION: usize = 2;

pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let version = version as usize;
    if version > MIGRATIONS.len() {
        return Err(Error::UnsupportedVersion);
    }
    let tx = conn.transaction()?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        tx.execute_batch(sql)?;
        if i + 1 == IMPORT_RULES_VERSION {
            rules::write_rules(&tx, &Rule::defaults())?;
        }
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    tx.commit()?;
    Ok(())
}
