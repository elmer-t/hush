//! Schema migrations, tracked with `PRAGMA user_version`.

use rusqlite::{Connection, params};

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
"#, r#"
-- Per-tag classifier heads, trained on the user's own tags and feedback. Derived data:
-- the job retrains a head whose `inputs` (SHA-256 of what it was trained on) or `model`
-- no longer match. `weights` is little-endian f32s; `precision`/`recall` are
-- cross-validated and NULL when undefined.
CREATE TABLE tag_heads (
    tag        TEXT PRIMARY KEY,
    model      TEXT NOT NULL,
    weights    BLOB NOT NULL,
    bias       REAL NOT NULL,
    n_pos      INTEGER NOT NULL,
    n_neg      INTEGER NOT NULL,
    precision  REAL,
    recall     REAL,
    trained_at INTEGER NOT NULL,
    inputs     BLOB NOT NULL
);
"#, r#"
-- Notes whose vector may be out of date, so the embedding job works on these instead
-- of rescanning every note after every change. Filled by triggers, so every write
-- that can change the embedded text (title, body, trash, restore) queues the note in
-- its own transaction. `version` grows with every queueing: the job only drops a row
-- whose version it read, so a save made while the model ran is never lost.
CREATE TABLE embedding_dirty (
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    version INTEGER NOT NULL DEFAULT 0
);
CREATE TRIGGER embedding_dirty_insert AFTER INSERT ON notes
BEGIN
    INSERT INTO embedding_dirty(note_id) VALUES (NEW.id)
    ON CONFLICT(note_id) DO UPDATE SET version = version + 1;
END;
CREATE TRIGGER embedding_dirty_update AFTER UPDATE OF title, body, deleted_at ON notes
WHEN OLD.title IS NOT NEW.title OR OLD.body IS NOT NEW.body OR OLD.deleted_at IS NOT NEW.deleted_at
BEGIN
    INSERT INTO embedding_dirty(note_id) VALUES (NEW.id)
    ON CONFLICT(note_id) DO UPDATE SET version = version + 1;
