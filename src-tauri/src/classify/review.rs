//! The Suggestions view as the app runs it (see `db::review`): the tag's scorer is
//! loaded from the vault once, applied to the in-memory vectors of the related-notes
//! index (one dot product per note, without holding the vault), and the vault is taken
//! again to drop excluded notes, look up titles and give the notes shown their exact
//! score (the index holds `i8` vectors). Until the embedding job has built
//! the index for the ready model, the vectors are read from the vault instead.

use crate::db::{ReviewOrder, SuggestionReview, sort_review};
use crate::session::{Session, SessionError};

/// One tag's suggestions across the vault, the first `limit` in `order`. Counts as
/// activity: the user asked for it.
pub fn review(
    session: &Session,
    tag: &str,
    order: ReviewOrder,
    limit: usize,
) -> Result<SuggestionReview, SessionError> {
    let (scorer, model) =
        session.with_vault(|v| Ok((v.tag_scorer(tag)?, v.embedding_model_ready()?)))?;
    let Some(scorer) = scorer else {
        return session.with_vault(|v| v.suggestion_review_of(tag, None, &[], order, limit));
    };
    // The index lock is released before the vault is taken again.
    let scores = {
        let index = session.related_index();
        index
            .as_ref()
            .filter(|i| model.is_some() && i.model() == model.as_deref() && !i.is_empty())
            .map(|i| i.score_all(|x| scorer.score(x)))
    };
    session.with_vault(|v| {
        let Some(scores) = scores else {
            let scores = v.score_notes(&scorer)?;
            return v.suggestion_review_of(tag, Some(&scorer), &scores, order, limit);
        };
        let mut review = v.suggestion_review_of(tag, Some(&scorer), &scores, order, limit)?;
        // The index holds `i8` vectors, so its scores are close but not exact: the notes
        // shown get their exact score from the stored vectors and are re-sorted.
        if let Some(model) = &model {
            let ids: Vec<i64> = review.notes.iter().map(|n| n.id).collect();
            for stored in v.embedding_vectors(model, &ids)? {
                let mut x = stored.vector.clone();
                crate::embed::normalize(&mut x);
                if let Some(note) = review.notes.iter_mut().find(|n| n.id == stored.id) {
                    note.score = scorer.score(&x);
                }
            }
            let mut order_of: Vec<(i64, f32)> =
                review.notes.iter().map(|n| (n.id, n.score)).collect();
            sort_review(&mut order_of, order, scorer.threshold);
            review
                .notes
                .sort_by_key(|n| order_of.iter().position(|o| o.0 == n.id));
        }
        Ok(review)
    })
}
