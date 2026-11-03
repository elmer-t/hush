//! What tag discovery (`crate::discover`) needs from the vault: the notes in scope,
//! their vectors, and creating a tag from a group. Nothing about the groups is stored.

use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

use super::metadata::{MetadataEdit, MetadataEditReport, normalize_tag};
use super::tags::set_feedback;
use super::{Result, SearchQuery, Vault, now_ms};
use crate::{embed, markdown};

/// Which notes tag discovery groups.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiscoverScope {
    #[default]
    All,
    /// Notes without any tag (frontmatter or inline), to find what no tag covers.
    Untagged,
    /// A folder and its subfolders.
    Folder { folder: String },
    /// The notes matching a search (text and filters), all of them, unranked.
    Search { query: SearchQuery },
}

/// Where [`Vault::discover_scope`] continues; start from the default.
#[derive(Debug, Clone, Default)]
pub struct ScopeCursor {
    id: i64,
    folder: String,
    title: String,
}

impl Vault {
    /// Up to `limit` ids of live notes in `scope` after `cursor`, and the cursor to
    /// continue from (`None` after the last page). Not in id order. Whether a note has
    /// a vector isn't checked here.
    pub fn discover_scope(
        &self,
        scope: &DiscoverScope,
        cursor: &ScopeCursor,
        limit: u32,
    ) -> Result<(Vec<i64>, Option<ScopeCursor>)> {
        if let DiscoverScope::Search { query } = scope {
            let ids = self.search_ids(query, cursor.id, limit)?;
            let next = (ids.len() == limit as usize).then(|| ScopeCursor {
                id: *ids.last().unwrap(),
                ..Default::default()
            });
            return Ok((ids, next));
        }
        // The other scopes walk `notes_folder_title`, which holds only live notes and
        // has the id of each, so no row (and no body) is read; (folder, title) is unique
        // there, which makes it the cursor.
        let folder = match scope {
            DiscoverScope::Folder { folder } => markdown::normalize_folder(folder),
            _ => String::new(),
        };
        let untagged = matches!(scope, DiscoverScope::Untagged);
        // Subfolders are the range ['folder/', 'folder0'): '0' follows '/'.
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, folder, title FROM notes INDEXED BY notes_folder_title
             WHERE deleted_at IS NULL AND (folder, title) > (?1, ?2) AND folder >= ?3
               AND (?3 = '' OR folder = ?3 OR (folder >= ?3 || '/' AND folder < ?3 || '0'))
               AND (?4 = 0 OR NOT EXISTS (SELECT 1 FROM note_tags nt WHERE nt.note_id = notes.id))
             ORDER BY folder, title LIMIT ?5",
        )?;
        let mut ids = Vec::with_capacity(limit as usize);
        let mut last = None;
        let mut rows = stmt.query(params![
            cursor.folder,
            cursor.title,
            folder,
            untagged,
            limit
        ])?;
        while let Some(r) = rows.next()? {
            ids.push(r.get(0)?);
            last = Some((r.get(1)?, r.get(2)?));
        }
        let next = match last {
            Some((folder, title)) if ids.len() == limit as usize => Some(ScopeCursor {
                id: 0,
                folder,
                title,
            }),
            _ => None,
        };
        Ok((ids, next))
    }

    /// The `model` vector of each of `ids` that has one.
    pub fn discover_vectors(&self, model: &str, ids: &[i64]) -> Result<Vec<(i64, Vec<f32>)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT vector FROM note_embeddings WHERE note_id = ?1 AND model = ?2",
        )?;
        let mut vectors = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(blob) = stmt
                .query_row(params![id, model], |r| r.get::<_, Vec<u8>>(0))
                .optional()?
            {
                vectors.push((id, embed::blob_to_vector(&blob)));
            }
        }
        Ok(vectors)
    }

    /// Up to `limit` `model` vectors in storage order (whatever note they belong to,
    /// live or not), with the position to continue after (`None` on the last page): one
    /// pass over every vector reads the table front to back instead of seeking per note.
    pub fn discover_vector_page(
        &self,
        model: &str,
        after: i64,
        limit: u32,
    ) -> Result<(Vec<(i64, Vec<f32>)>, Option<i64>)> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT rowid, note_id, vector FROM note_embeddings
             WHERE model = ?1 AND rowid > ?2 ORDER BY rowid LIMIT ?3",
        )?;
        let mut last = None;
        let mut rows = Vec::with_capacity(limit as usize);
        let mut query = stmt.query(params![model, after, limit])?;
        while let Some(r) = query.next()? {
            last = Some(r.get::<_, i64>(0)?);
            rows.push((r.get(1)?, embed::blob_to_vector(&r.get::<_, Vec<u8>>(2)?)));
        }
        let next = if rows.len() < limit as usize {
            None
        } else {
            last
        };
        Ok((rows, next))
    }

    /// Folder and title of each of `ids` that is live, by id.
    pub fn discover_titles(&self, ids: &[i64]) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT folder, title FROM notes WHERE id = ?1 AND deleted_at IS NULL",
        )?;
        let mut rows = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some((folder, title)) = stmt
                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
            {
                rows.push((id, folder, title));
            }
        }
        Ok(rows)
    }

    /// Those of `ids` that are live notes, in the given order.
    pub fn live_notes(&self, ids: &[i64]) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL")?;
        let mut live = Vec::with_capacity(ids.len());
        for &id in ids {
            if stmt.exists([id])? {
                live.push(id);
            }
        }
        Ok(live)
    }

    /// Creates a tag from a discovered group, in one transaction: saves `description`
    /// (an empty one leaves any existing description alone), adds the tag to the
    /// frontmatter of `ids` as the bulk *Add tag* edit does, and records an accepted
    /// example for each note that has the tag afterwards. Notes whose frontmatter can't
    /// be read are skipped and reported, without feedback.
    pub fn create_tag_from_group(
        &mut self,
        tag: &str,
        description: &str,
        ids: &[i64],
    ) -> Result<MetadataEditReport> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        let report = Self::edit_notes_metadata_in(&tx, ids, &MetadataEdit::AddTag(tag.clone()))?;
        if !description.trim().is_empty() {
            Self::set_tag_description_in(&tx, &tag, description)?;
        }
        let now = now_ms();
        let unreadable: std::collections::HashSet<i64> =
            report.unreadable.iter().map(|n| n.id).collect();
        for &id in ids {
            if !unreadable.contains(&id) {
                set_feedback(&tx, id, &tag, true, now)?;
            }
        }
        tx.commit()?;
        Ok(report)
    }
}
