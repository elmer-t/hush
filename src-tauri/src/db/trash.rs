//! The trash: deleting a note moves it here, from where it can be restored. Trashed
//! notes keep their row but lose their tag, attribute and link rows, and every other
//! query skips them, so they vanish from lists, search, filters and links. Deleting
//! from the trash is a real delete (`secure_delete` is on).

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::notes::{derive, title_taken};
use super::{Error, Result, Vault, links, now_ms};

/// How many days a note stays in the trash before it is deleted on unlock; 0 = never.
pub const TRASH_DAYS_SETTING: &str = "trash_days";
pub const DEFAULT_TRASH_DAYS: u32 = 30;
pub const MAX_TRASH_DAYS: u32 = 3650;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedNote {
    pub id: i64,
    /// The folder it was deleted from, and is restored to.
    pub folder: String,
    pub title: String,
    pub deleted_at: i64,
}

impl Vault {
    /// Moves a note to the trash.
    pub fn trash_note(&mut self, id: i64) -> Result<()> {
        self.trash_notes(&[id])
    }

    /// Moves several notes to the trash in one transaction: all or nothing, so an
    /// unknown (or already trashed) id changes nothing. The single entry point for
    /// bulk delete.
    pub fn trash_notes(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let now = now_ms();
        for &id in ids {
            if tx.execute("UPDATE notes SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, now])?
                == 0
            {
                return Err(Error::NoteNotFound);
            }
            for table in ["note_tags", "note_attributes"] {
                tx.execute(&format!("DELETE FROM {table} WHERE note_id = ?1"), [id])?;
            }
            tx.execute("DELETE FROM links WHERE from_note_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Trashed notes, most recently deleted first.
    pub fn trashed_notes(&self) -> Result<Vec<TrashedNote>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, folder, title, deleted_at FROM notes WHERE deleted_at IS NOT NULL
             ORDER BY deleted_at DESC, id DESC",
        )?;
        let notes = stmt
            .query_map([], |r| Ok(TrashedNote { id: r.get(0)?, folder: r.get(1)?, title: r.get(2)?, deleted_at: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(notes)
    }

    /// Restores a trashed note to its folder. If a live note has taken its folder and
    /// title meanwhile, it comes back as "Title (restored)" (or "(restored 2)", …).
    /// Like creating a note, links that resolved uniquely before keep their meaning.
    /// Returns the title it was restored under.
    pub fn restore_note(&mut self, id: i64) -> Result<String> {
        let tx = self.conn.transaction()?;
        let (title, body): (String, String) = tx
            .query_row("SELECT title, body FROM notes WHERE id = ?1 AND deleted_at IS NOT NULL", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .ok_or(Error::NoteNotFound)?;
        let mut n = 1;
        let restored = loop {
            let candidate = match n {
                1 => title.clone(),
                2 => format!("{title} (restored)"),
                n => format!("{title} (restored {})", n - 1),
            };
            let outcome = links::preserving_links(&tx, &[&candidate], |conn| {
                conn.execute(
                    "UPDATE notes SET title = ?2, deleted_at = NULL WHERE id = ?1",
                    params![id, candidate],
                )
                .map_err(title_taken)?;
                derive(conn, id, &body)
            });
            match outcome {
                Ok(()) => break candidate,
                Err(Error::TitleTaken) => n += 1,
                Err(e) => return Err(e),
            }
        };
        tx.commit()?;
        Ok(restored)
    }

    /// Deletes every trashed note for good. Returns how many.
    pub fn empty_trash(&mut self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM notes WHERE deleted_at IS NOT NULL", [])?)
    }

    /// The trash retention in days; 0 means trashed notes are kept until deleted by hand.
    pub fn trash_days(&self) -> Result<u32> {
        Ok(self
            .setting(TRASH_DAYS_SETTING)?
            .and_then(|v| v.parse().ok())
            .filter(|d| *d <= MAX_TRASH_DAYS)
            .unwrap_or(DEFAULT_TRASH_DAYS))
    }

    pub fn set_trash_days(&self, days: u32) -> Result<()> {
        if days > MAX_TRASH_DAYS {
            return Err(Error::InvalidTrashDays);
        }
        self.set_setting(TRASH_DAYS_SETTING, &days.to_string())
    }

    /// Deletes trashed notes older than the retention period at `now` (Unix millis).
    /// Run on unlock. Returns how many were deleted.
    pub fn purge_trash_at(&mut self, now: i64) -> Result<usize> {
        let days = self.trash_days()?;
        if days == 0 {
            return Ok(0);
        }
        let cutoff = now - i64::from(days) * DAY_MS;
        Ok(self.conn.execute("DELETE FROM notes WHERE deleted_at IS NOT NULL AND deleted_at <= ?1", [cutoff])?)
    }

    pub fn purge_trash(&mut self) -> Result<usize> {
        self.purge_trash_at(now_ms())
    }

    /// Pretends a trashed note was deleted `days` ago.
    #[cfg(test)]
    pub(crate) fn backdate_trash(&self, id: i64, days: i64) -> Result<()> {
        self.conn.execute("UPDATE notes SET deleted_at = deleted_at - ?2 WHERE id = ?1", params![id, days * DAY_MS])?;
        Ok(())
    }
}
