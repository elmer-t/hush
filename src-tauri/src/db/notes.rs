//! Note CRUD. Every write re-derives the note's index rows (tags, attributes, links,
//! author, topic, created date) from its body.

use std::time::{Duration, Instant};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, ToSql, params};
use serde::{Deserialize, Serialize};

use super::{Error, Result, Vault, links, now_ms, tags};
use crate::markdown;

/// SQLite's default page cache, in KB.
const DEFAULT_CACHE_KB: i64 = 2_000;
const BULK_CACHE_KB: i64 = 256 * 1024;
/// FTS5's default for how much new index data it buffers before writing it out.
const DEFAULT_FTS_BYTES: i64 = 1024 * 1024;
const BULK_FTS_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub folder: String,
    pub title: String,
    pub body: String,
    pub author: Option<String>,
    pub topic: Option<String>,
    /// 1–5 stars, from frontmatter `rating`.
    pub rating: Option<u8>,
    pub read_status: ReadStatus,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    pub id: i64,
    pub folder: String,
    pub title: String,
    pub author: Option<String>,
    pub topic: Option<String>,
    pub rating: Option<u8>,
    pub read_status: ReadStatus,
    pub created_at: i64,
    pub updated_at: i64,
    /// UTF-8 bytes of the body. Only set where it is cheap: the explorer tree's pages
    /// ([`Vault::folder_notes`]) and a just-read note ([`Note::summary`]); search hits and
    /// other lists leave it out rather than read every body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A folder with live notes directly in it, and the UTF-8 bytes of their bodies. Kept
/// apart from [`Vault::folders`] so a save, which changes sizes, leaves the folder list
/// (and the tree built from it) as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderSize {
    pub path: String,
    pub bytes: u64,
}

/// How far the reader got through a note, marked by hand. Stored in the vault, not in
/// the note's text; notes never marked are unread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadStatus {
    #[default]
    Unread,
    PartlyRead,
    Read,
}

impl ReadStatus {
    /// The value of the `notes.read_status` column.
    pub(super) fn code(self) -> i64 {
        match self {
            Self::Unread => 0,
            Self::PartlyRead => 1,
            Self::Read => 2,
        }
    }
}

impl ToSql for ReadStatus {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.code().into())
    }
}

impl FromSql for ReadStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_i64()? {
            0 => Ok(Self::Unread),
            1 => Ok(Self::PartlyRead),
            2 => Ok(Self::Read),
            n => Err(FromSqlError::OutOfRange(n)),
        }
    }
}

impl Note {
    pub fn summary(&self) -> NoteSummary {
        NoteSummary {
            id: self.id,
            folder: self.folder.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            topic: self.topic.clone(),
            rating: self.rating,
            read_status: self.read_status,
            created_at: self.created_at,
            updated_at: self.updated_at,
            size: Some(self.body.len() as u64),
        }
    }
}

/// A note to import, with its own dates.
#[derive(Debug, Clone)]
pub struct NewNote<'a> {
    pub folder: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Select list matching [`NoteSummary::from_row`], for queries aliasing `notes` as `n`.
pub(super) const SUMMARY_COLUMNS: &str = "n.id, n.folder, n.title, n.author, n.topic, n.created_at, n.updated_at, n.rating, n.read_status";
/// How many columns [`SUMMARY_COLUMNS`] has: a query that selects more after them reads
/// those from this index on.
pub(super) const SUMMARY_COLUMN_COUNT: usize = 9;

impl NoteSummary {
    pub(super) fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            folder: r.get(1)?,
            title: r.get(2)?,
            author: r.get(3)?,
            topic: r.get(4)?,
            created_at: r.get(5)?,
            updated_at: r.get(6)?,
            rating: r.get(7)?,
            read_status: r.get(SUMMARY_COLUMN_COUNT - 1)?,
            size: None,
        })
    }
}

impl Vault {
    pub fn create_note(&mut self, folder: &str, title: &str, body: &str) -> Result<i64> {
        let folder = markdown::normalize_folder(folder);
        let title = validate_title(title)?;
        let tx = self.conn.transaction()?;
        let id = links::preserving_links(&tx, &[title], |conn| {
            let now = now_ms();
            conn.execute(
                "INSERT INTO notes(folder, title, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![folder, title, body, now],
            )
            .map_err(title_taken)?;
            let id = conn.last_insert_rowid();
            derive(conn, id, body)?;
            Ok(id)
        })?;
        tx.commit()?;
        Ok(id)
    }

    /// Creates a note with the first free title of "Untitled", "Untitled 2", … in `folder`.
    pub fn create_untitled(&mut self, folder: &str, body: &str) -> Result<i64> {
        let mut n = 1;
        loop {
            let title = if n == 1 { "Untitled".to_string() } else { format!("Untitled {n}") };
            match self.create_note(folder, &title, body) {
                Err(Error::TitleTaken) => n += 1,
                result => return result,
            }
        }
    }

