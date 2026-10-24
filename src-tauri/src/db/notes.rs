//! Note CRUD. Every write re-derives the note's index rows (tags, attributes, links,
//! author, topic, created date) from its body.

use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};
use serde::Serialize;

use super::{Error, Result, Vault, links, now_ms};
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
            "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1 ORDER BY t.name",
        )?;
        note.tags = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(note)
    }

    /// Saves a new body (the autosave path). Title and folder are unchanged, so no
    /// other note's links can change meaning.
    pub fn update_body(&mut self, id: i64, body: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let n = tx.execute(
            "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, body, now_ms()],
        )?;
        if n == 0 {
            return Err(Error::NoteNotFound);
        }
        derive(&tx, id, body)?;
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
            "SELECT {SUMMARY_COLUMNS} FROM notes n WHERE n.folder = ?1 ORDER BY n.title, n.id LIMIT ?2 OFFSET ?3"
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