END;
-- Notes deleted for good since the job last looked, so it can drop them from the
-- related-notes index (the queue's rows go with the note).
CREATE TABLE embedding_gone (note_id INTEGER PRIMARY KEY);
CREATE TRIGGER embedding_gone_delete AFTER DELETE ON notes
BEGIN
    INSERT OR IGNORE INTO embedding_gone(note_id) VALUES (OLD.id);
END;
-- Every live note is checked once after the upgrade.
INSERT INTO embedding_dirty(note_id) SELECT id FROM notes WHERE deleted_at IS NULL;
"#, r#"
-- Search as you type at 1M notes: the full-text index gains prefix indexes for 2 and 3
-- characters, so a short prefix reads one list instead of merging every word that
-- starts with it. The index is derived from `notes`, so it is dropped and recreated;
-- no note content is at risk. It is left empty here: the migration that removes the
-- prefix indexes again (`FTS_REFILL_VERSION`) drops it and refills its own.
DROP TRIGGER notes_fts_insert;
DROP TRIGGER notes_fts_delete;
DROP TRIGGER notes_fts_update;
DROP TABLE notes_fts;
CREATE VIRTUAL TABLE notes_fts USING fts5(
    title, body,
    content = 'notes', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
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
-- Folders moved to the trash as a whole. A group is the notes trashed with it: they
-- share its `deleted_at`, which no other trash operation uses. Notes keep their own
-- `folder` and `deleted_at`, so nothing existing changes.
CREATE TABLE trash_folders (
    deleted_at INTEGER PRIMARY KEY,
    folder     TEXT NOT NULL COLLATE NOCASE
);
"#, r#"
-- Star ratings: frontmatter `rating` (1–5) gets its own column, like author and topic,
-- so "4 stars or more" is a range. Most notes are unrated, so the index only holds
-- rated ones. Ratings written before this were generic attributes: a note with exactly
-- one valid `rating` attribute moves it into the column; anything else (`4.5`, `five`,
-- a list) stays an attribute, as the parser keeps it.
ALTER TABLE notes ADD COLUMN rating INTEGER CHECK (rating BETWEEN 1 AND 5);
CREATE INDEX notes_rating ON notes(rating) WHERE rating IS NOT NULL;
CREATE TEMP TABLE rating_moves AS
    SELECT note_id, CAST(min(value) AS INTEGER) AS rating FROM note_attributes
    WHERE key = 'rating' GROUP BY note_id
    HAVING count(*) = 1 AND min(value) IN ('1', '2', '3', '4', '5');
UPDATE notes SET rating = (SELECT m.rating FROM rating_moves m WHERE m.note_id = notes.id)
    WHERE id IN (SELECT note_id FROM rating_moves);
DELETE FROM note_attributes WHERE key = 'rating' AND note_id IN (SELECT note_id FROM rating_moves);
DROP TABLE rating_moves;
"#, r#"
-- Read status: 0 unread, 1 partly read, 2 read. Set by hand, about the reader rather
-- than the note, so it lives here and not in the frontmatter: marking a note doesn't
-- rewrite its body, bump `updated_at` or queue it for embedding (the triggers only
-- watch title, body and deleted_at). Every existing note starts unread. Nearly every
-- note is unread, so the index only holds marked ones, in the order the search lists them.
ALTER TABLE notes ADD COLUMN read_status INTEGER NOT NULL DEFAULT 0 CHECK (read_status BETWEEN 0 AND 2);
CREATE INDEX notes_read ON notes(read_status, updated_at) WHERE read_status > 0;
"#, r#"
-- Folders that hold live notes, with how many, so the folder tree is listed from a row
-- per folder rather than a scan of every note. Paths compare like `notes.folder`
-- (case-insensitively); a row keeps a spelling some live note in it uses. Only folders
-- with notes directly in them have a row; their ancestors are implied. The triggers
-- below are the only writers, so every path that inserts, deletes, trashes, restores or
-- moves notes keeps it right.
CREATE TABLE folders (
    path  TEXT PRIMARY KEY COLLATE NOCASE,
    notes INTEGER NOT NULL CHECK (notes > 0)
) WITHOUT ROWID;
INSERT INTO folders(path, notes)
    SELECT folder, count(*) FROM notes WHERE deleted_at IS NULL GROUP BY folder;

CREATE TRIGGER folders_note_insert AFTER INSERT ON notes WHEN new.deleted_at IS NULL BEGIN
    INSERT INTO folders(path, notes) VALUES (new.folder, 1)
        ON CONFLICT(path) DO UPDATE SET notes = notes + 1;
END;
-- Leaving a folder: drop its row with the last note, else count down and, if the row
-- showed the leaving note's spelling, take one a remaining note uses.
CREATE TRIGGER folders_note_delete AFTER DELETE ON notes WHEN old.deleted_at IS NULL BEGIN
    DELETE FROM folders WHERE path = old.folder AND notes = 1;
    UPDATE folders SET notes = notes - 1,
        path = CASE WHEN path = old.folder COLLATE BINARY
            THEN coalesce((SELECT n.folder FROM notes n WHERE n.folder = old.folder AND n.deleted_at IS NULL LIMIT 1), path)
            ELSE path END
        WHERE path = old.folder;
END;
CREATE TRIGGER folders_note_update AFTER UPDATE OF folder, deleted_at ON notes
WHEN (old.deleted_at IS NULL) <> (new.deleted_at IS NULL)
  OR (new.deleted_at IS NULL AND old.folder IS NOT new.folder COLLATE BINARY)
BEGIN
    DELETE FROM folders WHERE old.deleted_at IS NULL AND path = old.folder AND notes = 1;
    UPDATE folders SET notes = notes - 1,
        path = CASE WHEN path = old.folder COLLATE BINARY
            THEN coalesce((SELECT n.folder FROM notes n WHERE n.folder = old.folder AND n.deleted_at IS NULL AND n.id <> old.id LIMIT 1), path)
            ELSE path END
        WHERE old.deleted_at IS NULL AND path = old.folder;
    INSERT INTO folders(path, notes) SELECT new.folder, 1 WHERE new.deleted_at IS NULL
        ON CONFLICT(path) DO UPDATE SET notes = notes + 1;
END;
"#, r#"
-- Folder sizes: the UTF-8 bytes of the live notes' bodies directly in each folder, for
-- the folder tree; subtree totals are added up from these rows. `octet_length` is the
-- body's length in the vault's encoding, UTF-8. The column starts at 0 and is filled
-- afterwards (by `fill_folder_sizes`, in batches, with progress); nothing existing
-- changes. The triggers are replaced to keep `bytes` along with `notes`, and a new one
-- follows edits. A body edit is counted by exactly one of them: by
-- `folders_note_update` when the same statement also moves or trashes the note, else
-- by `folders_note_body`.
ALTER TABLE folders ADD COLUMN bytes INTEGER NOT NULL DEFAULT 0;

DROP TRIGGER folders_note_insert;
DROP TRIGGER folders_note_delete;
DROP TRIGGER folders_note_update;
CREATE TRIGGER folders_note_insert AFTER INSERT ON notes WHEN new.deleted_at IS NULL BEGIN
    INSERT INTO folders(path, notes, bytes) VALUES (new.folder, 1, octet_length(new.body))
        ON CONFLICT(path) DO UPDATE SET notes = notes + 1, bytes = bytes + excluded.bytes;
END;
CREATE TRIGGER folders_note_delete AFTER DELETE ON notes WHEN old.deleted_at IS NULL BEGIN
    DELETE FROM folders WHERE path = old.folder AND notes = 1;
    UPDATE folders SET notes = notes - 1, bytes = bytes - octet_length(old.body),
        path = CASE WHEN path = old.folder COLLATE BINARY
            THEN coalesce((SELECT n.folder FROM notes n WHERE n.folder = old.folder AND n.deleted_at IS NULL LIMIT 1), path)
            ELSE path END
        WHERE path = old.folder;
END;
CREATE TRIGGER folders_note_update AFTER UPDATE OF folder, deleted_at ON notes
WHEN (old.deleted_at IS NULL) <> (new.deleted_at IS NULL)
  OR (new.deleted_at IS NULL AND old.folder IS NOT new.folder COLLATE BINARY)
BEGIN
    DELETE FROM folders WHERE old.deleted_at IS NULL AND path = old.folder AND notes = 1;
    UPDATE folders SET notes = notes - 1, bytes = bytes - octet_length(old.body),
        path = CASE WHEN path = old.folder COLLATE BINARY
            THEN coalesce((SELECT n.folder FROM notes n WHERE n.folder = old.folder AND n.deleted_at IS NULL AND n.id <> old.id LIMIT 1), path)
            ELSE path END
        WHERE old.deleted_at IS NULL AND path = old.folder;
    INSERT INTO folders(path, notes, bytes) SELECT new.folder, 1, octet_length(new.body) WHERE new.deleted_at IS NULL
        ON CONFLICT(path) DO UPDATE SET notes = notes + 1, bytes = bytes + excluded.bytes;
END;
-- An edit of a live note that stays in its folder (spelled the same): the complement of
-- `folders_note_update`'s condition for live notes.
CREATE TRIGGER folders_note_body AFTER UPDATE OF body ON notes
WHEN old.deleted_at IS NULL AND new.deleted_at IS NULL AND old.folder IS new.folder COLLATE BINARY
  AND octet_length(old.body) <> octet_length(new.body)
BEGIN
    UPDATE folders SET bytes = bytes + octet_length(new.body) - octet_length(old.body) WHERE path = new.folder;
END;
"#, r#"
-- What the note lists show of a note, in an index, so listing never reads note bodies.
-- `deleted_at`, `rating` and `read_status` come after the body in a row, so reading them
-- from the table reads the whole body first. One index is in the tree's order (a
-- folder's notes by title), the other newest first, as lists without search text are.
-- `deleted_at` is in both (always NULL) so that a query naming it stays in the index.
CREATE INDEX notes_list_folder ON notes(folder, title, author, topic, created_at, updated_at, rating, read_status, deleted_at)
    WHERE deleted_at IS NULL;
-- `notes_list_updated` is created by `create_list_indexes`, so the unlock screen can
-- say how far this is: each index reads every note once.
"#, r#"
-- The prefix indexes go again: on a 569,170-note vault they took 7.5 GB, 55% of the
-- full-text index, to save a fraction of a second on a 2- or 3-letter search. A word
-- start is matched from 3 letters on (`search::PREFIX_MIN_CHARS`), from the whole-word
-- index. As before the index is derived from `notes`: it is dropped and refilled (by
-- `fill_fts`, in batches, with progress), and no note content is touched. The pages of
-- the old index are free for the new one right away; what is left over stays in the
-- file until the vault is compacted.
DROP TRIGGER notes_fts_insert;
DROP TRIGGER notes_fts_delete;
DROP TRIGGER notes_fts_update;
DROP TABLE notes_fts;
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
"#];

/// The last migration that recreates the full-text index; [`fill_fts`] refills it. An
/// earlier one (version 9, which added the prefix indexes this one removes) leaves the
/// index empty, so a vault from before it indexes its notes once, not twice.
const FTS_REFILL_VERSION: usize = 16;
/// Notes indexed per step while refilling, between progress reports.
const FTS_FILL_BATCH: i64 = 2_000;

/// The migration that adds `folders.bytes`; [`fill_folder_sizes`] fills it.
const FOLDER_SIZES_VERSION: usize = 14;
/// Notes measured per step while filling folder sizes, between progress reports.
/// Larger than [`FTS_FILL_BATCH`]: a step only reads bodies, it writes a row per folder.
const SIZES_FILL_BATCH: i64 = 10_000;

/// Which migration that takes a while is running, so the unlock screen can say what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpgradeStep {
    /// Rebuilding the full-text index.
    SearchIndex,
    /// Measuring note sizes for the folder tree.
    FolderSizes,
    /// Building the indexes the note lists read; `done` of `total` indexes, not notes.
    ListIndexes,
}

/// How far a migration that takes a while has got, for the unlock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Upgrade {
    pub step: UpgradeStep,
    pub done: usize,
    pub total: usize,
}

