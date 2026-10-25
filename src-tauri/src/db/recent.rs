//! Recently opened notes: a list of note ids, most recent first, kept in the vault's
//! encrypted `settings` table so no title or id ever reaches a plaintext file. Ids, not
//! titles or paths, so a rename or move needs no update.

use super::notes::NoteSummary;
use super::{Error, Result, Vault};

/// JSON array of note ids, most recently opened first.
pub const RECENT_OPENED_SETTING: &str = "recent_opened";
/// How many notes the list keeps; 0 turns it off.
pub const RECENT_OPENED_COUNT_SETTING: &str = "recent_opened_count";
pub const DEFAULT_RECENT_OPENED_COUNT: u32 = 10;
pub const MAX_RECENT_OPENED_COUNT: u32 = 50;

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
        let mut ids = self.recent_opened_ids()?;
        if ids.len() > count as usize {
            ids.truncate(count as usize);
            self.write_recent_opened(&ids)?;
        }
        Ok(())
    }

    /// Moves `id` to the front of the list, keeping each note once and at most the
    /// configured count. Opening the note already at the front writes nothing.
    pub fn record_opened(&self, id: i64) -> Result<()> {
        let count = self.recent_opened_count()? as usize;
        let mut ids = self.recent_opened_ids()?;
        if count == 0 || (ids.first() == Some(&id) && ids.len() <= count) {
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
