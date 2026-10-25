//! What tag discovery (`crate::discover`) needs from the vault: note vectors to group,
//! and creating a tag from a group. Nothing about the groups is stored.

use rusqlite::params;

use super::metadata::{MetadataEdit, MetadataEditReport, normalize_tag};
use super::tags::set_feedback;
use super::{Result, Vault, now_ms};
use crate::embed;

/// A note to group: id, folder, title and its vector.
pub type NoteVector = (i64, String, String, Vec<f32>);

impl Vault {
    /// Up to `limit` live notes with an id above `after` that have a `model` vector, in
    /// id order; with `untagged_only`, only notes without any tag (frontmatter or
    /// inline). Returns them with the id to continue after, `None` on the last page.
    pub fn discover_vectors(
        &self,
        model: &str,
        untagged_only: bool,
        after: i64,
        limit: u32,
    ) -> Result<(Vec<NoteVector>, Option<i64>)> {
        // CROSS JOIN keeps `notes` the outer loop, so a page walks note ids in order and
        // looks each vector up by primary key; left to itself SQLite reads every vector
        // of the model through the `model` index and sorts them, for every page.
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.id, n.folder, n.title, e.vector FROM notes n
             CROSS JOIN note_embeddings e ON e.note_id = n.id AND e.model = ?1
             WHERE n.deleted_at IS NULL AND n.id > ?2
               AND (?3 = 0 OR NOT EXISTS (SELECT 1 FROM note_tags nt WHERE nt.note_id = n.id))
             ORDER BY n.id LIMIT ?4",
        )?;
        let rows: Vec<NoteVector> = stmt
            .query_map(params![model, after, untagged_only, limit], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, embed::blob_to_vector(&r.get::<_, Vec<u8>>(3)?)))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let next = if rows.len() < limit as usize { None } else { rows.last().map(|r| r.0) };
        Ok((rows, next))
    }

    /// Creates a tag from a discovered group, in one transaction: saves `description`
    /// (an empty one leaves any existing description alone), adds the tag to the
    /// frontmatter of `ids` as the bulk *Add tag* edit does, and records an accepted
    /// example for each note that has the tag afterwards. Notes whose frontmatter can't
    /// be read are skipped and reported, without feedback.
    pub fn create_tag_from_group(&mut self, tag: &str, description: &str, ids: &[i64]) -> Result<MetadataEditReport> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        let report = Self::edit_notes_metadata_in(&tx, ids, &MetadataEdit::AddTag(tag.clone()))?;
        if !description.trim().is_empty() {
            Self::set_tag_description_in(&tx, &tag, description)?;
        }
        let now = now_ms();
        for &id in ids {
            if !report.unreadable.iter().any(|n| n.id == id) {
                set_feedback(&tx, id, &tag, true, now)?;
            }
        }
        tx.commit()?;
        Ok(report)
    }
}
