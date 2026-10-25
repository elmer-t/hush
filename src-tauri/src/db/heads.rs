//! Per-tag classifier heads (see [`crate::classify`]): their storage, the training data
//! the job reads, and how a tag is scored for a note. A tag with a head for the ready
//! model is scored by it; one without falls back to its description (blended with the
//! centroid of its positive notes when it has some), as zero-shot suggestions do.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use zeroize::Zeroizing;

use super::{Result, Vault, now_ms};
use crate::classify::{self, Head, MIN_NEGATIVES, MIN_POSITIVES, PROBABILITY_THRESHOLD};
use crate::embed;

/// A head as stored in `tag_heads`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredHead {
    pub tag: String,
    pub model: String,
    pub head: Head,
    pub positives: usize,
    pub negatives: usize,
    pub precision: Option<f32>,
    pub recall: Option<f32>,
    pub trained_at: i64,
    /// SHA-256 of everything the head was trained on (see `classify::job`), so the job
    /// can tell when it is out of date.
    pub inputs: [u8; 32],
}

/// One page of note vectors for training, in note id order.
pub struct TrainingPage {
    /// `(note id, content hash, vector)`.
    pub vectors: Vec<(i64, [u8; 32], Zeroizing<Vec<f32>>)>,
    /// The id to continue after, or `None` when this was the last page.
    pub next: Option<i64>,
}

/// A tag's labelled notes: live notes that have it (or on which it was accepted), and
/// live notes on which it was rejected. Sorted ids; the two sets don't overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct TagLabels {
    pub tag: String,
    pub positives: Vec<i64>,
    pub rejected: Vec<i64>,
}

/// How a tag was scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScoreMethod {
    /// The tag's trained head; the score is a probability.
    Classifier,
    /// Cosine similarity with the description alone.
    Description,
    /// Cosine similarity with the description blended with the centroid of the tag's
    /// positive notes (too few for a head yet).
    DescriptionAndExamples,
}

/// Scores one tag for any note vector of the ready model.
#[derive(Debug, Clone)]
pub struct TagScorer {
    pub tag: String,
    pub method: ScoreMethod,
    /// A score at or above this suggests the tag.
    pub threshold: f32,
    kind: ScorerKind,
}

#[derive(Debug, Clone)]
enum ScorerKind {
    Head(Head),
    Cosine(Vec<f32>),
}

impl TagScorer {
    pub fn score(&self, vector: &[f32]) -> f32 {
        match &self.kind {
            ScorerKind::Head(head) => head.probability(vector),
            ScorerKind::Cosine(target) => embed::cosine(vector, target),
        }
    }
}

/// What the Tags settings show about a tag's classifier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagTraining {
    pub tag: String,
    /// Usable positive examples now: live notes with a ready vector that have the tag
    /// (or on which it was accepted).
    pub positives: usize,
    /// Every other live note with a ready vector.
    pub negatives: usize,
    /// Of `negatives`, notes on which the user rejected the tag.
    pub rejected: usize,
    pub status: TrainingStatus,
    /// More positive examples needed before a head is trained (0 when enough).
    pub needed: usize,
    /// Cross-validated, from the last training; `None` without a head or when undefined.
    pub precision: Option<f32>,
    pub recall: Option<f32>,
    pub trained_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrainingStatus {
    /// Has a head for the ready model.
    Trained,
    /// Has enough examples; the job trains it after the vault has been quiet for a while.
    Waiting,
    /// Too few examples; suggested from its description.
    DescriptionOnly,
    /// Too few examples and no description: not suggested.
    NeedsExamples,
}

impl Vault {
    /// The stored head of `tag`, for any model.
    pub fn tag_head(&self, tag: &str) -> Result<Option<StoredHead>> {
        Ok(self.read_heads(Some(tag))?.pop())
    }

    /// Every stored head, for any model, by tag.
    pub fn tag_heads(&self) -> Result<Vec<StoredHead>> {
        self.read_heads(None)
    }