/// Indexes every note, trashed ones too (as the triggers do), in batches.
fn fill_fts(tx: &Connection, progress: &mut dyn FnMut(Upgrade)) -> Result<()> {
    let total: i64 = tx.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?;
    let mut stmt = tx.prepare(
        "INSERT INTO notes_fts(rowid, title, body)
         SELECT id, title, body FROM notes WHERE id > ?1 ORDER BY id LIMIT ?2",
    )?;
    let mut last = tx.prepare("SELECT max(id) FROM (SELECT id FROM notes WHERE id > ?1 ORDER BY id LIMIT ?2)")?;
    let (mut after, mut done) = (0i64, 0usize);
    let report = |done| Upgrade { step: UpgradeStep::SearchIndex, done, total: total as usize };
    progress(report(done));
    loop {
        let Some(next) = last.query_row(params![after, FTS_FILL_BATCH], |r| r.get::<_, Option<i64>>(0))? else { break };
        done += stmt.execute(params![after, FTS_FILL_BATCH])?;
        after = next;
        progress(report(done));
    }
    Ok(())
}

/// Adds every live note's body length to its folder's `bytes`, a batch of notes (by id)
/// at a time. `total` counts trashed notes too, since each batch walks them: the count
/// moves at an even pace whatever is in the trash.
fn fill_folder_sizes(tx: &Connection, progress: &mut dyn FnMut(Upgrade)) -> Result<()> {
    let total = tx.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))? as usize;
    // Grouped like the `folders` key (the column's NOCASE), so each folder row matches
    // one group.
    let mut add = tx.prepare(
        "UPDATE folders SET bytes = folders.bytes + s.bytes
         FROM (SELECT folder, sum(octet_length(body)) AS bytes FROM notes
               WHERE id > ?1 AND id <= ?2 AND deleted_at IS NULL GROUP BY folder) AS s
         WHERE folders.path = s.folder",
    )?;
    let mut last = tx.prepare("SELECT max(id), count(*) FROM (SELECT id FROM notes WHERE id > ?1 ORDER BY id LIMIT ?2)")?;
    let (mut after, mut done) = (0i64, 0usize);
    let report = |done| Upgrade { step: UpgradeStep::FolderSizes, done, total };
    progress(report(done));
    loop {
        let (next, count) = last.query_row(params![after, SIZES_FILL_BATCH], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?)))?;
        let Some(next) = next else { break };
        add.execute(params![after, next])?;
        done += count as usize;
        after = next;
        progress(report(done));
    }
    Ok(())
}

/// The migration that adds the note list indexes; [`create_list_indexes`] finishes it.
const LIST_INDEXES_VERSION: usize = 15;

/// Creates the second list index and reports both: an index is built in one statement,
/// which reads every note (the indexed columns lie after the body), so on a large vault
/// each takes a while and the progress is one step per index.
fn create_list_indexes(tx: &rusqlite::Transaction<'_>, first_sql: &str, progress: &mut dyn FnMut(Upgrade)) -> Result<()> {
    let report = |done| Upgrade { step: UpgradeStep::ListIndexes, done, total: 2 };
    progress(report(0));
    tx.execute_batch(first_sql)?;
    progress(report(1));
    tx.execute_batch(
        "CREATE INDEX notes_list_updated ON notes(updated_at, id, folder, title, author, topic, created_at, rating, read_status, deleted_at)
             WHERE deleted_at IS NULL;",
    )?;
    progress(report(2));
    Ok(())
}

/// Vaults created before this version get the default import rules.
const IMPORT_RULES_VERSION: usize = 2;

/// Brings the vault to the current schema, in one transaction: a failure part way (an
/// error, a full disk, a crash) leaves the vault at the version it had. Returns whether
/// there was anything to do.
pub(super) fn migrate(conn: &mut Connection, progress: &mut dyn FnMut(Upgrade)) -> Result<bool> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let version = version as usize;
    if version > MIGRATIONS.len() {
        return Err(Error::UnsupportedVersion);
    }
    if version == MIGRATIONS.len() {
        return Ok(false);
    }
    apply_migrations(conn, version, progress).map_err(|e| if disk_full(&e) { Error::UpgradeDiskFull } else { e })?;
    Ok(true)
}

/// Whether SQLite gave up because the drive (or the `-wal` file's drive) is full.
fn disk_full(e: &Error) -> bool {
    matches!(e, Error::Sql(rusqlite::Error::SqliteFailure(f, _)) if f.code == rusqlite::ErrorCode::DiskFull)
}

