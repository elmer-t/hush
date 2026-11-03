//! The Suggestions view: one tag's suggestions across the whole vault, to tag many notes
//! at once (most likely first) or to teach the tag's classifier fastest by answering the
//! notes it is least sure about (needs review first). Scoring is one scorer
//! (`Vault::tag_scorers`) applied to every note vector; the app scores the in-memory
//! vectors of the related-notes index (`classify::review`), [`Vault::score_notes`] reads
//! them from the vault instead.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::heads::{ScoreMethod, TagScorer};
use super::metadata::{MetadataEdit, MetadataEditReport, normalize_tag};
use super::tags::set_feedback;
use super::{Error, Result, Vault, now_ms};
use crate::markdown;

/// A classifier's suggestions below this probability aren't listed. (A description's
/// are listed from the model's suggestion threshold.)
pub const REVIEW_MIN_PROBABILITY: f32 = 0.2;
/// Characters of body text shown under a suggestion's title.
const EXCERPT_CHARS: usize = 160;
/// Vectors read per query when scoring from the vault.
const PAGE: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewOrder {
    /// Highest score first, for tagging many notes at once.
    MostLikely,
    /// The score closest to the scorer's point of doubt first (probability 0.5 for a
    /// classifier, the suggestion threshold for a description), for training.
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewNote {
    pub id: i64,
    pub folder: String,
    pub title: String,
    /// The start of the body without frontmatter, whitespace collapsed.
    pub excerpt: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionReview {
    pub tag: String,
    /// How the tag is scored; `None` when it can't be (no head and no embedded
    /// description, or no model ready), and then there are no notes.
    pub method: Option<ScoreMethod>,
    /// The lowest score listed.
    pub min_score: f32,
    pub order: ReviewOrder,
    /// How many notes are suggested in all; `notes` holds the first of them.
    pub total: usize,
    pub notes: Vec<ReviewNote>,
}

impl Vault {
    /// The scorer of one tag (case-sensitive), as [`Vault::tag_scorers`] makes it.
    pub fn tag_scorer(&self, tag: &str) -> Result<Option<TagScorer>> {
        Ok(self.tag_scorers()?.into_iter().find(|s| s.tag == tag))
    }

    /// Scores every live note with a vector of the ready model, reading the vectors from
    /// the vault in pages. In note id order.
    pub fn score_notes(&self, scorer: &TagScorer) -> Result<Vec<(i64, f32)>> {
        let Some(model) = self.embedding_model_ready()? else {
            return Ok(Vec::new());
        };
        let mut scores = Vec::new();
        let mut after = 0;
        loop {
            let page = self.training_vectors(&model, after, PAGE)?;
            scores.extend(page.vectors.iter().map(|(id, _, v)| (*id, scorer.score(v))));
            match page.next {
                Some(next) => after = next,
                None => return Ok(scores),
            }
        }
    }

    /// One tag's suggestions across the vault, scored from the vectors in the vault.
    pub fn suggestion_review(
        &self,
        tag: &str,
        order: ReviewOrder,
        limit: usize,
    ) -> Result<SuggestionReview> {
        let scorer = self.tag_scorer(tag)?;
        let scores = match &scorer {
            Some(s) => self.score_notes(s)?,
            None => Vec::new(),
        };
        self.suggestion_review_of(tag, scorer.as_ref(), &scores, order, limit)
    }

    /// One tag's suggestions from `scores` (`(note id, score)` from `scorer`), in
    /// `order`, the first `limit` with their title, folder and excerpt. Left out: notes
    /// that have the tag (frontmatter or inline), notes with a decision recorded for it
    /// (rejected, or accepted), trashed notes, and scores below [`review_min_score`].
    pub fn suggestion_review_of(
        &self,
        tag: &str,
        scorer: Option<&TagScorer>,
        scores: &[(i64, f32)],
        order: ReviewOrder,
        limit: usize,
    ) -> Result<SuggestionReview> {
        let Some(scorer) = scorer else {
            return Ok(SuggestionReview {
                tag: tag.to_string(),
                method: None,
                min_score: 0.0,
                order,
                total: 0,
                notes: Vec::new(),
            });
        };
        let mut excluded: HashSet<i64> = HashSet::new();
        {
            let mut stmt = self.conn.prepare_cached(
                "SELECT nt.note_id FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE t.name = ?1
                 UNION SELECT note_id FROM tag_feedback WHERE tag = ?1
                 UNION SELECT id FROM notes WHERE deleted_at IS NOT NULL",
            )?;
            for id in stmt.query_map([tag], |r| r.get::<_, i64>(0))? {
                excluded.insert(id?);
            }
        }
        let min_score = review_min_score(scorer);
        let mut listed: Vec<(i64, f32)> = scores
            .iter()
            .copied()
            .filter(|(id, s)| *s >= min_score && !excluded.contains(id))
            .collect();
        sort_review(&mut listed, order, doubt_point(scorer));
        let total = listed.len();
        listed.truncate(limit);

        let mut stmt = self.conn.prepare_cached(
            "SELECT folder, title, body FROM notes WHERE id = ?1 AND deleted_at IS NULL",
        )?;
        let mut notes = Vec::with_capacity(listed.len());
        for (id, score) in listed {
            let row: Option<(String, String, Zeroizing<String>)> = stmt
                .query_row([id], |r| {
                    Ok((r.get(0)?, r.get(1)?, Zeroizing::new(r.get(2)?)))
                })
                .optional()?;
            if let Some((folder, title, body)) = row {
                notes.push(ReviewNote {
                    id,
                    folder,
                    title,
                    excerpt: excerpt(&body),
                    score,
                });
            }
        }
        Ok(SuggestionReview {
            tag: tag.to_string(),
            method: Some(scorer.method),
            min_score,
            order,
            total,
            notes,
        })
    }

    /// Accepts a tag on several notes at once: the bulk *Add tag* edit (as
    /// `edit_notes_metadata`) and an accepted example for every note that now has it, in
    /// one transaction, all or nothing. Notes whose frontmatter can't be read are skipped
    /// and get no feedback.
    pub fn accept_tag_suggestions(&mut self, ids: &[i64], tag: &str) -> Result<MetadataEditReport> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        let report = Self::edit_notes_metadata_in(&tx, ids, &MetadataEdit::AddTag(tag.clone()))?;
        let now = now_ms();
        for &id in ids {
            if !report.unreadable.iter().any(|n| n.id == id) {
                set_feedback(&tx, id, &tag, true, now)?;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Rejects a tag on several notes at once: only rejected examples are recorded; the
    /// notes themselves don't change. All or nothing: an unknown or trashed id rejects nothing.
    pub fn reject_tag_suggestions(&mut self, ids: &[i64], tag: &str) -> Result<()> {
        let tag = normalize_tag(tag)?;
        let tx = self.conn.transaction()?;
        let now = now_ms();
        {
            let mut live =
                tx.prepare_cached("SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL")?;
            for &id in ids {
                if !live.exists([id])? {
                    return Err(Error::NoteNotFound);
                }
                set_feedback(&tx, id, &tag, false, now)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether `tag` is on note `id` (frontmatter or inline), for tests and callers that
    /// need to check one note.
    pub fn note_has_tag(&self, id: i64, tag: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1 AND t.name = ?2",
            )?
            .exists(params![id, tag])?)
    }
}

/// The lowest score the Suggestions view lists for a scorer: [`REVIEW_MIN_PROBABILITY`]
/// for a classifier, else the description's suggestion threshold.
pub fn review_min_score(scorer: &TagScorer) -> f32 {
    match scorer.method {
        ScoreMethod::Classifier => REVIEW_MIN_PROBABILITY,
        _ => scorer.threshold,
    }
}

/// Where a scorer is least sure: probability 0.5 for a classifier (its threshold), the
/// suggestion threshold for a description. Both are the scorer's `threshold`.
fn doubt_point(scorer: &TagScorer) -> f32 {
    scorer.threshold
}

/// Sorts `(note id, score)` in place. Ties go to the higher score, then the lower id, so
/// the order is stable between calls.
pub fn sort_review(scores: &mut [(i64, f32)], order: ReviewOrder, doubt: f32) {
    match order {
        ReviewOrder::MostLikely => scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0))),
        ReviewOrder::NeedsReview => scores.sort_by(|a, b| {
            (a.1 - doubt)
                .abs()
                .total_cmp(&(b.1 - doubt).abs())
                .then(b.1.total_cmp(&a.1))
                .then(a.0.cmp(&b.0))
        }),
    }
}

/// The first [`EXCERPT_CHARS`] characters of the body without frontmatter, whitespace
/// collapsed; `…` when cut.
fn excerpt(body: &str) -> String {
    let text = match markdown::split_frontmatter(body) {
        Some((_, offset)) => &body[offset..],
        None => body,
    };
    let mut out = String::new();
    let mut chars = 0;
    for word in text.split_whitespace() {
        if chars > 0 {
            out.push(' ');
            chars += 1;
        }
        for c in word.chars() {
            if chars >= EXCERPT_CHARS {
                out.push('…');
                return out;
            }
            out.push(c);
            chars += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_skip_frontmatter_and_collapse_whitespace() {
        assert_eq!(
            excerpt("---\ntags: [a]\n---\n# Title\n\nSome   text\r\nhere"),
            "# Title Some text here"
        );
        assert_eq!(excerpt(""), "");
        let long = "word ".repeat(100);
        let e = excerpt(&long);
        assert_eq!(e.chars().count(), EXCERPT_CHARS + 1);
        assert!(e.ends_with('…'));
    }

    #[test]
    fn needs_review_puts_the_point_of_doubt_first() {
        let mut s = vec![(1, 0.9), (2, 0.42), (3, 0.55), (4, 0.2), (5, 0.5)];
        sort_review(&mut s, ReviewOrder::NeedsReview, 0.5);
        assert_eq!(s.iter().map(|x| x.0).collect::<Vec<_>>(), [5, 3, 2, 4, 1]);
        sort_review(&mut s, ReviewOrder::MostLikely, 0.5);
        assert_eq!(s.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 3, 5, 2, 4]);
    }
}
