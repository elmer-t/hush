//! Note CRUD. Every write re-derives the note's index rows (tags, attributes, links,
//! author, topic, created date) from its body.

use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};
use serde::Serialize;

use super::{Error, Result, Vault, links, now_ms, tags};
use crate::markdown;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub folder: String,
    pub title: String,
    pub body: String,
    pub author: Option<String>,
    pub topic: Option<String>,
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
    pub created_at: i64,
    pub updated_at: i64,
}

impl Note {
    pub fn summary(&self) -> NoteSummary {
        NoteSummary {
            id: self.id,
            folder: self.folder.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            topic: self.topic.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
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
pub(super) const SUMMARY_COLUMNS: &str = "n.id, n.folder, n.title, n.author, n.topic, n.created_at, n.updated_at";

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
                "SELECT id, folder, title, body, author, topic, created_at, updated_at FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| {
                    Ok(Note {
                        id: r.get(0)?,
                        folder: r.get(1)?,
                        title: r.get(2)?,
                        body: r.get(3)?,
                        author: r.get(4)?,
                        topic: r.get(5)?,
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
        let len = from.chars().count() as i64;
        let mut moving: Vec<(i64, String, String)> = Vec::new();
        {
            let mut stmt = tx.prepare_cached(
                "SELECT id, folder, title FROM notes
                 WHERE deleted_at IS NULL AND (folder = ?1 OR substr(folder, 1, ?2) = ?3 COLLATE NOCASE)",
            )?;
            let rows = stmt.query_map(params![from, len + 1, format!("{from}/")], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for row in rows {
                let (id, folder, title) = row?;
                let rest: String = folder.chars().skip(len as usize).collect();
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
        let tx = self.conn.transaction()?;
        let mut outcomes = Vec::with_capacity(notes.len());
        for note in notes {
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

    pub fn note_exists(&self, folder: &str, title: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached("SELECT 1 FROM notes WHERE folder = ?1 AND title = ?2 AND deleted_at IS NULL")?;
        Ok(stmt.exists(params![markdown::normalize_folder(folder), title.trim()])?)
    }

    /// Deletes a note for good, live or trashed (`secure_delete` overwrites its text).
    /// The default delete is [`Vault::trash_note`].
    pub fn delete_note(&mut self, id: i64) -> Result<()> {
        if self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])? == 0 {
            return Err(Error::NoteNotFound);
        }
        Ok(())
    }

    /// Notes whose folder is exactly `folder` (`""` for the root), not those of its
    /// subfolders, sorted by title case-insensitively. For the explorer tree, which
    /// loads one folder at a time.
    pub fn folder_notes(&self, folder: &str, offset: u32, limit: u32) -> Result<Vec<NoteSummary>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {SUMMARY_COLUMNS} FROM notes n WHERE n.folder = ?1 AND n.deleted_at IS NULL ORDER BY n.title, n.id LIMIT ?2 OFFSET ?3"
        ))?;
        let notes = stmt
            .query_map(params![markdown::normalize_folder(folder), limit, offset], NoteSummary::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(notes)
    }

    /// Every folder that contains at least one note, sorted.
    pub fn folders(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT folder FROM notes WHERE deleted_at IS NULL ORDER BY folder")?;
        let folders = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(folders)
    }
}

/// Rebuilds the index rows of one note from its body.
pub(super) fn derive(conn: &Connection, id: i64, body: &str) -> Result<()> {
    let p = markdown::parse(body);
    conn.prepare_cached(
        "UPDATE notes SET author = ?2, topic = ?3, created_at = COALESCE(?4, created_at) WHERE id = ?1",
    )?
    .execute(params![id, p.author, p.topic, p.created_at])?;

    for table in ["note_tags", "note_attributes"] {
        conn.prepare_cached(&format!("DELETE FROM {table} WHERE note_id = ?1"))?.execute([id])?;
    }
    conn.prepare_cached("DELETE FROM links WHERE from_note_id = ?1")?.execute([id])?;

    let mut insert_tag = conn.prepare_cached("INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING")?;
    let mut link_tag =
        conn.prepare_cached("INSERT OR IGNORE INTO note_tags(note_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2")?;
    for tag in &p.tags {
        insert_tag.execute([tag])?;
        link_tag.execute(params![id, tag])?;
    }
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