fn apply_migrations(conn: &mut Connection, version: usize, progress: &mut dyn FnMut(Upgrade)) -> Result<()> {
    let tx = conn.transaction()?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        if i + 1 == LIST_INDEXES_VERSION {
            create_list_indexes(&tx, sql, progress)?;
        } else {
            tx.execute_batch(sql)?;
        }
        if i + 1 == IMPORT_RULES_VERSION {
            rules::write_rules(&tx, &Rule::defaults())?;
        }
        if i + 1 == FTS_REFILL_VERSION {
            fill_fts(&tx, progress)?;
        }
        if i + 1 == FOLDER_SIZES_VERSION {
            fill_folder_sizes(&tx, progress)?;
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

        migrate(&mut conn, &mut |_| {}).unwrap();
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

        migrate(&mut conn, &mut |_| {}).unwrap();
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

        migrate(&mut conn, &mut |_| {}).unwrap();
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

        migrate(&mut conn, &mut |_| {}).unwrap();
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

    /// A vault from before tag classifiers (version 6) keeps its notes, vectors,
    /// descriptions and feedback and gains an empty, case-sensitive `tag_heads` table.
    #[test]
    fn tag_heads_migration_keeps_descriptions_and_feedback() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..6] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 6).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (1, 'T', 'body #jr', 1, 1);
             INSERT INTO tags(id, name) VALUES (7, 'jr');
             INSERT INTO note_tags VALUES (1, 7);
             INSERT INTO note_embeddings VALUES (1, 'm', x'00', x'0000803f');
             INSERT INTO tag_descriptions VALUES ('jr', 'journal', x'0000803f', 'm');
             INSERT INTO tag_feedback VALUES (1, 'dr', 0, 5);
             INSERT INTO settings VALUES ('suggest_threshold:m', '0.5');",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        assert_eq!(version, 16);
        let body: String = conn.query_row("SELECT body FROM notes WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "body #jr");
        let vectors: i64 = conn.query_row("SELECT count(*) FROM note_embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(vectors, 1);
        let description: (String, String) =
            conn.query_row("SELECT description, model FROM tag_descriptions", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(description, ("journal".to_string(), "m".to_string()));
        let feedback: (String, bool, i64) =
            conn.query_row("SELECT tag, accepted, created_at FROM tag_feedback", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(feedback, ("dr".to_string(), false, 5));
        let threshold: String =
            conn.query_row("SELECT value FROM settings WHERE key = 'suggest_threshold:m'", [], |r| r.get(0)).unwrap();
        assert_eq!(threshold, "0.5");
        let heads: i64 = conn.query_row("SELECT count(*) FROM tag_heads", [], |r| r.get(0)).unwrap();
        assert_eq!(heads, 0);

        let insert = "INSERT INTO tag_heads VALUES (?1, 'm', x'0000803f', 0.5, 10, 12, 0.9, NULL, 3, x'00')";
        conn.execute(insert, ["jr"]).unwrap();
        conn.execute(insert, ["Jr"]).unwrap();
        assert!(conn.execute(insert, ["jr"]).is_err(), "one head per tag");
        let recall: Option<f64> = conn.query_row("SELECT recall FROM tag_heads WHERE tag = 'jr'", [], |r| r.get(0)).unwrap();
        assert_eq!(recall, None);
    }

    /// A vault from before the embedding queue (version 7) keeps its notes and vectors
    /// and queues every live note exactly once; later writes queue through the triggers.
    #[test]
    fn embedding_queue_migration_queues_every_live_note_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..7] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (1, 'A', 'a', 1, 1), (2, 'B', 'b', 1, 1);
             INSERT INTO notes(id, title, body, created_at, updated_at, deleted_at) VALUES (3, 'C', 'c', 1, 1, 9);
             INSERT INTO note_embeddings VALUES (1, 'm', x'00', x'0000803f');",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let queued = |conn: &Connection| -> Vec<(i64, i64)> {
            let mut stmt = conn.prepare("SELECT note_id, version FROM embedding_dirty ORDER BY note_id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(queued(&conn), [(1, 0), (2, 0)], "live notes, once each");
        let vectors: i64 = conn.query_row("SELECT count(*) FROM note_embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(vectors, 1);
        migrate(&mut conn, &mut |_| {}).unwrap();
        assert_eq!(queued(&conn), [(1, 0), (2, 0)], "migrating again changes nothing");

        conn.execute("DELETE FROM embedding_dirty", []).unwrap();
        conn.execute("UPDATE notes SET updated_at = 5, folder = 'x' WHERE id = 1", []).unwrap();
        assert_eq!(queued(&conn), [], "a folder or date change doesn't queue");
        conn.execute("UPDATE notes SET body = 'new' WHERE id = 1", []).unwrap();
        conn.execute("UPDATE notes SET title = 'New' WHERE id = 1", []).unwrap();
        conn.execute("UPDATE notes SET deleted_at = NULL WHERE id = 3", []).unwrap();
        conn.execute("INSERT INTO notes(id, title, created_at, updated_at) VALUES (4, 'D', 1, 1)", []).unwrap();
        assert_eq!(queued(&conn), [(1, 1), (3, 0), (4, 0)]);
        conn.execute("DELETE FROM notes WHERE id = 4", []).unwrap();
        assert_eq!(queued(&conn), [(1, 1), (3, 0)], "the queue cascades from notes");
        let gone: i64 = conn.query_row("SELECT note_id FROM embedding_gone", [], |r| r.get(0)).unwrap();
        assert_eq!(gone, 4, "a deleted note is remembered for the related-notes index");
    }

    /// A vault from before the prefix index (version 8) gets its full-text index rebuilt
    /// with every note, trashed ones too, and reports progress; search works after. The
    /// index is recreated twice on the way (versions 9 and 16) but filled once.
    #[test]
    fn a_vault_from_before_the_prefix_indexes_gets_its_index_filled_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..8] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 8).unwrap();
        for i in 0..4_500 {
            conn.execute(
                "INSERT INTO notes(title, body, created_at, updated_at, deleted_at) VALUES (?1, ?2, 1, 1, ?3)",
                params![format!("Note {i}"), format!("lighthouse number{i}"), (i == 7).then_some(5)],
            )
            .unwrap();
        }
        let mut reports = Vec::new();
        migrate(&mut conn, &mut |u| reports.push(u)).unwrap();
        // Later migrations report their own step; only the index's counts here.
        reports.retain(|u| u.step == UpgradeStep::SearchIndex);
        let at = |done| Upgrade { step: UpgradeStep::SearchIndex, done, total: 4_500 };
        assert_eq!(reports.first(), Some(&at(0)));
        assert_eq!(reports.last(), Some(&at(4_500)));
        assert_eq!(reports.len(), 4, "start + three batches of 2,000");

        let count = |q: &str| -> i64 {
            conn.query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH ?1", [q], |r| r.get(0)).unwrap()
        };
        assert_eq!(count("lighthouse"), 4_500);
        assert_eq!(count("\"number44\"*"), 111, "number44, 440–449 and 4400–4499");
        assert_eq!(count("\"nu\"*"), 4_500);
        // The triggers are back: edits keep the index in step.
        conn.execute("UPDATE notes SET body = 'harbour' WHERE id = 1", []).unwrap();
        assert_eq!(count("harbour"), 1);
        assert_eq!(count("lighthouse"), 4_499);
        conn.execute("DELETE FROM notes WHERE id = 2", []).unwrap();
        assert_eq!(count("lighthouse"), 4_498);
        conn.execute("INSERT INTO notes_fts(notes_fts) VALUES ('integrity-check')", []).unwrap();
    }

    fn fts_definition(conn: &Connection) -> String {
        conn.query_row("SELECT sql FROM sqlite_master WHERE name = 'notes_fts'", [], |r| r.get(0)).unwrap()
    }

    /// A vault with the prefix indexes (version 15) gets its full-text index rebuilt
    /// without them, with every note, trashed ones too, and reports progress. Whole
    /// words and word starts of 3 and 4 letters find the same notes as before.
    #[test]
    fn fts_without_prefix_migration_finds_the_same_notes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..15] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "CREATE INDEX notes_list_updated ON notes(updated_at, id, folder, title, author, topic, created_at, rating, read_status, deleted_at)
                 WHERE deleted_at IS NULL;",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 15).unwrap();
        assert!(fts_definition(&conn).contains("prefix"), "the vault starts with the prefix indexes");
        const WORDS: [&str; 6] = ["lighthouse", "light", "lignite", "harbour", "hark", "Café"];
        for i in 0..4_500usize {
            conn.execute(
                "INSERT INTO notes(title, body, created_at, updated_at, deleted_at) VALUES (?1, ?2, 1, 1, ?3)",
                params![format!("Note {i}"), format!("{} {} number{i}", WORDS[i % 6], WORDS[i % 5]), (i == 7).then_some(5)],
            )
            .unwrap();
        }
        let ids = |conn: &Connection, q: &str| -> Vec<i64> {
            let mut stmt = conn.prepare("SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?1 ORDER BY rowid").unwrap();
            stmt.query_map([q], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        // Whole words, 3-letter and 4-letter word starts, in a title, with a diacritic,
        // as a phrase and combined.
        let queries = [
            "\"lighthouse\"", "\"hark\"", "\"cafe\"", "\"7\"", "\"lig\"*", "\"har\"*", "\"caf\"*", "\"not\"*", "\"ligh\"*",
            "\"harb\"*", "\"numb\"*", "\"number44\"*", "\"light har\"*", "\"note\" \"lign\"*", "\"xyz\"*",
        ];
        let before: Vec<Vec<i64>> = queries.iter().map(|q| ids(&conn, q)).collect();
        for (q, found) in queries.iter().zip(&before) {
            assert_eq!(found.is_empty(), *q == "\"xyz\"*", "{q} finds something to compare");
        }
        assert_eq!(before[11].len(), 111, "number44, 440–449 and 4400–4499");

        let mut reports = Vec::new();
        migrate(&mut conn, &mut |u| reports.push(u)).unwrap();
        let at = |done| Upgrade { step: UpgradeStep::SearchIndex, done, total: 4_500 };
        assert_eq!(reports, [at(0), at(2_000), at(4_000), at(4_500)]);

        assert!(!fts_definition(&conn).contains("prefix"), "{}", fts_definition(&conn));
        for (q, before) in queries.iter().zip(&before) {
            assert_eq!(&ids(&conn, q), before, "{q}");
        }
        // The triggers are back: edits keep the index in step.
        let count = |q: &str| ids(&conn, q).len();
        let lighthouses = before[0].len();
        assert_eq!(before[0][..2], [1, 6], "notes 1 and 6 have the word");
        conn.execute("UPDATE notes SET body = 'breakwater' WHERE id = 1", []).unwrap();
        assert_eq!(count("breakwater"), 1);
        assert_eq!(count("lighthouse"), lighthouses - 1);
        conn.execute("DELETE FROM notes WHERE id = 6", []).unwrap();
        assert_eq!(count("lighthouse"), lighthouses - 2);
        conn.execute("INSERT INTO notes(title, created_at, updated_at) VALUES ('Mole', 1, 1)", []).unwrap();
        assert_eq!(count("\"mol\"*"), 1);
        conn.execute("INSERT INTO notes_fts(notes_fts) VALUES ('integrity-check')", []).unwrap();
    }

    /// When the drive fills up during the refill the upgrade says so, and the vault is
    /// as it was: same version, same notes, and it upgrades once there is room.
    #[test]
    fn a_full_disk_leaves_the_vault_at_its_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..15] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 15).unwrap();
        for i in 0..500 {
            let body: String = (0..200).map(|w| format!("word{} ", i * 200 + w)).collect();
            conn.execute("INSERT INTO notes(title, body, created_at, updated_at) VALUES (?1, ?2, 1, 1)", params![format!("Note {i}"), body])
                .unwrap();
        }
        // An empty index and no free pages, so the refill has to grow the file; then no
        // room to grow.
        conn.execute_batch("INSERT INTO notes_fts(notes_fts) VALUES ('delete-all'); VACUUM;").unwrap();
        let pages: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0)).unwrap();
        let limit = |conn: &Connection, max: i64| {
            conn.query_row(&format!("PRAGMA max_page_count = {max}"), [], |r| r.get::<_, i64>(0)).unwrap();
        };
        limit(&conn, pages + 10);

        let err = migrate(&mut conn, &mut |_| {}).unwrap_err();
        assert!(matches!(err, Error::UpgradeDiskFull), "{err:?}");
        assert_eq!(conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)).unwrap(), 15);
        assert!(fts_definition(&conn).contains("prefix"), "the old index is still there");
        assert_eq!(conn.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0)).unwrap(), 500);

        limit(&conn, 1_000_000);
        assert!(migrate(&mut conn, &mut |_| {}).unwrap());
        let found: i64 =
            conn.query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH '\"word4\"*'", [], |r| r.get(0)).unwrap();
        assert_eq!(found, 1 + 1 + 5 + 50, "word4… is in the first note, the third, 5 from the 21st and 50 from the 201st");
        assert!(!migrate(&mut conn, &mut |_| {}).unwrap(), "nothing left to do");
    }

    /// Benchmark, not a check: what removing the prefix indexes costs and gives on a
    /// generated vault (`HUSH_PREFIX_NOTES` notes of 400 words, default 50,000). Builds an
    /// encrypted vault at version 15, times searches, upgrades it through `Vault::open`,
    /// times the same searches again, and prints sizes along the way.
    /// `cargo test --release --lib prefix_indexes_size_and_speed -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark; run by hand"]
    fn prefix_indexes_size_and_speed() {
        use crate::db::{NewNote, SearchQuery, Vault, compact::available_space, run_pragma};
        use std::time::Instant;

        const PASS: &str = "correct horse battery staple";
        const COMMON: [&str; 40] = [
            "the", "of", "and", "to", "a", "in", "is", "it", "you", "that", "he", "was", "for", "on", "are", "with", "as", "his",
            "they", "be", "at", "one", "have", "this", "from", "or", "had", "by", "not", "word", "but", "what", "some", "we", "can",
            "out", "other", "were", "all", "there",
        ];
        const ONSETS: [&str; 20] =
            ["b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "st", "tr", "pl", "gr", "sh"];
        const VOWELS: [&str; 8] = ["a", "e", "i", "o", "u", "ai", "ou", "ee"];
        const CODAS: [&str; 8] = ["", "n", "r", "s", "t", "l", "nd", "st"];
        // SplitMix64: the same vault every run.
        let mut state = 7u64;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut words: Vec<String> = COMMON.iter().map(|w| w.to_string()).collect();
        while words.len() < 30_000 {
            let word: String = (0..2 + next() % 3)
                .map(|_| {
                    let mut pick = |from: &[&'static str]| from[(next() % from.len() as u64) as usize];
                    format!("{}{}{}", pick(&ONSETS), pick(&VOWELS), pick(&CODAS))
                })
                .collect();
            if !words.contains(&word) {
                words.push(word);
            }
        }
        let notes: usize = std::env::var("HUSH_PREFIX_NOTES").ok().and_then(|n| n.parse().ok()).unwrap_or(50_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.hush");
        let size = |suffix: &str| std::fs::metadata(format!("{}{suffix}", path.display())).map(|m| m.len()).unwrap_or(0);
        let mb = |bytes: u64| format!("{:.1} MB", bytes as f64 / 1e6);

        let mut vault = Vault::create(&path, PASS).unwrap();
        // Back to the index of version 15, so the notes below fill its prefix indexes.
        vault.conn.execute_batch(MIGRATIONS[8]).unwrap();
        vault.conn.pragma_update(None, "user_version", 15).unwrap();
        let started = Instant::now();
        let mut text_bytes = 0;
        for chunk in 0..notes.div_ceil(1_000) {
            let rows: Vec<(String, String)> = (chunk * 1_000..notes.min((chunk + 1) * 1_000))
                .map(|i| {
                    // Zipf (s = 1) over the vocabulary, like word frequencies in text.
                    let body: Vec<&str> = (0..400)
                        .map(|_| {
                            let unit = (next() >> 11) as f64 / (1u64 << 53) as f64;
                            words[((words.len() as f64 + 1.0).powf(unit) as usize).saturating_sub(1).min(words.len() - 1)].as_str()
                        })
                        .collect();
                    (format!("Note {i}"), body.join(" "))
                })
                .collect();
            text_bytes += rows.iter().map(|(_, body)| body.len()).sum::<usize>();
            let new: Vec<NewNote> =
                rows.iter().map(|(title, body)| NewNote { folder: "", title, body, created_at: 1, updated_at: 1 }).collect();
            assert!(vault.import_notes(&new).unwrap().iter().all(|r| r.is_ok()));
        }
        run_pragma(&vault.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        println!("{notes} notes, {} of text, written in {:.0?}", mb(text_bytes as u64), started.elapsed());

        let index_bytes = |vault: &Vault| -> u64 {
            let sql = "SELECT (SELECT coalesce(sum(length(block)), 0) FROM notes_fts_data)
                            + (SELECT coalesce(sum(length(term) + 16), 0) FROM notes_fts_idx)";
            vault.conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as u64
        };
        let segments = |vault: &Vault| -> i64 {
            vault.conn.query_row("SELECT count(DISTINCT segid) FROM notes_fts_idx", [], |r| r.get(0)).unwrap()
        };
        // The most common word start of a length, the median one and the rarest, by how
        // often the words that start with it are drawn.
        let starts = |len: usize| -> [String; 3] {
            let mut weight = std::collections::BTreeMap::<&str, f64>::new();
            for (rank, word) in words.iter().enumerate().filter(|(_, w)| w.len() >= len) {
                *weight.entry(&word[..len]).or_default() += 1.0 / (rank as f64 + 1.0);
            }
            let mut by_weight: Vec<(&str, f64)> = weight.into_iter().collect();
            by_weight.sort_by(|a, b| b.1.total_cmp(&a.1));
            [0, by_weight.len() / 2, by_weight.len() - 1].map(|i| by_weight[i].0.to_string())
        };
        let typed: Vec<String> = [starts(3), starts(4)].concat();
        let two = starts(2);
        let timings = |vault: &Vault| -> Vec<(f64, usize)> {
            typed
                .iter()
                .map(|typed| {
                    let q = SearchQuery { text: typed.to_string(), ..Default::default() };
                    let mut ms: Vec<f64> = (0..5)
                        .map(|_| {
                            let started = Instant::now();
                            vault.search_page(&q).unwrap();
                            started.elapsed().as_secs_f64() * 1e3
                        })
                        .collect();
                    ms.sort_by(f64::total_cmp);
                    let matches: i64 = vault
                        .conn
                        .query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH ?1", [format!("\"{typed}\"*")], |r| r.get(0))
                        .unwrap();
                    (ms[2], matches as usize)
                })
                .collect()
        };
        // A word start of 2 letters as version 15 ran it, on whichever index there is.
        let two_letter_starts = |vault: &Vault| -> Vec<f64> {
            two.iter()
                .map(|typed| {
                    let mut ms: Vec<f64> = (0..5)
                        .map(|_| {
                            let started = Instant::now();
                            let sql = "SELECT count(*) FROM (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?1 LIMIT 10001)";
                            vault.conn.query_row(sql, [format!("\"{typed}\"*")], |r| r.get::<_, i64>(0)).unwrap();
                            started.elapsed().as_secs_f64() * 1e3
                        })
                        .collect();
                    ms.sort_by(f64::total_cmp);
                    ms[2]
                })
                .collect()
        };

        let (file_with, index_with, segments_with) = (size(""), index_bytes(&vault), segments(&vault));
        let (with, two_with) = (timings(&vault), two_letter_starts(&vault));
        drop(vault);

        let started = Instant::now();
        // The `-wal` file as each progress report sees it: what the upgrade needs in free
        // disk space, less what the last batch and the commit still add.
        let (mut steps, mut wal_peak) = (0, 0);
        let mut vault = Vault::open_with_progress(&path, PASS, &mut |_| {
            steps += 1;
            wal_peak = wal_peak.max(size("-wal"));
        })
        .unwrap();
        let upgrade = started.elapsed();
        let (file_after, wal_after, index_without, segments_without) = (size(""), size("-wal"), index_bytes(&vault), segments(&vault));
        let (without, two_without) = (timings(&vault), two_letter_starts(&vault));
        let space = vault.space().unwrap();

        println!("upgrade: {upgrade:.1?} ({steps} progress reports), -wal file up to {} meanwhile", mb(wal_peak));
        println!("search index: {} with prefix indexes ({segments_with} segments), {} without ({segments_without} segments)", mb(index_with), mb(index_without));
        println!("vault file: {} before, {} after the upgrade (-wal {}), {} free inside, worth compacting: {}", mb(file_with), mb(file_after), mb(wal_after), mb(space.reclaimable_bytes), space.worth_compacting);
        let compacted = vault.compact(&available_space).unwrap();
        println!("after Compact vault: {} ({} ms)", mb(compacted.after_bytes), compacted.millis);
        println!("search as typed (most common, median and rarest start of 3 and of 4 letters), median of 5 (ms):");
        println!("  text, matching notes, with prefix indexes, without");
        for (i, typed) in typed.iter().enumerate() {
            println!("  {typed:<5} {:>7} {:>9.2} {:>9.2}", with[i].1, with[i].0, without[i].0);
        }
        println!("counting a 2-letter word start up to the rank limit, which no search does any more (ms): with, without");
        for (i, typed) in two.iter().enumerate() {
            println!("  {typed:<5} {:>9.2} {:>9.2}", two_with[i], two_without[i]);
        }
    }

    /// A new vault gets the same full-text index as an upgraded one: no prefix indexes.
    #[test]
    fn a_new_vault_has_no_prefix_indexes() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &mut |_| {}).unwrap();
        let sql = fts_definition(&conn);
        assert!(!sql.contains("prefix"), "{sql}");
        assert!(sql.contains("content = 'notes', content_rowid = 'id'") && sql.contains("unicode61 remove_diacritics 2"), "{sql}");
    }

    /// A vault from before folder trash (version 9) keeps its notes, trashed ones too,
    /// and gains an empty `trash_folders` table keyed on `deleted_at`.
    #[test]
    fn trash_folders_migration_keeps_notes_and_trash() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..9] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 9).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, folder, title, body, created_at, updated_at) VALUES (1, 'a', 'A', 'x', 1, 1);
             INSERT INTO notes(id, folder, title, body, created_at, updated_at, deleted_at) VALUES (2, 'a/b', 'B', 'y', 1, 1, 7);",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let notes: Vec<(i64, Option<i64>)> = conn
            .prepare("SELECT id, deleted_at FROM notes ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(notes, [(1, None), (2, Some(7))]);
        let groups: i64 = conn.query_row("SELECT count(*) FROM trash_folders", [], |r| r.get(0)).unwrap();
        assert_eq!(groups, 0);
        conn.execute("INSERT INTO trash_folders VALUES (7, 'a/b')", []).unwrap();
        assert!(conn.execute("INSERT INTO trash_folders VALUES (7, 'c')", []).is_err(), "one group per deleted_at");
    }

    /// A vault from before ratings (version 10) moves valid `rating` attributes into the
    /// new column and keeps invalid ones (and lists) as attributes.
    #[test]
    fn rating_migration_moves_valid_attributes_into_the_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..10] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 10).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES
                 (1, 'Four', '---\nrating: 4\n---\n', 1, 1), (2, 'Half', '---\nrating: 4.5\n---\n', 1, 1),
                 (3, 'List', '---\nrating: [3, 5]\n---\n', 1, 1), (4, 'None', '', 1, 1),
                 (5, 'Nine', '---\nrating: 9\nmood: calm\n---\n', 1, 1);
             INSERT INTO note_attributes VALUES (1, 'rating', '4'), (2, 'rating', '4.5'), (3, 'rating', '3'),
                 (3, 'rating', '5'), (5, 'rating', '9'), (5, 'mood', 'calm');",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let ratings: Vec<(i64, Option<i64>)> = conn
            .prepare("SELECT id, rating FROM notes ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ratings, [(1, Some(4)), (2, None), (3, None), (4, None), (5, None)]);
        let attrs: Vec<(i64, String, String)> = conn
            .prepare("SELECT note_id, key, value FROM note_attributes ORDER BY note_id, key, value")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let attrs: Vec<(i64, &str, &str)> = attrs.iter().map(|(n, k, v)| (*n, k.as_str(), v.as_str())).collect();
        assert_eq!(attrs, [(2, "rating", "4.5"), (3, "rating", "3"), (3, "rating", "5"), (5, "mood", "calm"), (5, "rating", "9")]);
        assert!(conn.execute("UPDATE notes SET rating = 6 WHERE id = 4", []).is_err(), "1 to 5 only");
        assert!(conn.execute("UPDATE notes SET rating = 0 WHERE id = 4", []).is_err());
    }

    /// A vault from before read status (version 11) opens with every note unread, trashed
    /// ones too, and nothing queued for embedding or re-indexed.
    #[test]
    fn read_status_migration_leaves_every_note_unread() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..11] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 11).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, title, body, created_at, updated_at) VALUES (1, 'A', 'a', 1, 2), (2, 'B', 'b', 1, 3);
             INSERT INTO notes(id, title, body, created_at, updated_at, deleted_at) VALUES (3, 'C', 'c', 1, 4, 9);
             DELETE FROM embedding_dirty;",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let rows: Vec<(i64, i64, i64)> = conn
            .prepare("SELECT id, read_status, updated_at FROM notes ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(rows, [(1, 0, 2), (2, 0, 3), (3, 0, 4)], "unread, dates kept");
        let queued: i64 = conn.query_row("SELECT count(*) FROM embedding_dirty", [], |r| r.get(0)).unwrap();
        assert_eq!(queued, 0, "the upgrade queues nothing");

        conn.execute("UPDATE notes SET read_status = 2 WHERE id = 1", []).unwrap();
        let queued: i64 = conn.query_row("SELECT count(*) FROM embedding_dirty", [], |r| r.get(0)).unwrap();
        assert_eq!(queued, 0, "marking a note doesn't queue it");
        assert!(conn.execute("UPDATE notes SET read_status = 3 WHERE id = 1", []).is_err(), "0 to 2 only");
        assert!(conn.execute("UPDATE notes SET read_status = -1 WHERE id = 1", []).is_err());
        assert!(conn.execute("UPDATE notes SET read_status = NULL WHERE id = 1", []).is_err());
        let found: i64 = conn
            .query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(found, 1, "the full-text index is untouched");
    }

    /// A vault from before the folders table (version 12) gets one row per folder that
    /// holds live notes, with their count; trashed notes and case variants don't add rows.
    #[test]
    fn folders_migration_counts_live_notes_per_folder() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..12] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 12).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, folder, title, body, created_at, updated_at) VALUES
                 (1, '', 'R', 'r', 1, 1), (2, 'a', 'A', 'a', 1, 1), (3, 'A', 'B', 'b', 1, 1),
                 (4, 'a/b', 'C', 'c', 1, 1);
             INSERT INTO notes(id, folder, title, body, created_at, updated_at, deleted_at) VALUES
                 (5, 'a/b', 'D', 'd', 1, 1, 9), (6, 'gone', 'E', 'e', 1, 1, 9);",
        )
        .unwrap();

        migrate(&mut conn, &mut |_| {}).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let rows: Vec<(String, i64)> = conn
            .prepare("SELECT lower(path), notes FROM folders ORDER BY path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(rows, [("".into(), 1), ("a".into(), 2), ("a/b".into(), 1)]);

        conn.execute("UPDATE notes SET deleted_at = NULL WHERE id = 6", []).unwrap();
        let gone: i64 = conn.query_row("SELECT notes FROM folders WHERE path = 'gone'", [], |r| r.get(0)).unwrap();
        assert_eq!(gone, 1, "the triggers keep it up from here");
    }

    /// A vault from before folder sizes (version 13) gets each folder's byte total
    /// filled from its live notes, equal to a direct SUM over them, with progress in
    /// batches; trashed notes don't count and the triggers keep it right from there.
    #[test]
    fn folder_sizes_migration_backfills_live_bytes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..13] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 13).unwrap();
        const NOTES: i64 = 25_000;
        let folders = ["", "a", "A", "a/b", "ünï/cødé", "c/d/e"];
        {
            let mut insert = conn
                .prepare("INSERT INTO notes(folder, title, body, created_at, updated_at, deleted_at) VALUES (?1, ?2, ?3, 1, 1, ?4)")
                .unwrap();
            for i in 0..NOTES {
                let folder = folders[i as usize % folders.len()];
                let body = format!("{} é🙂 {i}", "word ".repeat((i % 37) as usize));
                insert.execute(params![folder, format!("Note {i}"), body, (i % 11 == 0).then_some(5)]).unwrap();
            }
        }
        let direct = |conn: &Connection| -> Vec<(String, i64)> {
            conn.prepare(
                "SELECT lower(folder), sum(length(CAST(body AS BLOB))) FROM notes WHERE deleted_at IS NULL
                 GROUP BY lower(folder) ORDER BY lower(folder)",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };
        let stored = |conn: &Connection| -> Vec<(String, i64)> {
            conn.prepare("SELECT lower(path), bytes FROM folders ORDER BY lower(path)")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };

        let mut reports = Vec::new();
        migrate(&mut conn, &mut |u| reports.push(u)).unwrap();
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        assert_eq!(stored(&conn), direct(&conn));
        assert!(stored(&conn).iter().all(|(_, b)| *b > 0));

        // Later migrations report too.
        reports.retain(|u| u.step == UpgradeStep::FolderSizes);
        let at = |done| Upgrade { step: UpgradeStep::FolderSizes, done, total: NOTES as usize };
        assert_eq!(reports.first(), Some(&at(0)));
        assert_eq!(reports.last(), Some(&at(NOTES as usize)));
        let batches = (NOTES as usize).div_ceil(SIZES_FILL_BATCH as usize);
        assert_eq!(reports.len(), 1 + batches, "start + one report per batch");
        assert!(reports.windows(2).all(|w| w[0].done < w[1].done));

        conn.execute("UPDATE notes SET body = body || 'ß' WHERE id = 2", []).unwrap();
        conn.execute("UPDATE notes SET deleted_at = NULL WHERE id = 1", []).unwrap();
        conn.execute("UPDATE notes SET folder = 'moved' WHERE id = 3", []).unwrap();
        conn.execute("DELETE FROM notes WHERE id = 4", []).unwrap();
        assert_eq!(stored(&conn), direct(&conn), "the triggers keep it up from here");
    }
    /// A vault from before the list indexes gets both, holding its live notes only.
    #[test]
    fn list_indexes_migration_indexes_the_live_notes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..14] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 14).unwrap();
        conn.execute_batch(
            "INSERT INTO notes(id, folder, title, body, created_at, updated_at) VALUES (1, 'a', 'T', 'one', 1, 5);
             INSERT INTO notes(id, folder, title, body, created_at, updated_at) VALUES (2, 'a', 'U', 'two', 2, 4);
             INSERT INTO notes(id, folder, title, body, created_at, updated_at, deleted_at) VALUES (3, 'a', 'V', 'gone', 3, 6, 9);",
        )
        .unwrap();

        let mut reports = Vec::new();
        migrate(&mut conn, &mut |u| reports.push(u)).unwrap();
        // The later search index migration reports its own step.
        reports.retain(|u| u.step == UpgradeStep::ListIndexes);
        let at = |done| Upgrade { step: UpgradeStep::ListIndexes, done, total: 2 };
        assert_eq!(reports, [at(0), at(1), at(2)]);
        for (index, order, expected) in [("notes_list_folder", "title", [1, 2]), ("notes_list_updated", "updated_at DESC, id DESC", [1, 2])] {
            let sql = format!("SELECT id FROM notes INDEXED BY {index} WHERE deleted_at IS NULL ORDER BY {order}");
            let ids: Vec<i64> = conn.prepare(&sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
            assert_eq!(ids, expected, "{index}");
        }
    }
}
