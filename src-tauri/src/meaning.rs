//! Search by meaning (#146): the sidebar search's `Meaning` mode. It fuses two ranked
//! lists with reciprocal rank fusion: the notes whose vectors are closest to the
//! query's (above the model's [`crate::embed::ModelDescriptor::meaning_threshold`]),
//! and today's full-text ranking. Vectors alone miss exact names, codes and rare words
//! typed on purpose; the full-text side keeps those.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::db::{SearchPage, SearchQuery};
use crate::embed::related::{self, Allowed};
use crate::embed::{self, EmbedError, Embedder};
use crate::session::{Session, SessionError};

/// The `k` of reciprocal rank fusion: a note scores `1 / (RRF_K + rank)` per list.
pub const RRF_K: f32 = 60.0;
/// The fused list is cut here; the summary then says "top 1,000".
pub const FUSED_LIMIT: usize = 1_000;
/// Notes the vector side contributes at most, after the exact rerank.
pub const VECTOR_LIMIT: usize = 200;

/// One note of the fused list.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedHit {
    pub id: i64,
    /// In the full-text list.
    pub words: bool,
    /// In the vector list, with its cosine to the query.
    pub cosine: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fused {
    pub hits: Vec<FusedHit>,
    /// More notes than `limit` were in the lists, so the list was cut.
    pub cut: bool,
}

/// Fuses `words` (note ids, best first) and `meaning` (`(id, cosine)`, best first) with
/// reciprocal rank fusion. Vector hits below `min_cosine` don't count, so a query with
/// no real match by meaning returns the full-text hits alone. Ties go to notes in the
/// full-text list, then to the lower id, so the order is stable. The first `limit`.
pub fn fuse(words: &[i64], meaning: &[(i64, f32)], min_cosine: f32, limit: usize) -> Fused {
    use std::collections::HashMap;
    let mut fused: Vec<(f32, FusedHit)> = Vec::with_capacity(words.len() + meaning.len());
    let mut at: HashMap<i64, usize> = HashMap::with_capacity(words.len() + meaning.len());
    for (rank, &id) in words.iter().enumerate() {
        if at.contains_key(&id) {
            continue;
        }
        at.insert(id, fused.len());
        fused.push((rrf(rank), FusedHit { id, words: true, cosine: None }));
    }
    let mut rank = 0;
    for &(id, cosine) in meaning {
        if !(cosine >= min_cosine) {
            continue;
        }
        let score = rrf(rank);
        rank += 1;
        match at.get(&id) {
            Some(&i) if fused[i].1.cosine.is_none() => {
                fused[i].0 += score;
                fused[i].1.cosine = Some(cosine);
            }
            Some(_) => {}
            None => {
                at.insert(id, fused.len());
                fused.push((score, FusedHit { id, words: false, cosine: Some(cosine) }));
            }
        }
    }
    fused.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.words.cmp(&a.1.words)).then(a.1.id.cmp(&b.1.id)));
    let cut = fused.len() > limit;
    fused.truncate(limit);
    Fused { hits: fused.into_iter().map(|(_, h)| h).collect(), cut }
}

/// Where a meaning search gets the embedder for its query.
pub trait QueryModels: Send + Sync {
    /// An embedder of `model` for search queries, loaded if it isn't yet. It must not
    /// wait for the indexing job's model runs.
    fn query_embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError>;
    /// Whether [`QueryModels::query_embedder`] would return at once.
    fn query_loaded(&self, model: &str) -> bool;
}

/// A sidebar search in `Meaning` mode, ranked as in [`fuse`], with every filter and the
/// folder applied to both sides: the vector side scores only the notes that pass them,
/// so a folder's best matches are found even when none are among the vault's best.
///
/// Falls back to a words search (saying why in `words_only`) when the query can't be
/// searched by meaning: under two characters, with quoted phrases, no ready model, or
/// the in-memory index not built for it yet. A newer search cancels this one (`None`).
pub fn search(session: &Session, models: &dyn QueryModels, q: &SearchQuery) -> Result<Option<SearchPage>, SessionError> {
    let ticket = session.begin_search();
    let words_only = |reason: Option<String>| {
        session.run_search(ticket, |v, stop| {
            Ok(v.search_page_until(q, stop)?.map(|mut page| {
                page.words_only = reason;
                page
            }))
        })
    };
    let text = q.text.trim();
    if text.chars().count() < 2 {
        return words_only(None);
    }
    if text.contains('"') {
        return words_only(Some("quoted phrases search by words".into()));
    }
    let Some(model) = session.with_vault(|v| v.embedding_model_ready())? else {
        return words_only(Some("no model is ready".into()));
    };
    if !index_serves(session, &model) {
        return words_only(Some("the meaning index is loading".into()));
    }
    let query = match models.query_embedder(&model).and_then(|e| e.embed_query(text)) {
        Ok(v) => Zeroizing::new(v),
        Err(e) => return words_only(Some(e.to_string())),
    };
    if !session.search_is_current(ticket) {
        return Ok(None);
    }
    let Some(allowed) = session.run_search(ticket, |v, _| Ok(Some(v.search_filter_ids(q)?.map(|ids| Allowed::new(&ids)))))?
    else {
        return Ok(None);
    };
    let candidates = {
        let index = session.related_index();
        match index.as_ref().filter(|i| i.model() == Some(model.as_str())) {
            Some(index) => index.search(&query, VECTOR_LIMIT, allowed.as_ref()),
            None => return words_only(Some("the meaning index is loading".into())),
        }
    };
    let threshold = embed::descriptor(&model).map_or(0.5, |d| d.meaning_threshold);
    let limit = q.limit.unwrap_or(200) as usize;
    let offset = q.offset as usize;
    session.run_search(ticket, |v, stop| {
        // The index holds `i8` vectors: the candidates get their exact cosine.
        let vectors = v.embedding_vectors(&model, &candidates)?;
        let meaning = related::rerank(&query, &candidates, &vectors, VECTOR_LIMIT);
        let Some((mut words, _)) = v.search_word_ids(q, FUSED_LIMIT as u32 + 1, stop)? else { return Ok(None) };
        let words_cut = words.len() > FUSED_LIMIT;
        words.truncate(FUSED_LIMIT);
        let fused = fuse(&words, &meaning, threshold, FUSED_LIMIT);
        let page = fused.hits.get(offset..).unwrap_or_default();
        let page = &page[..page.len().min(limit)];
        let Some(hits) = v.meaning_hits(q, page, stop)? else { return Ok(None) };
        Ok(Some(SearchPage {
            hits,
            newest_first: false,
            by_meaning: true,
            top: (fused.cut || words_cut).then_some(FUSED_LIMIT),
            words_only: None,
        }))
    })
}

