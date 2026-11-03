//! Tag descriptions and zero-shot tag suggestions. The user describes a tag in a
//! sentence; the background job embeds the description with the ready model, and a note
//! whose vector is close enough to it gets the tag suggested. Accepting or rejecting a
//! suggestion is remembered in `tag_feedback`, the training data for the per-tag
//! classifiers in [`crate::classify`]: a tag with a trained head is scored by it instead
//! (see `db::heads`).
//!
//! The frontmatter stays the single source of truth for tags: accepting a suggestion
//! rewrites the note's frontmatter and re-derives it, like a save.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::heads::ScoreMethod;
use super::metadata::{normalize_tag, write_tags};
use super::notes::derive;
use super::{Error, Result, Vault, now_ms};
use crate::{embed, markdown};

/// Prefix of the per-model setting that overrides the model's default threshold:
/// `suggest_threshold:<model ID>`.
pub const SUGGEST_THRESHOLD_SETTING: &str = "suggest_threshold:";
/// Most suggestions shown for one note.
pub const SUGGESTIONS_PER_NOTE: usize = 3;

/// A tag for the Tags settings: used on notes, described, or both.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    pub tag: String,
    /// Live notes that have the tag.
    pub notes: usize,
    /// Empty when the tag has no description (and so is never suggested).
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSuggestion {
    pub tag: String,
    /// The head's probability for [`ScoreMethod::Classifier`], else the cosine
    /// similarity of the note and the tag's description (blended with its examples).
    pub score: f32,
    pub method: ScoreMethod,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagFeedback {
    pub tag: String,
    pub accepted: bool,
    pub created_at: i64,
}

impl Vault {
    /// Every tag used on a live note or described, sorted by name, with its note count.
    pub fn tag_list(&self) -> Result<Vec<TagInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, sum(notes), max(description) FROM (
                 SELECT t.name AS name, count(*) AS notes, '' AS description
                 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id GROUP BY t.name
                 UNION ALL
                 SELECT tag, 0, description FROM tag_descriptions
             ) GROUP BY name ORDER BY name COLLATE NOCASE, name",
        )?;
        let tags = stmt
            .query_map([], |r| {
                Ok(TagInfo { tag: r.get(0)?, notes: r.get::<_, i64>(1)? as usize, description: r.get(2)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(tags)
    }

    /// Sets a tag's description; an empty one removes it, so the tag is no longer
    /// suggested. A changed description loses its vector until the job embeds it again.
    pub fn set_tag_description(&mut self, tag: &str, description: &str) -> Result<()> {
        Self::set_tag_description_in(&self.conn, tag, description)
    }

    /// [`Vault::set_tag_description`] on `conn`, which may be a transaction.
    pub(super) fn set_tag_description_in(conn: &Connection, tag: &str, description: &str) -> Result<()> {
        let tag = normalize_tag(tag)?;
        let description = description.trim();
        if description.is_empty() {
            conn.execute("DELETE FROM tag_descriptions WHERE tag = ?1", [&tag])?;
            return Ok(());
        }
        conn.execute(
            "INSERT INTO tag_descriptions(tag, description) VALUES (?1, ?2)
             ON CONFLICT(tag) DO UPDATE SET
                 vector = CASE WHEN description = excluded.description THEN vector END,
                 model = CASE WHEN description = excluded.description THEN model END,
                 description = excluded.description",
            params![tag, description],
        )?;
        Ok(())
    }

    /// The model and vector of a tag's description, once the job has embedded it.
    pub fn tag_description_vector(&self, tag: &str) -> Result<Option<(String, Vec<f32>)>> {
        let row: Option<(Option<String>, Option<Vec<u8>>)> = self
            .conn
            .query_row("SELECT model, vector FROM tag_descriptions WHERE tag = ?1", [tag], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(match row {
            Some((Some(model), Some(blob))) => Some((model, embed::blob_to_vector(&blob))),
            _ => None,
        })
    }

    /// Descriptions with no vector from `model` yet, as `(tag, description)`.
    pub fn stale_tag_descriptions(&self, model: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT tag, description FROM tag_descriptions WHERE vector IS NULL OR model IS NOT ?1 ORDER BY tag",
        )?;
        let rows = stmt.query_map([model], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Stores `model` vectors for `(tag, description, vector)`. A description edited
    /// since it was read keeps no vector, so the job embeds the new text next.
    pub fn write_tag_description_vectors(&mut self, model: &str, vectors: &[(String, String, Vec<f32>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE tag_descriptions SET vector = ?3, model = ?4 WHERE tag = ?1 AND description = ?2",
            )?;
            for (tag, description, vector) in vectors {
                stmt.execute(params![tag, description, embed::vector_to_blob(vector), model])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The similarity at or above which `model` suggests a tag: the vault's
    /// `suggest_threshold:<model>` setting, else the model's default. `None` for a model
    /// Hush doesn't know that has no setting.
    pub fn suggestion_threshold(&self, model: &str) -> Result<Option<f32>> {
        let set = self
            .setting(&format!("{SUGGEST_THRESHOLD_SETTING}{model}"))?
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite());
        Ok(set.or_else(|| embed::descriptor(model).map(|d| d.suggestion_threshold)))
    }

    /// Overrides `model`'s threshold for this vault; `None` goes back to the default.
    pub fn set_suggestion_threshold(&self, model: &str, threshold: Option<f32>) -> Result<()> {
        let key = format!("{SUGGEST_THRESHOLD_SETTING}{model}");
        match threshold {
            None => {
                self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
                Ok(())
            }
            Some(t) if t.is_finite() && (-1.0..=1.0).contains(&t) => self.set_setting(&key, &t.to_string()),
            Some(_) => Err(Error::InvalidThreshold),
        }
    }

    /// Tags to suggest for a note, best first: tags whose scorer (`Vault::tag_scorers`:
    /// the trained head, else the description from the ready model) scores the note's
    /// vector at or above its threshold, that the note doesn't have and that the user
    /// hasn't rejected on it. Classifier suggestions come before description ones (their
    /// scores aren't comparable, and a head has learned from the user's own notes).
    pub fn tag_suggestions(&self, id: i64) -> Result<Vec<TagSuggestion>> {
        let Some(note) = self.note_embedding(id)? else { return Ok(Vec::new()) };
        let scorers = self.tag_scorers()?;
        if scorers.is_empty() {
            return Ok(Vec::new());
        }
        let mut excluded: Vec<String> = note_tag_names(&self.conn, id)?;
        let mut stmt = self.conn.prepare_cached("SELECT tag FROM tag_feedback WHERE note_id = ?1 AND accepted = 0")?;
        for tag in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            excluded.push(tag?);
        }
        let mut suggestions: Vec<TagSuggestion> = scorers
            .iter()
            .filter(|s| !excluded.contains(&s.tag))
            .filter_map(|s| {
                let score = s.score(&note);
                (score >= s.threshold).then(|| TagSuggestion { tag: s.tag.clone(), score, method: s.method })
            })
            .collect();
        let rank = |m: ScoreMethod| if m == ScoreMethod::Classifier { 0 } else { 1 };
        suggestions.sort_by(|a, b| {
            rank(a.method).cmp(&rank(b.method)).then(b.score.total_cmp(&a.score)).then_with(|| a.tag.cmp(&b.tag))
        });
        suggestions.truncate(SUGGESTIONS_PER_NOTE);
        Ok(suggestions)
    }

    /// Accepts a suggested tag: adds it to the note's frontmatter `tags` (as adding a tag
    /// in the metadata panel does), re-derives the note and records an accepted example.
    pub fn accept_tag_suggestion(&mut self, id: i64, tag: &str) -> Result<()> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        let body: String = tx
            .query_row("SELECT body FROM notes WHERE id = ?1 AND deleted_at IS NULL", [id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NoteNotFound)?;
        if markdown::split_frontmatter(&body).is_some_and(|(yaml, _)| !markdown::frontmatter_is_valid(yaml)) {
            return Err(Error::InvalidTag("the note's frontmatter can't be read, so it can't be added".into()));
        }
        let mut tags = markdown::parse(&body).frontmatter_tags;
        let now = now_ms();
        if !tags.contains(&tag) {
            tags.push(tag.clone());
            let new_body = write_tags(&body, &tags);
            let before = note_tag_names(&tx, id)?;
            tx.execute("UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1", params![id, new_body, now])?;
            derive(&tx, id, &new_body)?;
            record_hand_edits(&tx, id, &before, now)?;
        }
        set_feedback(&tx, id, &tag, true, now)?;
        tx.commit()?;
        Ok(())
    }

    /// Rejects a suggested tag: it is never suggested on this note again.
    pub fn reject_tag_suggestion(&mut self, id: i64, tag: &str) -> Result<()> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        if !tx.prepare_cached("SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL")?.exists([id])? {
            return Err(Error::NoteNotFound);
        }
        set_feedback(&tx, id, &tag, false, now_ms())?;
        tx.commit()?;
        Ok(())
    }

    /// A live note's recorded decisions, by tag. Trashed notes keep theirs hidden.
    pub fn tag_feedback(&self, id: i64) -> Result<Vec<TagFeedback>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.tag, f.accepted, f.created_at FROM tag_feedback f JOIN notes n ON n.id = f.note_id
             WHERE f.note_id = ?1 AND n.deleted_at IS NULL ORDER BY f.tag",
        )?;
        let rows = stmt
            .query_map([id], |r| Ok(TagFeedback { tag: r.get(0)?, accepted: r.get(1)?, created_at: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// The note's derived tags (frontmatter and inline).
pub(super) fn note_tag_names(conn: &Connection, id: i64) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare_cached("SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1")?;
    let names = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

/// Records what an edit by hand did to a note's tags, given its tags before the edit.
/// Removing a tag that could be suggested (it has a description or a classifier head, or
/// the note has feedback for it) counts as a rejection, so it isn't suggested again right away.
/// Other tags are left alone: typing `tags: [dream]` saves `d`, `dr`, … on the way,
/// and those aren't decisions. Adding a tag back clears its rejection: the tag itself
/// is the positive example.
pub(super) fn record_hand_edits(conn: &Connection, id: i64, before: &[String], now: i64) -> Result<()> {
    let after = note_tag_names(conn, id)?;
    for tag in before.iter().filter(|t| !after.contains(t)) {
        let suggestible = conn
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM tag_descriptions WHERE tag = ?2)
                     OR EXISTS (SELECT 1 FROM tag_heads WHERE tag = ?2)
                     OR EXISTS (SELECT 1 FROM tag_feedback WHERE note_id = ?1 AND tag = ?2)",
            )?
            .query_row(params![id, tag], |r| r.get::<_, bool>(0))?;
        if suggestible {
            set_feedback(conn, id, tag, false, now)?;
        }
    }
    let mut clear = conn.prepare_cached("DELETE FROM tag_feedback WHERE note_id = ?1 AND tag = ?2 AND accepted = 0")?;
    for tag in after.iter().filter(|t| !before.contains(t)) {
        clear.execute(params![id, tag])?;
    }
    Ok(())
}

/// The latest decision wins.
pub(super) fn set_feedback(conn: &Connection, id: i64, tag: &str, accepted: bool, now: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO tag_feedback(note_id, tag, accepted, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(note_id, tag) DO UPDATE SET accepted = excluded.accepted, created_at = excluded.created_at",
    )?
    .execute(params![id, tag, accepted, now])?;
    Ok(())
}
