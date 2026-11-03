//! Numbers for the Activity panel and a note's internals in the details panel. The
//! counts are kept to index lookups, as the panel asks for them on a timer.

use rusqlite::OptionalExtension;
use serde::Serialize;

use super::tags::note_tag_names;
use super::{Result, Vault};
use crate::{embed, markdown};

/// A trained head as the Activity panel lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadTime {
    pub tag: String,
    pub model: String,
    pub trained_at: i64,
}

/// Where a note's tag came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TagSource {
    /// Typed in the note (frontmatter or inline).
    Manual,
    /// Added by accepting a suggestion.
    Suggestion,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteTag {
    pub tag: String,
    pub source: TagSource,
}

/// A stored vector of the note.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteVector {
    pub model: String,
    /// Lowercase hex SHA-256 of the text it was made from.
    pub hash: String,
    /// Whether that is the note's current text.
    pub current: bool,
}

/// What Hush stores and derives for one note, for the details panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteInternals {
    pub id: i64,
    pub folder: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Words and characters of the body, frontmatter left out.
    pub words: usize,
    pub chars: usize,
    /// Lowercase hex SHA-256 of the text the embedding job embeds (title and body);
    /// `None` for a note with nothing to embed.
    pub content_hash: Option<String>,
    pub vectors: Vec<NoteVector>,
    /// The model the job embeds with now.
    pub model: String,
    /// Waiting in the embedding queue.
    pub queued: bool,
    pub backlinks: usize,
    pub outgoing: usize,
    pub tags: Vec<NoteTag>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Vault {
    /// Live notes, counted through indexes: every note less the trashed ones (a
    /// partial index), without reading a row.
    pub fn live_note_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT (SELECT count(*) FROM notes) - (SELECT count(*) FROM notes WHERE deleted_at IS NOT NULL)",
            [],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as usize)
    }

    /// Every trained head, without its weights, by tag.
    pub fn tag_head_times(&self) -> Result<Vec<HeadTime>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT tag, model, trained_at FROM tag_heads ORDER BY tag")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(HeadTime {
                    tag: r.get(0)?,
                    model: r.get(1)?,
                    trained_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// The schema version (number of migrations applied).
    pub fn schema_version(&self) -> Result<u64> {
        let v: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        Ok(v.max(0) as u64)
    }

    /// See [`NoteInternals`]; live notes only.
    pub fn note_internals(&self, id: i64) -> Result<NoteInternals> {
        let note = self.note(id)?;
        let content = markdown::split_frontmatter(&note.body)
            .map_or(note.body.as_str(), |(_, start)| &note.body[start..]);
        let words = content.split_whitespace().count();
        let chars = content.chars().count();
        let current =
            embed::note_text(&note.title, &note.body).map(|text| embed::content_hash(&text));

        let mut stmt = self.conn.prepare_cached(
            "SELECT model, content_hash FROM note_embeddings WHERE note_id = ?1 ORDER BY model",
        )?;
        let vectors = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .map(|row| {
                let (model, hash) = row?;
                Ok(NoteVector {
                    current: current.is_some_and(|c| c[..] == hash[..]),
                    hash: hex(&hash),
                    model,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let queued = self
            .conn
            .prepare_cached("SELECT 1 FROM embedding_dirty WHERE note_id = ?1")?
            .query_row([id], |_| Ok(()))
            .optional()?
            .is_some();

        let accepted: Vec<String> = self
            .conn
            .prepare_cached("SELECT tag FROM tag_feedback WHERE note_id = ?1 AND accepted = 1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut tags: Vec<NoteTag> = note_tag_names(&self.conn, id)?
            .into_iter()
            .map(|tag| {
                let source = if accepted.contains(&tag) {
                    TagSource::Suggestion
                } else {
                    TagSource::Manual
                };
                NoteTag { tag, source }
            })
            .collect();
        tags.sort_by(|a, b| {
            a.tag
                .to_lowercase()
                .cmp(&b.tag.to_lowercase())
                .then_with(|| a.tag.cmp(&b.tag))
        });

        Ok(NoteInternals {
            id,
            folder: note.folder.clone(),
            created_at: note.created_at,
            updated_at: note.updated_at,
            words,
            chars,
            content_hash: current.map(|h| hex(&h)),
            vectors,
            model: self.embedding_model()?,
            queued,
            backlinks: self.backlinks(id)?.len(),
            outgoing: self.outgoing_links(id)?.len(),
            tags,
        })
    }
}