/// Whether the in-memory index holds `model`'s vectors (the check of `classify::review`).
fn index_serves(session: &Session, model: &str) -> bool {
    session.related_index().as_ref().is_some_and(|i| i.model() == Some(model) && !i.is_empty())
}

/// The fusion score of 0-based `rank`.
fn rrf(rank: usize) -> f32 {
    1.0 / (RRF_K + rank as f32 + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(f: &Fused) -> Vec<i64> {
        f.hits.iter().map(|h| h.id).collect()
    }

    #[test]
    fn a_note_in_both_lists_ranks_above_notes_in_one() {
        // 3 is second in both lists; 1 and 9 lead one list each.
        let f = fuse(&[1, 3, 4], &[(9, 0.9), (3, 0.8)], 0.5, 10);
        assert_eq!(ids(&f), vec![3, 1, 9, 4]);
        assert_eq!(f.hits[0], FusedHit { id: 3, words: true, cosine: Some(0.8) });
        assert_eq!(f.hits[1], FusedHit { id: 1, words: true, cosine: None });
        assert_eq!(f.hits[2], FusedHit { id: 9, words: false, cosine: Some(0.9) });
        assert!(!f.cut);
    }

    #[test]
    fn equal_ranks_tie_to_words_then_lower_id() {
        // 7 and 2 lead one list each: the same score.
        assert_eq!(ids(&fuse(&[7], &[(2, 0.9)], 0.0, 10)), vec![7, 2]);
        // Both from the vector list at different ranks: rank decides, not id.
        assert_eq!(ids(&fuse(&[], &[(8, 0.9), (2, 0.8)], 0.0, 10)), vec![8, 2]);
        // Equal sums from different ranks (1st + 3rd for both 5 and 4): the lower id.
        let f = fuse(&[5, 6, 4], &[(4, 0.9), (6, 0.8), (5, 0.7)], 0.0, 10);
        // 5: 1/61 + 1/63; 6: 1/62 + 1/62; 4: 1/63 + 1/61. 6 scores lowest (convexity).
        assert_eq!(ids(&f), vec![4, 5, 6]);
    }

    #[test]
    fn vector_hits_below_the_threshold_do_not_count_or_take_a_rank() {
        let f = fuse(&[1], &[(2, 0.2), (3, 0.6)], 0.5, 10);
        assert_eq!(ids(&f), vec![1, 3], "2 is below the threshold; 3 takes rank 1, tying with 1");
        // Nothing above it: the words hits alone, in their order.
        assert_eq!(ids(&fuse(&[4, 5], &[(6, 0.1)], 0.5, 10)), vec![4, 5]);
        // NaN never counts.
        assert_eq!(ids(&fuse(&[], &[(6, f32::NAN)], 0.0, 10)), Vec::<i64>::new());
    }

    #[test]
    fn the_list_is_cut_at_the_limit() {
        let words: Vec<i64> = (1..=5).collect();
        let f = fuse(&words, &[(9, 0.9)], 0.0, 3);
        assert_eq!(f.hits.len(), 3);
        assert!(f.cut);
        assert!(!fuse(&words, &[], 0.0, 5).cut, "exactly the limit is not cut");
    }

    #[test]
    fn duplicates_count_once() {
        let f = fuse(&[1, 1, 2], &[(2, 0.9), (2, 0.8)], 0.0, 10);
        assert_eq!(f.hits.len(), 2);
        assert_eq!(f.hits.iter().find(|h| h.id == 2).unwrap().cosine, Some(0.9));
    }
}
