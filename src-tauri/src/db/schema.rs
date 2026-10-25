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
"#, r#"
-- Trash: a trashed note keeps its row (and FTS row) but has no tag, attribute or link
-- rows, and every query skips it. (folder, title) is only unique among live notes.
ALTER TABLE notes ADD COLUMN deleted_at INTEGER;
DROP INDEX notes_folder_title;
CREATE UNIQUE INDEX notes_folder_title ON notes(folder, title) WHERE deleted_at IS NULL;
CREATE INDEX notes_deleted ON notes(deleted_at) WHERE deleted_at IS NOT NULL;
"#, r#"
-- Tags become case-sensitive: `jr` and `Jr` are different tags. The collation can't be
-- altered in place, so both tables are rebuilt; the child goes first so that dropping
-- `tags` doesn't cascade.
CREATE TABLE tags_new (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
INSERT INTO tags_new(id, name) SELECT id, name FROM tags;
CREATE TABLE note_tags_new (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags_new(id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, tag_id)
) WITHOUT ROWID;
INSERT INTO note_tags_new(note_id, tag_id) SELECT note_id, tag_id FROM note_tags;
DROP TABLE note_tags;
DROP TABLE tags;
ALTER TABLE tags_new RENAME TO tags;
ALTER TABLE note_tags_new RENAME TO note_tags;
CREATE INDEX note_tags_tag ON note_tags(tag_id, note_id);
"#, r#"
-- Embeddings: one vector per note and model, so two models' vectors can coexist while
-- the active model changes. `vector` is little-endian f32s; `content_hash` is the
-- SHA-256 of the embedded text, so a save that doesn't change it doesn't re-embed.
CREATE TABLE note_embeddings (
    note_id      INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    model        TEXT NOT NULL,
    content_hash BLOB NOT NULL,
    vector       BLOB NOT NULL,
    PRIMARY KEY (note_id, model)
);
CREATE INDEX note_embeddings_model ON note_embeddings(model);
"#, r#"
-- Zero-shot tag suggestions. A description may name a tag no note uses yet; `tag` is
-- case-sensitive like `tags.name`. `vector` is the description embedded with `model`,
-- NULL until the job gets to it.
CREATE TABLE tag_descriptions (
    tag         TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    vector      BLOB,
    model       TEXT
);
-- The user's decisions on suggested tags; the latest one per note and tag wins. User
-- input, not derived: kept while the note is in the trash, deleted with the note.
CREATE TABLE tag_feedback (
    note_id    INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag        TEXT NOT NULL,
    accepted   INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (note_id, tag)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault from before the trash (version 2) gets `deleted_at` and the partial index.
    #[test]
    fn trash_migration_keeps_existing_notes() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..2] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute("INSERT INTO notes(folder, title, body, created_at, updated_at) VALUES ('a', 'T', 'x', 1, 1)", [])
            .unwrap();

        migrate(&mut conn).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let deleted: Option<i64> = conn.query_row("SELECT deleted_at FROM notes", [], |r| r.get(0)).unwrap();
        assert_eq!(deleted, None);

        let insert = "INSERT INTO notes(folder, title, body, created_at, updated_at) VALUES ('a', 't', '', 2, 2)";
        assert!(conn.execute(insert, []).is_err(), "still unique among live notes");
        conn.execute("UPDATE notes SET deleted_at = 5", []).unwrap();
        conn.execute(insert, []).unwrap();
    }

    /// A vault from before case-sensitive tags (version 3) keeps its tags and gains `Jr` next to `jr`.
    #[test]
    fn tag_case_migration_keeps_note_tags() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, created_at, updated_at) VALUES (1, 'T', 1, 1);
             INSERT INTO tags(id, name) VALUES (7, 'jr');
             INSERT INTO note_tags VALUES (1, 7);",
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        let tagged: i64 = conn.query_row("SELECT tag_id FROM note_tags WHERE note_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(tagged, 7);
        conn.execute("INSERT INTO tags(name) VALUES ('Jr')", []).unwrap();
        assert!(conn.execute("INSERT INTO tags(name) VALUES ('jr')", []).is_err());
        conn.execute("DELETE FROM notes", []).unwrap();
        let left: i64 = conn.query_row("SELECT count(*) FROM note_tags", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "note_tags still cascades from notes");
    }

    /// A vault from before embeddings (version 4) keeps its notes, tags and settings and
    /// gains an empty `note_embeddings` table that cascades from notes.
    #[test]
    fn embeddings_migration_keeps_notes_tags_and_settings() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (1, 'T', 'body #jr', 1, 1);
             INSERT INTO tags(id, name) VALUES (7, 'jr');
             INSERT INTO note_tags VALUES (1, 7);
             INSERT INTO settings VALUES ('trash_days', '12');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let body: String = conn.query_row("SELECT body FROM notes WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "body #jr");
        let tag: i64 = conn.query_row("SELECT tag_id FROM note_tags WHERE note_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(tag, 7);
        let days: String = conn.query_row("SELECT value FROM settings WHERE key = 'trash_days'", [], |r| r.get(0)).unwrap();
        assert_eq!(days, "12");
        let rows: i64 = conn.query_row("SELECT count(*) FROM note_embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);

        conn.execute("INSERT INTO note_embeddings VALUES (1, 'm', x'00', x'0000803f')", []).unwrap();
        conn.execute("DELETE FROM notes", []).unwrap();
        let rows: i64 = conn.query_row("SELECT count(*) FROM note_embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "note_embeddings cascades from notes");
    }

    /// A vault from before tag suggestions (version 5) keeps its notes, vectors and
    /// settings and gains the two new tables; feedback cascades from notes.
    #[test]
    fn tag_suggestions_migration_keeps_notes_and_embeddings() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..5] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 5).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (1, 'T', 'body #jr', 1, 1);
             INSERT INTO tags(id, name) VALUES (7, 'jr');
             INSERT INTO note_tags VALUES (1, 7);
             INSERT INTO note_embeddings VALUES (1, 'm', x'00', x'0000803f');
             INSERT INTO settings VALUES ('embedding_model_ready', 'm');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let body: String = conn.query_row("SELECT body FROM notes WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "body #jr");
        let tag: i64 = conn.query_row("SELECT tag_id FROM note_tags WHERE note_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(tag, 7);
        let vectors: i64 = conn.query_row("SELECT count(*) FROM note_embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(vectors, 1);
        let ready: String =
            conn.query_row("SELECT value FROM settings WHERE key = 'embedding_model_ready'", [], |r| r.get(0)).unwrap();
        assert_eq!(ready, "m");

        conn.execute("INSERT INTO tag_descriptions(tag, description) VALUES ('jr', 'journal'), ('Jr', 'junior')", [])
            .unwrap();
        conn.execute("INSERT INTO tag_feedback VALUES (1, 'jr', 0, 1), (1, 'Jr', 1, 1)", []).unwrap();
        conn.execute("DELETE FROM notes", []).unwrap();
        let rows: i64 = conn.query_row("SELECT count(*) FROM tag_feedback", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "tag_feedback cascades from notes");
        let rows: i64 = conn.query_row("SELECT count(*) FROM tag_descriptions", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2, "descriptions are case-sensitive and don't depend on notes");
    }
}
