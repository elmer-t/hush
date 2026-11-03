//! Bulk metadata edits: add or remove a tag, set the topic, the author or the rating on several
//! notes at once. The frontmatter is the source of truth, so each edit rewrites the
//! note's body and re-derives its index rows, the same as a save. Inline `#tags` in
//! the body text are never touched.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::notes::derive;
use super::tags;
use super::{Error, Result, Vault, now_ms};
use crate::markdown;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum MetadataEdit {
    AddTag(String),
    RemoveTag(String),
    /// An empty value removes the key.
    SetTopic(String),
    /// An empty value removes the key.
    SetAuthor(String),
    /// 1–5 stars; `null` removes the key.
    SetRating(Option<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitledNote {
    pub id: i64,
    pub folder: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataEditReport {
    /// Notes whose body was rewritten (and `updated_at` bumped).
    pub changed: Vec<i64>,
    /// Notes the edit left as they were, such as a tag that was already there.
    pub unchanged: usize,
    /// After removing a tag: notes that still have it, as an inline `#tag`.
    pub still_inline: Vec<TitledNote>,
    /// Notes skipped because their frontmatter can't be read; rewriting it could lose
    /// what the user wrote.
    pub unreadable: Vec<TitledNote>,
}

impl Vault {
    /// Applies one metadata edit to every note in `ids`, in one transaction: all or
    /// nothing, so an unknown (or trashed) id changes nothing. Title and folder don't
    /// change, so no other note's links change meaning.
    pub fn edit_notes_metadata(
        &mut self,
        ids: &[i64],
        edit: &MetadataEdit,
    ) -> Result<MetadataEditReport> {
        let tx = self.conn.transaction()?;
        let report = Self::edit_notes_metadata_in(&tx, ids, edit)?;
        tx.commit()?;
        Ok(report)
    }

    /// [`Vault::edit_notes_metadata`] inside the caller's transaction.
    pub(super) fn edit_notes_metadata_in(
        tx: &Connection,
        ids: &[i64],
        edit: &MetadataEdit,
    ) -> Result<MetadataEditReport> {
        let edit = normalize(edit)?;
        let now = now_ms();
        let mut report = MetadataEditReport::default();
        let mut seen = std::collections::HashSet::with_capacity(ids.len());
        for &id in ids {
            if !seen.insert(id) {
                continue;
            }
            let (folder, title, body): (String, String, String) = tx
                .query_row(
                    "SELECT folder, title, body FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
                .ok_or(Error::NoteNotFound)?;
            let note = || TitledNote {
                id,
                folder: folder.clone(),
                title: title.clone(),
            };

            if markdown::split_frontmatter(&body)
                .is_some_and(|(yaml, _)| !markdown::frontmatter_is_valid(yaml))
            {
                report.unreadable.push(note());
                continue;
            }
            let new_body = apply(&body, &edit);
            if let MetadataEdit::RemoveTag(tag) = &edit {
                if markdown::parse(&new_body)
                    .tags
                    .iter()
                    .any(|t| same_tag(t, tag))
                {
                    report.still_inline.push(note());
                }
            }
            if new_body == body {
                report.unchanged += 1;
                continue;
            }
            let before = tags::note_tag_names(&tx, id)?;
            tx.execute(
                "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, new_body, now],
            )?;
            derive(&tx, id, &new_body)?;
            tags::record_hand_edits(&tx, id, &before, now)?;
            report.changed.push(id);
        }
        Ok(report)
    }
}

/// Trims the value; a tag also loses its leading `#` and must be one word, because
/// frontmatter tags are split on commas and spaces.
fn normalize(edit: &MetadataEdit) -> Result<MetadataEdit> {
    let tag = normalize_tag;
    Ok(match edit {
        MetadataEdit::AddTag(t) => MetadataEdit::AddTag(tag(t)?),
        MetadataEdit::RemoveTag(t) => MetadataEdit::RemoveTag(tag(t)?),
        MetadataEdit::SetTopic(v) => MetadataEdit::SetTopic(v.trim().to_string()),
        MetadataEdit::SetAuthor(v) => MetadataEdit::SetAuthor(v.trim().to_string()),
        MetadataEdit::SetRating(Some(r)) if !(1..=5).contains(r) => {
            return Err(Error::InvalidRating);
        }
        MetadataEdit::SetRating(r) => MetadataEdit::SetRating(*r),
    })
}

/// Trims a tag and drops its leading `#`; it must be one word, because frontmatter tags
/// are split on commas and spaces.
pub(super) fn normalize_tag(t: &str) -> Result<String> {
    let t = t.trim().trim_start_matches('#').trim();
    if t.is_empty() {
        Err(Error::InvalidTag("tag is empty".into()))
    } else if t.contains(|c: char| c == ',' || c.is_whitespace()) {
        Err(Error::InvalidTag(
            "tag must not contain spaces or commas".into(),
        ))
    } else {
        Ok(t.to_string())
    }
}

/// The new body; equal to `body` when the edit changes nothing.
fn apply(body: &str, edit: &MetadataEdit) -> String {
    let parsed = markdown::parse(body);
    match edit {
        MetadataEdit::AddTag(tag) => {
            if parsed.frontmatter_tags.iter().any(|t| same_tag(t, tag)) {
                return body.to_string();
            }
            let mut tags = parsed.frontmatter_tags;
            tags.push(tag.clone());
            write_tags(body, &tags)
        }
        MetadataEdit::RemoveTag(tag) => {
            if !parsed.frontmatter_tags.iter().any(|t| same_tag(t, tag)) {
                return body.to_string();
            }
            let tags: Vec<String> = parsed
                .frontmatter_tags
                .into_iter()
                .filter(|t| !same_tag(t, tag))
                .collect();
            write_tags(body, &tags)
        }
        MetadataEdit::SetTopic(value) => set_scalar(body, "topic", parsed.topic.as_deref(), value),
        MetadataEdit::SetAuthor(value) => {
            set_scalar(body, "author", parsed.author.as_deref(), value)
        }
        MetadataEdit::SetRating(rating) => {
            // Setting or clearing also replaces a `rating` value that isn't a valid rating.
            let has_key =
                parsed.rating.is_some() || parsed.attributes.iter().any(|(k, _)| k == "rating");
            if parsed.rating == *rating && (rating.is_some() || !has_key) {
                return body.to_string();
            }
            let values: Vec<String> = rating.iter().map(u8::to_string).collect();
            markdown::set_frontmatter_field(body, "rating", &values)
        }
    }
}

/// Writes the frontmatter tags as one `tags` list. A singular `tag` key, which is read
/// the same way, is folded into it so the list isn't split over two keys.
pub(super) fn write_tags(body: &str, tags: &[String]) -> String {
    let body = markdown::set_frontmatter_field(body, "tag", &[]);
    markdown::set_frontmatter_field(&body, "tags", tags)
}

fn set_scalar(body: &str, key: &str, current: Option<&str>, value: &str) -> String {
    if current.unwrap_or("") == value {
        return body.to_string();
    }
    let values: Vec<String> = if value.is_empty() {
        Vec::new()
    } else {
        vec![value.to_string()]
    };
    markdown::set_frontmatter_field(body, key, &values)
}

/// Tags are case-sensitive: `jr` and `Jr` are different tags.
fn same_tag(a: &str, b: &str) -> bool {
    a == b
}
