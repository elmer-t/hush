//! Recently opened notes: a list of note ids, most recent first, kept in the vault's
//! encrypted `settings` table so no title or id ever reaches a plaintext file. Ids, not
//! titles or paths, so a rename or move needs no update.
//!
//! Next to it, the last open note, which the app opens again after an unlock or a
//! restart. It differs from the list's first entry: trashing the open note closes it, so
//! it is forgotten, while the list keeps trashed notes for when they are restored.

use rusqlite::{Connection, params};

use super::notes::NoteSummary;
use super::{Error, Result, Vault};

/// JSON array of note ids, most recently opened first.
pub const RECENT_OPENED_SETTING: &str = "recent_opened";
/// How many notes the list keeps; 0 turns it off.
pub const RECENT_OPENED_COUNT_SETTING: &str = "recent_opened_count";
pub const DEFAULT_RECENT_OPENED_COUNT: u32 = 10;
pub const MAX_RECENT_OPENED_COUNT: u32 = 1000;
/// The id of the note that was open last. Stored only while the list is on (a count
/// above 0): turning history off is a privacy choice, and one remembered note goes
/// against it.
pub const LAST_OPENED_SETTING: &str = "last_opened";

impl Vault {
    /// How many recently opened notes are kept; 0 means none are recorded.
    pub fn recent_opened_count(&self) -> Result<u32> {
        Ok(self
            .setting(RECENT_OPENED_COUNT_SETTING)?
            .and_then(|v| v.parse().ok())
            .filter(|n| *n <= MAX_RECENT_OPENED_COUNT)
            .unwrap_or(DEFAULT_RECENT_OPENED_COUNT))
    }

    /// Sets the count and cuts the stored list down to it at once (0 clears it).
    pub fn set_recent_opened_count(&self, count: u32) -> Result<()> {
        if count > MAX_RECENT_OPENED_COUNT {
            return Err(Error::InvalidRecentOpenedCount);
        }
        self.set_setting(RECENT_OPENED_COUNT_SETTING, &count.to_string())?;
        if count == 0 {
            self.clear_last_opened()?;
        }
        let mut ids = self.recent_opened_ids()?;
        if ids.len() > count as usize {
            ids.truncate(count as usize);
            self.write_recent_opened(&ids)?;
        }
        Ok(())
    }

    /// Moves `id` to the front of the list, keeping each note once and at most the
    /// configured count, and remembers it as the last open note. Opening the note
    /// already at the front writes nothing.
    pub fn record_opened(&self, id: i64) -> Result<()> {
        let count = self.recent_opened_count()? as usize;
        if count == 0 {
            return Ok(());
        }
        let last = id.to_string();
        if self.setting(LAST_OPENED_SETTING)?.as_deref() != Some(last.as_str()) {
            self.set_setting(LAST_OPENED_SETTING, &last)?;
        }
        let mut ids = self.recent_opened_ids()?;
        if ids.first() == Some(&id) && ids.len() <= count {
            return Ok(());
        }
        ids.retain(|&i| i != id);
        ids.insert(0, id);
        ids.truncate(count);
        self.write_recent_opened(&ids)
    }

    pub fn clear_recent_opened(&self) -> Result<()> {
        self.write_recent_opened(&[])
    }

    /// The recently opened notes, most recent first. Trashed notes are left out but
    /// stay in the list, so a restored note shows again in its old place; notes deleted
    /// for good are dropped from it.
    pub fn recent_opened_notes(&self) -> Result<Vec<NoteSummary>> {
        if self.recent_opened_count()? == 0 {
            return Ok(Vec::new());
        }
        let ids = self.recent_opened_ids()?;
        let mut exists = self.conn.prepare_cached("SELECT 1 FROM notes WHERE id = ?1")?;
        let mut kept = Vec::with_capacity(ids.len());
        for &id in &ids {
            if exists.exists([id])? {
                kept.push(id);
            }
        }
        if kept.len() != ids.len() {
            self.write_recent_opened(&kept)?;
        }
        self.note_summaries(&kept)
    }

    /// The last open note, if it is still a live note and history is on. Otherwise the
    /// stored id, if any, is cleared and there is nothing to reopen.
    pub fn last_opened(&self) -> Result<Option<i64>> {
        let Some(value) = self.setting(LAST_OPENED_SETTING)? else {
            return Ok(None);
        };
        let live = match value.parse::<i64>() {
            Ok(id) if self.recent_opened_count()? > 0 => self
                .conn
                .prepare_cached("SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL")?
                .exists([id])?
                .then_some(id),
            _ => None,
        };
        if live.is_none() {
            self.clear_last_opened()?;
        }
        Ok(live)
    }

    pub fn clear_last_opened(&self) -> Result<()> {
        self.delete_setting(LAST_OPENED_SETTING)
    }

    fn recent_opened_ids(&self) -> Result<Vec<i64>> {
        // An unreadable value is treated as an empty list; the next open rewrites it.
        Ok(self
            .setting(RECENT_OPENED_SETTING)?
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }

    fn write_recent_opened(&self, ids: &[i64]) -> Result<()> {
        let json = serde_json::to_string(ids).expect("a list of integers serialises");
        self.set_setting(RECENT_OPENED_SETTING, &json)
    }
}

/// Forgets the last open note if it is `id`; for a trash or delete, inside its transaction.
pub(super) fn forget_last_opened(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM settings WHERE key = ?1 AND value = ?2",
        params![LAST_OPENED_SETTING, id.to_string()],
    )?;
    Ok(())
}

/// Forgets the last open note if it is one of the notes trashed at `deleted_at`.
pub(super) fn forget_last_opened_trashed_at(conn: &Connection, deleted_at: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM settings WHERE key = ?1
           AND value IN (SELECT CAST(id AS TEXT) FROM notes WHERE deleted_at = ?2)",
        params![LAST_OPENED_SETTING, deleted_at],
    )?;
    Ok(())
}
