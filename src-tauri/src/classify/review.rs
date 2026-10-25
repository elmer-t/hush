//! The Suggestions view as the app runs it (see `db::review`): the tag's scorer is
//! loaded from the vault once, applied to the in-memory vectors of the related-notes
//! index (one dot product per note, without holding the vault), and the vault is taken
//! again to drop excluded notes and look up titles. Until the embedding job has built
//! the index for the ready model, the vectors are read from the vault instead.

use crate::db::{ReviewOrder, SuggestionReview};
use crate::session::{Session, SessionError};

/// One tag's suggestions across the vault, the first `limit` in `order`. Counts as
/// activity: the user asked for it.
pub fn review(session: &Session, tag: &str, order: ReviewOrder, limit: usize) -> Result<SuggestionReview, SessionError> {
    let (scorer, model) = session.with_vault(|v| Ok((v.tag_scorer(tag)?, v.embedding_model_ready()?)))?;
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
        let scores = match scores {
            Some(s) => s,
            None => v.score_notes(&scorer)?,
        };
        v.suggestion_review_of(tag, Some(&scorer), &scores, order, limit)
    })
}
