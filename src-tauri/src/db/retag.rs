//! Re-deriving every note's tags once the tag rules change
//! ([`markdown::TAG_RULES_VERSION`]): notes saved under older rules can hold tags the
//! parser no longer reads (`#a` from an anchor link, `#1e1e1e` from a colour). Only the
//! derived `note_tags` rows change; bodies are only read. The background pass that runs
//! these pages is `crate::retag`.

use std::collections::BTreeSet;

use rusqlite::params;
use zeroize::Zeroizing;

use super::notes::set_note_tags;
use super::tags::note_tag_names;
use super::{Result, Vault};
use crate::markdown;

/// The [`markdown::TAG_RULES_VERSION`] the vault's tags were last derived with.
pub const TAG_RULES_SETTING: &str = "tag_rules_version";
/// Where an unfinished pass continues, as `<rules version>:<note id>`, so a pass cut off
/// by a lock doesn't start over at the next unlock.
pub const TAG_RULES_PROGRESS_SETTING: &str = "tag_rules_progress";

/// One page of the pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetagPage {
    /// Live notes read.
    pub checked: usize,
    /// Of those, notes whose tags changed.
    pub changed: usize,
    /// The note id to continue after, or `None` when this was the last page.
    pub next: Option<i64>,
}

/// What finishing the pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetagEnd {
    /// Tags no note has any more.
    pub tags_dropped: usize,
    /// Classifier heads of those tags.
    pub heads_dropped: usize,
}

impl Vault {
    /// Where the pass has to start: `None` when the vault's tags already follow the
    /// current rules, else the note id to continue after (0 from the start).
    pub fn retag_start(&self) -> Result<Option<i64>> {
        let version = markdown::TAG_RULES_VERSION.to_string();
        if self.setting(TAG_RULES_SETTING)?.as_deref() == Some(version.as_str()) {
            return Ok(None);
        }
        let after = self
            .setting(TAG_RULES_PROGRESS_SETTING)?
            .and_then(|p| p.split_once(':').filter(|(v, _)| *v == version).and_then(|(_, id)| id.parse().ok()))
            .unwrap_or(0);
        Ok(Some(after))
    }

    /// Re-derives the tags of up to `limit` live notes after id `after`, in one
    /// transaction, and remembers where to continue. A note's rows are only rewritten
    /// when its tags differ, so a vault without junk tags writes nothing but progress.
    pub fn retag_page(&mut self, after: i64, limit: u32) -> Result<RetagPage> {
        let tx = self.conn.transaction()?;
        let mut page = RetagPage { checked: 0, changed: 0, next: None };
        {
            let mut stmt = tx.prepare_cached(
                "SELECT id, body FROM notes WHERE id > ?1 AND deleted_at IS NULL ORDER BY id LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![after, limit])?;
            let mut last = after;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                // Note text read in the background is wiped once used, as the jobs do.
                let body = Zeroizing::new(row.get::<_, String>(1)?);
                let tags = markdown::parse(&body).tags;
                last = id;
                page.checked += 1;
                let have: BTreeSet<String> = note_tag_names(&tx, id)?.into_iter().collect();
                if have != tags.iter().cloned().collect::<BTreeSet<_>>() {
                    set_note_tags(&tx, id, &tags)?;
                    page.changed += 1;
                }
            }
            if page.checked == limit as usize {
                page.next = Some(last);
            }
        }
        tx.commit()?;
        if let Some(next) = page.next {
            self.set_setting(TAG_RULES_PROGRESS_SETTING, &format!("{}:{next}", markdown::TAG_RULES_VERSION))?;
        }
        Ok(page)
    }

    /// Ends the pass: drops tags no note has any more and the classifier heads of tags
    /// without a note, then records that the vault follows the current rules. Tag
    /// descriptions and feedback are the user's own and stay.
    pub fn finish_retag(&mut self) -> Result<RetagEnd> {
        let tx = self.conn.transaction()?;
        let tags_dropped =
            tx.execute("DELETE FROM tags WHERE NOT EXISTS (SELECT 1 FROM note_tags nt WHERE nt.tag_id = tags.id)", [])?;
        let heads_dropped = tx.execute("DELETE FROM tag_heads WHERE tag NOT IN (SELECT name FROM tags)", [])?;
        tx.commit()?;
        self.set_setting(TAG_RULES_SETTING, &markdown::TAG_RULES_VERSION.to_string())?;
        self.delete_setting(TAG_RULES_PROGRESS_SETTING)?;
        Ok(RetagEnd { tags_dropped, heads_dropped })
    }
}