    fn read_heads(&self, tag: Option<&str>) -> Result<Vec<StoredHead>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT tag, model, weights, bias, n_pos, n_neg, precision, recall, trained_at, inputs
             FROM tag_heads WHERE ?1 IS NULL OR tag = ?1 ORDER BY tag",
        )?;
        let heads = stmt
            .query_map([tag], |r| {
                let inputs: Vec<u8> = r.get(9)?;
                Ok(StoredHead {
                    tag: r.get(0)?,
                    model: r.get(1)?,
                    head: Head { weights: embed::blob_to_vector(&r.get::<_, Vec<u8>>(2)?), bias: r.get::<_, f64>(3)? as f32 },
                    positives: r.get::<_, i64>(4)? as usize,
                    negatives: r.get::<_, i64>(5)? as usize,
                    precision: r.get::<_, Option<f64>>(6)?.map(|v| v as f32),
                    recall: r.get::<_, Option<f64>>(7)?.map(|v| v as f32),
                    trained_at: r.get(8)?,
                    inputs: inputs.try_into().unwrap_or([0; 32]),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(heads)
    }

    /// Stores a head, replacing the tag's previous one. Refused (returns `false`) when
    /// `head.model` is no longer the ready model: the vectors moved on while it trained.
    pub fn write_tag_head(&mut self, head: &StoredHead) -> Result<bool> {
        if self.embedding_model_ready()?.as_deref() != Some(head.model.as_str()) {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO tag_heads(tag, model, weights, bias, n_pos, n_neg, precision, recall, trained_at, inputs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                head.tag,
                head.model,
                embed::vector_to_blob(&head.head.weights),
                f64::from(head.head.bias),
                head.positives as i64,
                head.negatives as i64,
                head.precision.map(f64::from),
                head.recall.map(f64::from),
                head.trained_at,
                head.inputs.as_slice(),
            ],
        )?;
        Ok(true)
    }

    /// Deletes every head that isn't for `model` or whose tag isn't in `keep`. Returns
    /// how many went.
    pub fn retain_tag_heads(&mut self, model: &str, keep: &[String]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let tags: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT tag, model FROM tag_heads")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows
        };
        let mut gone = 0;
        for (tag, head_model) in tags {
            if head_model != model || !keep.contains(&tag) {
                gone += tx.execute("DELETE FROM tag_heads WHERE tag = ?1", [&tag])?;
            }
        }
        tx.commit()?;
        Ok(gone)
    }

    /// Up to `limit` `model` vectors of live notes with an id above `after`.
    pub fn training_vectors(&self, model: &str, after: i64, limit: u32) -> Result<TrainingPage> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.note_id, e.content_hash, e.vector FROM note_embeddings e JOIN notes n ON n.id = e.note_id
             WHERE e.model = ?1 AND e.note_id > ?2 AND n.deleted_at IS NULL ORDER BY e.note_id LIMIT ?3",
        )?;
        let vectors: Vec<(i64, [u8; 32], Zeroizing<Vec<f32>>)> = stmt
            .query_map(params![model, after, limit], |r| {
                let hash: Vec<u8> = r.get(1)?;
                let blob: Zeroizing<Vec<u8>> = Zeroizing::new(r.get(2)?);
                Ok((r.get(0)?, hash.try_into().unwrap_or([0; 32]), Zeroizing::new(embed::blob_to_vector(&blob))))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let next = (vectors.len() == limit as usize).then(|| vectors.last().map(|v| v.0)).flatten();
        Ok(TrainingPage { vectors, next })
    }

    /// Every tag with at least one positive note, by name.
    pub fn training_labels(&self) -> Result<Vec<TagLabels>> {
        let mut sets: HashMap<String, (BTreeSet<i64>, BTreeSet<i64>)> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.name, nt.note_id, 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
             UNION ALL
             SELECT f.tag, f.note_id, f.accepted FROM tag_feedback f JOIN notes n ON n.id = f.note_id
             WHERE n.deleted_at IS NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?;
        for row in rows {
            let (tag, id, positive) = row?;
            let entry = sets.entry(tag).or_default();
            if positive { entry.0.insert(id) } else { entry.1.insert(id) };
        }
        let mut labels: Vec<TagLabels> = sets
            .into_iter()
            .filter(|(_, (pos, _))| !pos.is_empty())
            .map(|(tag, (pos, rej))| TagLabels {
                tag,
                rejected: rej.difference(&pos).copied().collect(),
                positives: pos.into_iter().collect(),
            })
            .collect();
        labels.sort_by(|a, b| a.tag.cmp(&b.tag));
        Ok(labels)
    }

    /// How every tag would be scored for a note of the ready model: by its head for that
    /// model if it has one, else by its embedded description, blended with the centroid
    /// of its positive notes when there are any. Tags with neither aren't included.
    /// Empty when no model is ready (or the model has no threshold for descriptions).
    pub fn tag_scorers(&self) -> Result<Vec<TagScorer>> {
        let Some(model) = self.embedding_model_ready()? else { return Ok(Vec::new()) };
        let mut scorers: Vec<TagScorer> = self
            .tag_heads()?
            .into_iter()
            .filter(|h| h.model == model)
            .map(|h| TagScorer {
                tag: h.tag,
                method: ScoreMethod::Classifier,
                threshold: PROBABILITY_THRESHOLD,
                kind: ScorerKind::Head(h.head),
            })
            .collect();
        let Some(threshold) = self.suggestion_threshold(&model)? else { return Ok(scorers) };
        let described: Vec<(String, Vec<u8>)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT tag, vector FROM tag_descriptions WHERE model = ?1 AND vector IS NOT NULL ORDER BY tag",
            )?;
            let rows = stmt.query_map([&model], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (tag, blob) in described {
            if scorers.iter().any(|s| s.tag == tag) {
                continue;
            }
            let mut target = embed::blob_to_vector(&blob);
            embed::normalize(&mut target);
            let (centroid, n) = self.positive_centroid(&model, &tag)?;
            let method = if n == 0 {
                ScoreMethod::Description
            } else {
                // The examples count for more as they approach the number a head needs:
                // 1 example adds a tenth of the centroid, 10 weigh as much as the description.
                let alpha = n.min(MIN_POSITIVES) as f32 / MIN_POSITIVES as f32;
                target.iter_mut().zip(&centroid).for_each(|(t, c)| *t += alpha * c);
                embed::normalize(&mut target);
                ScoreMethod::DescriptionAndExamples
            };
            scorers.push(TagScorer { tag, method, threshold, kind: ScorerKind::Cosine(target) });
        }
        scorers.sort_by(|a, b| a.tag.cmp(&b.tag));
        Ok(scorers)
    }

    /// The unit-length mean of the unit-length `model` vectors of a tag's positive notes,
    /// and how many there were.
    fn positive_centroid(&self, model: &str, tag: &str) -> Result<(Vec<f32>, usize)> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.vector FROM note_embeddings e JOIN notes n ON n.id = e.note_id
             WHERE e.model = ?1 AND n.deleted_at IS NULL AND e.note_id IN (
                 SELECT nt.note_id FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE t.name = ?2
                 UNION SELECT note_id FROM tag_feedback WHERE tag = ?2 AND accepted = 1)",
        )?;
        let mut sum: Vec<f32> = Vec::new();
        let mut n = 0;
        let rows = stmt.query_map(params![model, tag], |r| r.get::<_, Vec<u8>>(0))?;
        for blob in rows {
            let mut v = embed::blob_to_vector(&blob?);
            embed::normalize(&mut v);
            if sum.is_empty() {
                sum = vec![0.0; v.len()];
            }
            if v.len() == sum.len() {
                sum.iter_mut().zip(&v).for_each(|(s, x)| *s += x);
                n += 1;
            }
        }
        embed::normalize(&mut sum);
        Ok((sum, n))
    }

    /// The score of one tag for one note, whether or not it would be suggested (the note
    /// may have the tag, or have rejected it). `None` without a note vector or a scorer.
    pub fn tag_score(&self, id: i64, tag: &str) -> Result<Option<(f32, ScoreMethod)>> {
        let Some(note) = self.note_embedding(id)? else { return Ok(None) };
        Ok(self.tag_scorers()?.into_iter().find(|s| s.tag == tag).map(|s| (s.score(&note), s.method)))
    }

    /// The classifier state of every tag that is used or described, for the Tags settings.
    pub fn tag_training(&self) -> Result<Vec<TagTraining>> {
        let model = self.embedding_model_ready()?;
        let heads: HashMap<String, StoredHead> = match &model {
            Some(m) => self.tag_heads()?.into_iter().filter(|h| &h.model == m).map(|h| (h.tag.clone(), h)).collect(),
            None => HashMap::new(),
        };
        let total: usize = match &model {
            Some(m) => self
                .conn
                .query_row(
                    "SELECT count(*) FROM note_embeddings e JOIN notes n ON n.id = e.note_id
                     WHERE e.model = ?1 AND n.deleted_at IS NULL",
                    [m],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0) as usize,
            None => 0,
        };
        // Counted against the notes that have a ready vector: only those can be examples.
        let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
        if let Some(m) = &model {
            let mut stmt = self.conn.prepare_cached(
                "WITH v AS (SELECT e.note_id FROM note_embeddings e JOIN notes n ON n.id = e.note_id
                            WHERE e.model = ?1 AND n.deleted_at IS NULL),
                      pos AS (SELECT t.name AS tag, nt.note_id FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
                              UNION SELECT tag, note_id FROM tag_feedback WHERE accepted = 1)
                 SELECT tag, count(*), 0 FROM pos WHERE note_id IN v GROUP BY tag
                 UNION ALL
                 SELECT f.tag, 0, count(*) FROM tag_feedback f
                 WHERE f.accepted = 0 AND f.note_id IN v
                   AND NOT EXISTS (SELECT 1 FROM pos WHERE pos.tag = f.tag AND pos.note_id = f.note_id)
                 GROUP BY f.tag",
            )?;
            let rows = stmt.query_map([m], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
            for row in rows {
                let (tag, pos, rej) = row?;
                let entry = counts.entry(tag).or_default();
                entry.0 += pos as usize;
                entry.1 += rej as usize;
            }
        }
        let training = self
            .tag_list()?
            .into_iter()
            .map(|info| {
                let (positives, rejected) = counts.get(&info.tag).copied().unwrap_or_default();
                let negatives = total.saturating_sub(positives);
                let enough = positives >= MIN_POSITIVES && negatives >= MIN_NEGATIVES;
                let head = heads.get(&info.tag);
                let status = match (head, enough, info.description.is_empty()) {
                    (Some(_), _, _) => TrainingStatus::Trained,
                    (None, true, _) => TrainingStatus::Waiting,
                    (None, false, false) => TrainingStatus::DescriptionOnly,
                    (None, false, true) => TrainingStatus::NeedsExamples,
                };
                TagTraining {
                    tag: info.tag,
                    positives,
                    negatives,
                    rejected,
                    status,
                    needed: MIN_POSITIVES.saturating_sub(positives),
                    precision: head.and_then(|h| h.precision),
                    recall: head.and_then(|h| h.recall),
                    trained_at: head.map(|h| h.trained_at),
                }
            })
            .collect();
        Ok(training)
    }
}

/// A head ready to store, stamped now.
pub fn new_stored_head(
    tag: &str,
    model: &str,
    head: Head,
    counts: (usize, usize),
    quality: classify::Quality,
    inputs: [u8; 32],
) -> StoredHead {
    StoredHead {
        tag: tag.to_string(),
        model: model.to_string(),
        head,
        positives: counts.0,
        negatives: counts.1,
        precision: quality.precision,
        recall: quality.recall,
        trained_at: now_ms(),
        inputs,
    }
}