    pub fn note(&self, id: i64) -> Result<Note> {
        let mut note = self
            .conn
            .query_row(
                "SELECT id, folder, title, body, author, topic, created_at, updated_at, rating, read_status FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| {
                    Ok(Note {
                        id: r.get(0)?,
                        folder: r.get(1)?,
                        title: r.get(2)?,
                        body: r.get(3)?,
                        author: r.get(4)?,
                        topic: r.get(5)?,
                        rating: r.get(8)?,
                        read_status: r.get(9)?,
                        tags: Vec::new(),
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::NoteNotFound)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1 ORDER BY t.name COLLATE NOCASE, t.name",
        )?;
        note.tags = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(note)
    }

    /// Saves a new body (the autosave path). Title and folder are unchanged, so no
    /// other note's links can change meaning.
    pub fn update_body(&mut self, id: i64, body: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let before = tags::note_tag_names(&tx, id)?;
        let now = now_ms();
        let n = tx.execute(
            "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, body, now],
        )?;
        if n == 0 {
            return Err(Error::NoteNotFound);
        }
        derive(&tx, id, body)?;
        tags::record_hand_edits(&tx, id, &before, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Renames and/or moves a note. Links in other notes that pointed at it — or at a
    /// note that would become ambiguous because of it — are rewritten in the same
    /// transaction.
    pub fn rename_note(&mut self, id: i64, folder: &str, title: &str) -> Result<()> {
        let folder = markdown::normalize_folder(folder);
        let title = validate_title(title)?;
        let tx = self.conn.transaction()?;
        let old_title: String = tx
            .query_row("SELECT title FROM notes WHERE id = ?1 AND deleted_at IS NULL", [id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NoteNotFound)?;
        links::preserving_links(&tx, &[&old_title, title], |conn| {
            conn.execute(
                "UPDATE notes SET folder = ?2, title = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, folder, title, now_ms()],
            )
            .map_err(title_taken)?;
            Ok(())
        })?;
        tx.commit()?;
        Ok(())
    }

    /// Moves several notes into `folder` in one transaction, with one link-preserving
    /// pass over all their titles, so links are judged against the state after the
    /// whole batch has moved. All or nothing: if any note would clash on
    /// `(folder, title)` with a note already there or with another note in the batch,
    /// nothing moves and [`Error::MoveClash`] lists the titles. Notes already in
    /// `folder` are left alone.
    pub fn move_notes(&mut self, ids: &[i64], folder: &str) -> Result<()> {
        let folder = markdown::normalize_folder(folder);
        let tx = self.conn.transaction()?;
        let mut moving: Vec<(i64, String)> = Vec::new();
        {
            let mut stmt = tx.prepare_cached("SELECT folder, title FROM notes WHERE id = ?1 AND deleted_at IS NULL")?;
            for &id in ids {
                let (from, title): (String, String) = stmt
                    .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?
                    .ok_or(Error::NoteNotFound)?;
                if !from.eq_ignore_ascii_case(&folder) && !moving.iter().any(|(m, _)| *m == id) {
                    moving.push((id, title));
                }
            }
        }

        // Titles compare case-insensitively, like the unique index.
        let mut clashes: Vec<String> = Vec::new();
        {
            let mut taken = tx.prepare_cached(
                "SELECT 1 FROM notes WHERE folder = ?1 AND title = ?2 AND deleted_at IS NULL",
            )?;
            for (i, (_, title)) in moving.iter().enumerate() {
                let in_batch = moving[..i].iter().any(|(_, t)| t.to_lowercase() == title.to_lowercase());
                if (in_batch || taken.exists(params![folder, title])?)
                    && !clashes.iter().any(|c| c.to_lowercase() == title.to_lowercase())
                {
                    clashes.push(title.clone());
                }
            }
        }
        if !clashes.is_empty() {
            return Err(Error::MoveClash(clashes));
        }
        if moving.is_empty() {
            return Ok(());
        }

        let titles: Vec<&str> = moving.iter().map(|(_, t)| t.as_str()).collect();
        links::preserving_links(&tx, &titles, |conn| {
            let now = now_ms();
            let mut stmt = conn.prepare_cached("UPDATE notes SET folder = ?2, updated_at = ?3 WHERE id = ?1")?;
            for (id, _) in &moving {
                stmt.execute(params![id, folder, now]).map_err(title_taken)?;
            }
            Ok(())
        })?;
        tx.commit()?;
        Ok(())
    }

    /// Moves folder `from`, with every note in it and its subfolders, into folder `to`
    /// (`""` for the top level), keeping the structure below it: `a/b` into `c` puts
    /// `a/b/x` notes in `c/b/x`. Returns the folder's new path. One transaction with one
    /// link-preserving pass, like [`Vault::move_notes`], and all or nothing: if any note
    /// would clash with a note already at its new path, nothing moves and
    /// [`Error::MoveClash`] lists the titles. Moving a folder into itself or one of its
    /// subfolders is [`Error::InvalidFolderMove`]; into its current parent, a no-op.
    pub fn move_folder(&mut self, from: &str, to: &str) -> Result<String> {
        let from = markdown::normalize_folder(from);
        let to = markdown::normalize_folder(to);
        let Some(name) = from.rsplit('/').next().filter(|s| !s.is_empty()) else {
            return Err(Error::InvalidFolderMove("the top level can't be moved".into()));
        };
        // Folder paths compare case-insensitively, like the `folder` column.
        let (from_key, to_key) = (from.to_lowercase(), to.to_lowercase());
        if to_key == from_key || to_key.starts_with(&format!("{from_key}/")) {
            return Err(Error::InvalidFolderMove("a folder can't be moved into itself".into()));
        }
        let parent = from.rsplit_once('/').map_or("", |(p, _)| p);
        if parent.eq_ignore_ascii_case(&to) {
            return Ok(from);
        }
        let dest = if to.is_empty() { name.to_string() } else { format!("{to}/{name}") };

        let tx = self.conn.transaction()?;
        // Every live note in the folder or below it, with its new folder: `dest` plus the
        // rest of its path below `from`, in the case it is stored with.
        let len = from.chars().count();
        let mut moving: Vec<(i64, String, String)> = Vec::new();
        {
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT id, folder, title FROM notes WHERE deleted_at IS NULL AND {IN_SUBTREE}"
            ))?;
            let (a, b, c) = subtree_params(&from);
            let rows = stmt.query_map(params![a, b, c], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for row in rows {
                let (id, folder, title) = row?;
                let rest: String = folder.chars().skip(len).collect();
                moving.push((id, format!("{dest}{rest}"), title));
            }
        }

        // Moved notes keep their paths below the folder, so they can't clash with each
        // other, only with a note that stays put. (Moving `b/b` to the top level makes it
        // `b`: a moved note may take the old path of another moved note.)
        let mut clashes: Vec<String> = Vec::new();
        {
            let mut taken = tx.prepare_cached(
                "SELECT id FROM notes WHERE folder = ?1 AND title = ?2 AND deleted_at IS NULL",
            )?;
            for (_, folder, title) in &moving {
                let there: Option<i64> = taken.query_row(params![folder, title], |r| r.get(0)).optional()?;
                if there.is_some_and(|id| !moving.iter().any(|(m, _, _)| *m == id))
                    && !clashes.iter().any(|c| c.to_lowercase() == title.to_lowercase())
                {
                    clashes.push(title.clone());
                }
            }
        }
        if !clashes.is_empty() {
            return Err(Error::MoveClash(clashes));
        }
        if moving.is_empty() {
            return Ok(dest);
        }

        // Shallowest first, so a note taking another moved note's old path moves in only
        // after that note has moved out, and the unique index holds throughout.
        moving.sort_by_key(|(_, folder, _)| folder.matches('/').count());
        let titles: Vec<&str> = moving.iter().map(|(_, _, t)| t.as_str()).collect();
        links::preserving_links(&tx, &titles, |conn| {
            let now = now_ms();
            let mut stmt = conn.prepare_cached("UPDATE notes SET folder = ?2, updated_at = ?3 WHERE id = ?1")?;
            for (id, folder, _) in &moving {
                stmt.execute(params![id, folder, now]).map_err(title_taken)?;
            }
            Ok(())
        })?;
        tx.commit()?;
        Ok(dest)
    }

    /// Inserts imported notes in one transaction, returning each note's id or why it
    /// was not inserted. Unlike [`Vault::create_note`], links in other notes are not
    /// rewritten: an import reports ambiguous links instead.
    pub fn import_notes(&mut self, notes: &[NewNote]) -> Result<Vec<Result<i64>>> {
        self.import_notes_until(notes, None)
    }

    /// Like [`Vault::import_notes`], but stops inserting once `budget` has passed, so a big
    /// batch never holds the vault much longer than that. Returns the outcomes of the
    /// notes it got to (at least one); the rest are for the next call.
    pub fn import_notes_within(&mut self, notes: &[NewNote], budget: Duration) -> Result<Vec<Result<i64>>> {
        self.import_notes_until(notes, Some(Instant::now() + budget))
    }

    fn import_notes_until(&mut self, notes: &[NewNote], deadline: Option<Instant>) -> Result<Vec<Result<i64>>> {
        let tx = self.conn.transaction()?;
        let mut outcomes = Vec::with_capacity(notes.len());
        for note in notes {
            if !outcomes.is_empty() && deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            let outcome = (|| {
                let title = validate_title(note.title)?;
                tx.prepare_cached(
                    "INSERT INTO notes(folder, title, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                )?
                .execute(params![markdown::normalize_folder(note.folder), title, note.body, note.created_at, note.updated_at])
                .map_err(title_taken)?;
                let id = tx.last_insert_rowid();
                derive(&tx, id, note.body)?;
                Ok(id)
            })();
            match outcome {
                Err(Error::Sql(e)) => return Err(Error::Sql(e)),
                outcome => outcomes.push(outcome),
            }
        }
        tx.commit()?;
        Ok(outcomes)
    }

    /// Settings for writing many notes at once, as an import does: a 256 MB page cache
    /// and a 64 MB full-text buffer, which halve the write time into a large vault.
    /// `false` puts back the defaults. The buffer size is stored in the vault, so
    /// opening a vault also puts it back after a crash mid-import.
    pub fn set_bulk_writes(&self, on: bool) -> Result<()> {
        let (cache_kb, fts_bytes) = if on { (BULK_CACHE_KB, BULK_FTS_BYTES) } else { (DEFAULT_CACHE_KB, DEFAULT_FTS_BYTES) };
        self.conn.pragma_update(None, "cache_size", -cache_kb)?;
        set_fts_hashsize(&self.conn, fts_bytes)
    }

    pub fn note_exists(&self, folder: &str, title: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached("SELECT 1 FROM notes WHERE folder = ?1 AND title = ?2 AND deleted_at IS NULL")?;
        Ok(stmt.exists(params![markdown::normalize_folder(folder), title.trim()])?)
    }

    /// Deletes a note for good, live or trashed (`secure_delete` overwrites its text).
    /// The default delete is [`Vault::trash_note`].
    pub fn delete_note(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        if tx.execute("DELETE FROM notes WHERE id = ?1", [id])? == 0 {
            return Err(Error::NoteNotFound);
        }
        super::trash::forget_empty_groups(&tx)?;
        super::recent::forget_last_opened(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    /// Notes whose folder is exactly `folder` (`""` for the root), not those of its
    /// subfolders, sorted by title case-insensitively. For the explorer tree, which
    /// loads one folder at a time.
    pub fn folder_notes(&self, folder: &str, offset: u32, limit: u32) -> Result<Vec<NoteSummary>> {
        // The size is measured here, one page at a time, rather than stored.
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {SUMMARY_COLUMNS}, octet_length(n.body) FROM notes n WHERE n.folder = ?1 AND n.deleted_at IS NULL ORDER BY n.title, n.id LIMIT ?2 OFFSET ?3"
        ))?;
        let notes = stmt
            .query_map(params![markdown::normalize_folder(folder), limit, offset], |r| {
                Ok(NoteSummary { size: Some(r.get::<_, i64>(SUMMARY_COLUMN_COUNT)? as u64), ..NoteSummary::from_row(r)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(notes)
    }

    /// Summaries of the live notes among `ids`, in the order given; trashed or deleted
    /// ones are left out.
    pub fn note_summaries(&self, ids: &[i64]) -> Result<Vec<NoteSummary>> {
        let mut stmt =
            self.conn.prepare_cached(&format!("SELECT {SUMMARY_COLUMNS} FROM notes n WHERE n.id = ?1 AND n.deleted_at IS NULL"))?;
        let mut notes = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(note) = stmt.query_row([id], NoteSummary::from_row).optional()? {
                notes.push(note);
            }
        }
        Ok(notes)
    }

    /// Marks live notes among `ids` as `status`; returns those whose status changed, in
    /// the order given. Only the status column is written: the body, the dates, the
    /// full-text index and the embedding queue stay as they are.
    pub fn set_read_status(&mut self, ids: &[i64], status: ReadStatus) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let mut changed = Vec::new();
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE notes SET read_status = ?2 WHERE id = ?1 AND deleted_at IS NULL AND read_status <> ?2",
            )?;
            for &id in ids {
                if stmt.execute(params![id, status])? > 0 {
                    changed.push(id);
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Every folder that contains at least one note, sorted.
    pub fn folders(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(FOLDERS_QUERY)?;
        let folders = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(folders)
    }

    /// The same folders as [`Vault::folders`], in the same order, each with the bytes of
    /// the live notes directly in it (not its subfolders; the tree adds those up).
    pub fn folder_sizes(&self) -> Result<Vec<FolderSize>> {
        let mut stmt = self.conn.prepare_cached(FOLDER_SIZES_QUERY)?;
        let sizes = stmt
            .query_map([], |r| Ok(FolderSize { path: r.get(0)?, bytes: r.get::<_, i64>(1)?.max(0) as u64 }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(sizes)
    }
}

/// Lists folders from the `folders` table, which the triggers keep in step with the live
/// notes, so the cost follows the number of folders rather than notes.
const FOLDERS_QUERY: &str = "SELECT path FROM folders ORDER BY path";
/// [`FOLDERS_QUERY`] with the stored byte totals, also from the table alone.
const FOLDER_SIZES_QUERY: &str = "SELECT path, bytes FROM folders ORDER BY path";

/// Matches folder `?1` and every folder below it, case-insensitively like the column, as
/// one range on the `folder` index rather than a scan: `?2` is `?1/` and `?3` is `?1`
/// followed by `0`, the character after `/`. (Written as `folder = ?1 OR folder > ?2 AND
/// folder < ?3`, SQLite scans the whole index.) Bind with [`subtree_params`].
pub(super) const IN_SUBTREE: &str = "(folder >= ?1 AND folder < ?3 AND (folder = ?1 OR folder > ?2))";

pub(super) fn subtree_params(folder: &str) -> (String, String, String) {
    (folder.to_string(), format!("{folder}/"), format!("{folder}0"))
}

/// Rebuilds the index rows of one note from its body.
pub(super) fn derive(conn: &Connection, id: i64, body: &str) -> Result<()> {
    let p = markdown::parse(body);
    conn.prepare_cached(
        "UPDATE notes SET author = ?2, topic = ?3, rating = ?5, created_at = COALESCE(?4, created_at) WHERE id = ?1",
    )?
    .execute(params![id, p.author, p.topic, p.created_at, p.rating])?;

    set_note_tags(conn, id, &p.tags)?;
    conn.prepare_cached("DELETE FROM note_attributes WHERE note_id = ?1")?.execute([id])?;
    conn.prepare_cached("DELETE FROM links WHERE from_note_id = ?1")?.execute([id])?;

    let mut insert_attr = conn.prepare_cached("INSERT INTO note_attributes(note_id, key, value) VALUES (?1, ?2, ?3)")?;
    for (key, value) in &p.attributes {
        insert_attr.execute(params![id, key, value])?;
    }
    let mut insert_link =
        conn.prepare_cached("INSERT INTO links(from_note_id, target_path, target_title) VALUES (?1, ?2, ?3)")?;
    for link in &p.links {
        insert_link.execute(params![id, link.path, link.title])?;
    }
    Ok(())
}

/// Replaces the tag rows of note `id` with `tags`.
pub(super) fn set_note_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<()> {
    conn.prepare_cached("DELETE FROM note_tags WHERE note_id = ?1")?.execute([id])?;
    let mut insert_tag = conn.prepare_cached("INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING")?;
    let mut link_tag =
        conn.prepare_cached("INSERT OR IGNORE INTO note_tags(note_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2")?;
    for tag in tags {
        insert_tag.execute([tag])?;
        link_tag.execute(params![id, tag])?;
    }
    Ok(())
}

/// Titles can't contain characters that would break `[[wikilinks]]` or folder paths.
fn validate_title(title: &str) -> Result<&str> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::InvalidTitle("title is empty".into()));
    }
    if let Some(c) = title.chars().find(|c| matches!(c, '/' | '\\' | '[' | ']' | '|' | '#') || c.is_control()) {
        return Err(Error::InvalidTitle(format!("title must not contain '{}'", c.escape_default())));
    }
    Ok(title)
}

pub(super) fn title_taken(e: rusqlite::Error) -> Error {
    if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
        Error::TitleTaken
    } else {
        e.into()
    }
}

fn set_fts_hashsize(conn: &Connection, bytes: i64) -> Result<()> {
    conn.execute("INSERT INTO notes_fts(notes_fts, rank) VALUES ('hashsize', ?1)", [bytes])?;
    Ok(())
}

/// Puts back the full-text buffer size an import left behind (it crashed, or the vault
/// locked mid-import). Writes nothing when it is already the default.
pub(super) fn reset_bulk_writes(conn: &Connection) -> Result<()> {
    let stored: Option<i64> =
        conn.query_row("SELECT v FROM notes_fts_config WHERE k = 'hashsize'", [], |r| r.get(0)).optional()?;
    if stored.is_some_and(|b| b != DEFAULT_FTS_BYTES) {
        set_fts_hashsize(conn, DEFAULT_FTS_BYTES)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_hashsize(vault: &Vault) -> Option<i64> {
        vault.conn.query_row("SELECT v FROM notes_fts_config WHERE k = 'hashsize'", [], |r| r.get(0)).optional().unwrap()
    }

    /// Marking notes read changes only their read status: not the body, the dates, the
    /// tags, the full-text index or the embedding queue. Trashed notes are left alone.
    #[test]
    fn set_read_status_touches_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let a = vault.create_note("f", "A", "---\ntags: [x]\n---\nlighthouse #y").unwrap();
        let b = vault.create_note("f", "B", "keeper").unwrap();
        let gone = vault.create_note("f", "Gone", "old").unwrap();
        vault.trash_note(gone).unwrap();
        vault.conn.execute("UPDATE embedding_dirty SET version = 7", []).unwrap();
        let state = |v: &Vault| -> Vec<(i64, String, i64, i64, String)> {
            v.conn
                .prepare(
                    "SELECT n.id, n.body, n.created_at, n.updated_at,
                         (SELECT group_concat(t.name) FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = n.id)
                         || '|' || coalesce((SELECT version FROM embedding_dirty WHERE note_id = n.id), -1)
                     FROM notes n ORDER BY n.id",
                )
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<String>>(4)?.unwrap_or_default())))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        let before = state(&vault);

        let changed = vault.set_read_status(&[a, b, gone, 999], ReadStatus::Read).unwrap();
        assert_eq!(changed, [a, b], "trashed and missing notes are skipped");
        assert_eq!(vault.set_read_status(&[a], ReadStatus::Read).unwrap(), Vec::<i64>::new(), "already read");
        vault.set_read_status(&[b], ReadStatus::PartlyRead).unwrap();
        assert_eq!(state(&vault), before);
        let found: i64 =
            vault.conn.query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'lighthouse'", [], |r| r.get(0)).unwrap();
        assert_eq!(found, 1);
        assert_eq!(vault.note(a).unwrap().read_status, ReadStatus::Read);
        assert_eq!(vault.note_summaries(&[a, b]).unwrap().iter().map(|n| n.read_status).collect::<Vec<_>>(), [
            ReadStatus::Read,
            ReadStatus::PartlyRead
        ]);
        let trashed: i64 = vault.conn.query_row("SELECT read_status FROM notes WHERE id = ?1", [gone], |r| r.get(0)).unwrap();
        assert_eq!(trashed, 0);

        // A save keeps the status: it is about the reader, not the text.
        vault.update_body(a, "changed").unwrap();
        assert_eq!(vault.note(a).unwrap().read_status, ReadStatus::Read);
        let new = vault.create_note("f", "New", "").unwrap();
        assert_eq!(vault.note(new).unwrap().read_status, ReadStatus::Unread);
    }

    #[test]
    fn a_spent_budget_still_writes_one_note_per_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let titles = ["a", "b", "c"];
        let notes: Vec<NewNote> =
            titles.iter().map(|t| NewNote { folder: "", title: t, body: "x", created_at: 1, updated_at: 1 }).collect();
        let mut written = 0;
        let mut calls = 0;
        while written < notes.len() {
            written += vault.import_notes_within(&notes[written..], Duration::ZERO).unwrap().len();
            calls += 1;
        }
        assert_eq!(calls, 3);
        assert_eq!(vault.import_notes_within(&notes, Duration::from_secs(60)).unwrap().len(), 3);
    }

    #[test]
    fn bulk_write_settings_are_undone_and_reset_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        let vault = Vault::create(&path, "test passphrase").unwrap();
        vault.set_bulk_writes(true).unwrap();
        assert_eq!(stored_hashsize(&vault), Some(BULK_FTS_BYTES));
        let cache: i64 = vault.conn.query_row("PRAGMA cache_size", [], |r| r.get(0)).unwrap();
        assert_eq!(cache, -BULK_CACHE_KB);
        vault.set_bulk_writes(false).unwrap();
        assert_eq!(stored_hashsize(&vault), Some(DEFAULT_FTS_BYTES));

        // An import that never finished (crash, or the vault locked) leaves the big buffer.
        vault.set_bulk_writes(true).unwrap();
        vault.close().unwrap();
        let vault = Vault::open(&path, "test passphrase").unwrap();
        assert_eq!(stored_hashsize(&vault), Some(DEFAULT_FTS_BYTES));
    }

    /// The `folders` table as `(lowercased path, notes)`, and every stored spelling.
    fn folder_rows(v: &Vault) -> (Vec<(String, i64)>, Vec<String>) {
        let rows: Vec<(String, i64)> = v
            .conn
            .prepare("SELECT path, notes FROM folders ORDER BY path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let spellings = rows.iter().map(|(p, _)| p.clone()).collect();
        (rows.into_iter().map(|(p, n)| (p.to_lowercase(), n)).collect(), spellings)
    }

    /// Asserts the `folders` table holds exactly the live notes per folder, recounted from
    /// `notes`, and that each stored spelling is one a live note uses.
    fn assert_folders_match(v: &Vault, step: &str) {
        let expected: Vec<(String, i64)> = v
            .conn
            .prepare("SELECT lower(folder), count(*) FROM notes WHERE deleted_at IS NULL GROUP BY lower(folder) ORDER BY lower(folder)")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let (rows, spellings) = folder_rows(v);
        assert_eq!(rows, expected, "after {step}");
        for path in spellings {
            let used: bool = v
                .conn
                .prepare("SELECT 1 FROM notes WHERE folder = ?1 COLLATE BINARY AND deleted_at IS NULL")
                .unwrap()
                .exists([&path])
                .unwrap();
            assert!(used, "after {step}: stored spelling {path:?} is not used by a live note");
        }
        let listed = v.folders().unwrap();
        let distinct: Vec<String> = expected.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(listed.iter().map(|f| f.to_lowercase()).collect::<Vec<_>>(), distinct, "folders() after {step}");
        assert_sizes_match(v, step);
    }

    /// Asserts `folder_sizes()` gives each folder the UTF-8 byte length of its live notes'
    /// bodies, summed here in Rust from the bodies rather than by the triggers' SQL.
    fn assert_sizes_match(v: &Vault, step: &str) {
        let mut expected = std::collections::BTreeMap::<String, u64>::new();
        let bodies: Vec<(String, String)> = v
            .conn
            .prepare("SELECT folder, body FROM notes WHERE deleted_at IS NULL")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for (folder, body) in bodies {
            *expected.entry(folder.to_lowercase()).or_default() += body.len() as u64;
        }
        let sizes: Vec<(String, u64)> = v.folder_sizes().unwrap().into_iter().map(|s| (s.path.to_lowercase(), s.bytes)).collect();
        assert_eq!(sizes, expected.into_iter().collect::<Vec<_>>(), "folder_sizes() after {step}");
        let listed: Vec<String> = v.folder_sizes().unwrap().into_iter().map(|s| s.path).collect();
        assert_eq!(listed, v.folders().unwrap(), "folder_sizes() lists folders() in its order and spelling, after {step}");
    }

    /// Every way a note enters, leaves or changes folder keeps the `folders` table's
    /// counts right: a folder's row goes when its last live note leaves, and trashed
    /// notes don't count.
    #[test]
    fn folders_table_follows_every_write_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        assert_folders_match(&v, "create vault");

        let root = v.create_note("", "Root", "root body").unwrap();
        let a1 = v.create_note("a", "One", "één — ünïcode").unwrap();
        let a2 = v.create_note("a", "Two", "x").unwrap();
        let ab = v.create_note("a/b", "Deep", &"deep ".repeat(300)).unwrap();
        let untitled = v.create_untitled("c", "🙂 four-byte").unwrap();
        assert_folders_match(&v, "create");
        assert_eq!(folder_rows(&v).0, [("".into(), 1), ("a".into(), 2), ("a/b".into(), 1), ("c".into(), 1)]);

        let imported = v
            .import_notes(&[
                NewNote { folder: "d", title: "I1", body: "imported", created_at: 1, updated_at: 1 },
                NewNote { folder: "d/e", title: "I2", body: "imported ±", created_at: 1, updated_at: 1 },
                NewNote { folder: "a", title: "One", body: "taken", created_at: 1, updated_at: 1 },
            ])
            .unwrap();
        assert!(imported[2].is_err(), "a clashing title isn't inserted");
        assert_folders_match(&v, "import");

        v.move_notes(&[a1], "c").unwrap();
        assert_folders_match(&v, "move note");
        v.rename_note(untitled, "f", "Renamed").unwrap();
        assert_folders_match(&v, "rename into another folder");
        v.update_body(a2, &"grown ".repeat(1_000)).unwrap();
        assert_folders_match(&v, "edit that grows");
        v.update_body(a2, "ß").unwrap();
        assert_folders_match(&v, "edit that shrinks");
        v.update_body(a2, "").unwrap();
        assert_folders_match(&v, "edit to empty");

        v.move_folder("d", "c").unwrap();
        assert_folders_match(&v, "move folder into an existing one");
        v.move_folder("a", "g").unwrap();
        assert_folders_match(&v, "move folder");

        v.trash_note(root).unwrap();
        assert_folders_match(&v, "trash note");
        assert!(folder_rows(&v).0.iter().all(|(p, _)| !p.is_empty()), "the root's last note left");
        v.restore_note(root).unwrap();
        assert_folders_match(&v, "restore note");

        v.trash_folder("g").unwrap();
        assert_folders_match(&v, "trash folder");
        let group = v
            .trash_entries(0, 10)
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                crate::db::TrashEntry::Folder { deleted_at, .. } => Some(deleted_at),
                _ => None,
            })
            .unwrap();
        v.restore_folder(group).unwrap();
        assert_folders_match(&v, "restore folder");

        v.trash_folder("g").unwrap();
        let group = v
            .trash_entries(0, 10)
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                crate::db::TrashEntry::Folder { deleted_at, .. } => Some(deleted_at),
                _ => None,
            })
            .unwrap();
        v.delete_trash_folder(group).unwrap();
        assert_folders_match(&v, "delete trashed folder");

        v.trash_note(a1).unwrap();
        v.empty_trash().unwrap();
        assert_folders_match(&v, "empty trash");
        let live = v.create_note("h", "Live", "x").unwrap();
        v.delete_note(live).unwrap();
        assert_folders_match(&v, "delete note");
        let _ = ab;
    }

    /// `Projects` and `projects` are one folder with one row; when the spelling the row
    /// shows is no longer used, it takes a spelling a remaining note uses.
    #[test]
    fn case_variant_folders_share_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let upper = v.create_note("Projects", "A", "x").unwrap();
        v.create_note("projects", "B", "x").unwrap();
        assert_eq!(folder_rows(&v).0, [("projects".into(), 2)]);
        assert_eq!(v.folders().unwrap(), ["Projects"], "the first spelling stays");
        v.trash_note(upper).unwrap();
        assert_eq!(v.folders().unwrap(), ["projects"], "the remaining note's spelling");
        assert_folders_match(&v, "trash the shown spelling");
    }

    /// Random creates, moves, renames, trashes, restores and deletes over a few folders
    /// (with case variants) never let the table drift from a recount.
    #[test]
    fn folders_table_survives_random_operations() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        const FOLDERS: [&str; 7] = ["", "a", "A", "a/b", "A/B", "c", "c/d/e"];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let ids = |v: &Vault, live: bool| -> Vec<i64> {
            let sql = if live { "SELECT id FROM notes WHERE deleted_at IS NULL" } else { "SELECT id FROM notes WHERE deleted_at IS NOT NULL" };
            v.conn.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        for step in 0..400 {
            let live = ids(&v, true);
            let trashed = ids(&v, false);
            let op = next(9);
            let folder = FOLDERS[next(FOLDERS.len())];
            let _ = match op {
                0 | 1 => v.create_note(folder, &format!("N{step}"), &"x".repeat(next(40))).map(|_| ()),
                2 if !live.is_empty() => v.move_notes(&[live[next(live.len())]], folder),
                3 if !live.is_empty() => v.trash_note(live[next(live.len())]),
                4 if !trashed.is_empty() => v.restore_note(trashed[next(trashed.len())]).map(|_| ()),
                5 if !live.is_empty() => v.delete_note(live[next(live.len())]),
                6 if !folder.is_empty() => v.trash_folder(folder).map(|_| ()),
                7 if !folder.is_empty() => v.move_folder(folder, FOLDERS[next(FOLDERS.len())]).map(|_| ()),
                8 if !live.is_empty() => {
                    let body = "é".repeat(next(50));
                    v.update_body(live[next(live.len())], &body)
                }
                _ => Ok(()),
            };
            assert_folders_match(&v, &format!("step {step} op {op} {folder:?}"));
        }
    }

    /// Listing folders reads the `folders` table, not the notes or their index.
    #[test]
    fn folders_reads_the_folders_table() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let plan: Vec<String> = v
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {FOLDERS_QUERY}"))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let plan = plan.join("\n");
        assert!(plan.contains("folders") && !plan.contains("notes"), "{plan}");
    }

    /// Folder sizes read the `folders` table too, never the note bodies.
    #[test]
    fn folder_sizes_read_the_folders_table() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let plan: Vec<String> = v
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {FOLDER_SIZES_QUERY}"))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let plan = plan.join("\n");
        assert!(plan.contains("folders") && !plan.contains("notes"), "{plan}");
    }

    /// A folder's size is the UTF-8 bytes of its live notes' bodies; edits that grow or
    /// shrink a note, and changing only the case of its folder, keep it exact.
    #[test]
    fn folder_sizes_count_utf8_bytes_of_live_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let sizes = |v: &Vault| v.folder_sizes().unwrap().into_iter().map(|s| (s.path, s.bytes)).collect::<Vec<_>>();
        let a = v.create_note("Docs", "A", "héllo").unwrap(); // 6 bytes
        let b = v.create_note("Docs", "B", "🙂").unwrap(); // 4 bytes
        assert_eq!(sizes(&v), [("Docs".to_string(), 10)]);
        v.update_body(a, &"x".repeat(2_000)).unwrap();
        assert_eq!(sizes(&v), [("Docs".to_string(), 2_004)]);
        v.update_body(a, "").unwrap();
        assert_eq!(sizes(&v), [("Docs".to_string(), 4)]);
        v.move_notes(&[a, b], "docs").unwrap();
        let lower = |v: &Vault| sizes(v).into_iter().map(|(p, n)| (p.to_lowercase(), n)).collect::<Vec<_>>();
        assert_eq!(lower(&v), [("docs".to_string(), 4)], "only the case changed");
        v.move_folder("docs", "Archive").unwrap();
        assert_eq!(sizes(&v), [("Archive/docs".to_string(), 4)]);
        v.trash_note(b).unwrap();
        assert_eq!(sizes(&v), [("Archive/docs".to_string(), 0)], "a trashed note isn't counted");
        v.trash_note(a).unwrap();
        assert_eq!(sizes(&v), [], "the folder's row goes with its last note");
        v.restore_note(b).unwrap();
        assert_eq!(sizes(&v), [("Archive/docs".to_string(), 4)]);
    }

    /// Notes listed for the tree carry their size; other summaries don't read the body.
    #[test]
    fn folder_notes_carry_each_notes_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let id = v.create_note("a", "A", "naïve").unwrap();
        v.create_note("a", "B", "").unwrap();
        let listed = v.folder_notes("a", 0, 10).unwrap();
        assert_eq!(listed.iter().map(|n| n.size).collect::<Vec<_>>(), [Some(6), Some(0)]);
        assert_eq!(v.note(id).unwrap().summary().size, Some(6), "a saved note's summary has its size");
        assert_eq!(v.note_summaries(&[id]).unwrap()[0].size, None);
    }
}
