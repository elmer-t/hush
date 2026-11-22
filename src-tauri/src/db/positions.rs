//! Where the user was in each recently opened note: the selection and the text at the top
//! of the editor, so a note opens again where it was left (#242).
//!
//! Which notes you read, and where in them, is vault data: the positions live in the
//! encrypted `settings` table, next to the recently opened list, and never in a plaintext
//! file. They exist only for notes in that list, so the list's count bounds them, and
//! turning the list off (a count of 0) turns them off too.
//!
//! Positions are document offsets as the editor counts them, not pixels: pixels don't
//! survive a mode switch, another window width or the editor's estimated heights. The
//! frontend clamps them to the note's length, since the body can change in between.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Result, Vault};

/// JSON object `{ "<note id>": { "anchor": n, "head": n, "top": n } }`.
pub const NOTE_POSITIONS_SETTING: &str = "note_positions";
/// `"false"` turns reopening notes where they were left off; anything else, or no key, is on.
pub const RESTORE_POSITION_SETTING: &str = "restore_position";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotePosition {
    /// The selection's fixed end.
    pub anchor: u32,
    /// The selection's moving end: the cursor.
    pub head: u32,
    /// The offset at the top left of what the editor shows.
    pub top: u32,
}

type Positions = BTreeMap<i64, NotePosition>;

impl Vault {
    /// Whether notes reopen where they were left. On unless turned off.
    pub fn restore_position(&self) -> Result<bool> {
        Ok(self.setting(RESTORE_POSITION_SETTING)?.as_deref() != Some("false"))
    }

    /// Turning it off deletes the stored positions.
    pub fn set_restore_position(&self, on: bool) -> Result<()> {
        self.set_setting(RESTORE_POSITION_SETTING, if on { "true" } else { "false" })?;
        if !on {
            self.delete_setting(NOTE_POSITIONS_SETTING)?;
        }
        Ok(())
    }

    /// Where note `id` was left, if that is remembered.
    pub fn note_position(&self, id: i64) -> Result<Option<NotePosition>> {
        if !self.restore_position()? {
            return Ok(None);
        }
        Ok(self.note_positions()?.get(&id).copied())
    }

    /// Remembers where note `id` was left. Nothing is stored while the setting or the
    /// recently opened list is off, or for a note that is not in that list. Returns
    /// whether the vault was written to.
    pub fn set_note_position(&self, id: i64, position: NotePosition) -> Result<bool> {
        if !self.restore_position()? || self.recent_opened_count()? == 0 || !self.recent_opened_ids()?.contains(&id) {
            return Ok(false);
        }
        let mut positions = self.note_positions()?;
        if positions.insert(id, position) == Some(position) {
            return Ok(false);
        }
        self.write_note_positions(&positions)?;
        Ok(true)
    }

    /// Drops the positions of notes that are not in `recent`, the recently opened list as
    /// it was just written.
    pub(super) fn keep_note_positions(&self, recent: &[i64]) -> Result<()> {
        let Some(stored) = self.setting(NOTE_POSITIONS_SETTING)? else {
            return Ok(());
        };
        let mut positions: Positions = serde_json::from_str(&stored).unwrap_or_default();
        let before = positions.len();
        positions.retain(|id, _| recent.contains(id));
        if positions.is_empty() {
            self.delete_setting(NOTE_POSITIONS_SETTING)
        } else if positions.len() != before {
            self.write_note_positions(&positions)
        } else {
            Ok(())
        }
    }

    fn note_positions(&self) -> Result<Positions> {
        // An unreadable value is treated as no positions; the next write replaces it.
        Ok(self
            .setting(NOTE_POSITIONS_SETTING)?
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }

    fn write_note_positions(&self, positions: &Positions) -> Result<()> {
        let json = serde_json::to_string(positions).expect("a map of positions serialises");
        self.set_setting(NOTE_POSITIONS_SETTING, &json)
    }
}
