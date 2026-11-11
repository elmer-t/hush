//! The background pass that re-derives every note's tags once per tag rules version
//! ([`crate::markdown::TAG_RULES_VERSION`]), so tags older rules let in (`#a`, `#fff;`,
//! `#1`) leave `note_tags`. It starts on unlock and does nothing on a vault whose tags
//! already follow the current rules. Same rules as the embedding and training jobs: it
//! reaches the vault only through [`Session::with_vault_quiet`] (no activity, so it never
//! postpones auto-lock), a page at a time with a pause between pages so the app's own
//! reads go first, it holds off while an import runs, and it stops on lock. A pass cut
//! off by a lock continues where it was at the next unlock.
//!
//! Note bodies are only read: tags are derived from them, and only the derived rows are
//! rewritten (see `db::retag`).

use std::time::Duration;

use crate::activity::{Job, count};
use crate::db;
use crate::embed::job::sleep_while_open;
use crate::session::{Session, SessionError};

/// Notes per vault access.
pub const PAGE: u32 = 500;
/// Pause between pages: the vault mutex isn't fair (see `embed::job`).
const PAGE_PAUSE: Duration = Duration::from_millis(5);

#[derive(Debug, thiserror::Error)]
pub enum RetagError {
    /// The vault was locked (or another one unlocked): stop quietly.
    #[error("vault is locked")]
    Locked,
    #[error(transparent)]
    Db(#[from] db::Error),
}

impl From<SessionError> for RetagError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Db(e) => Self::Db(e),
            _ => Self::Locked,
        }
    }
}

/// What a finished pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Retag {
    /// Live notes read during this run (a pass resumed after a lock counts only the rest).
    pub checked: usize,
    /// Notes whose tags changed.
    pub changed: usize,
    pub end: db::RetagEnd,
}

/// Runs the pass for unlock `generation` if the vault needs it: `Ok(None)` when its
/// tags already follow the current rules. While an import runs it checks again every
/// `poll`. Fails with [`RetagError::Locked`] once the vault locks.
pub fn run(session: &Session, generation: u64, poll: Duration) -> Result<Option<Retag>, RetagError> {
    let activity = session.activity().reporter(generation);
    let Some(mut after) = session.with_vault_quiet(generation, |v| v.retag_start())? else {
        return Ok(None);
    };
    let total = session.with_vault_quiet(generation, |v| v.live_note_count())? as u64;
    let mut result = Retag::default();
    loop {
        if let Some(reason) = session.pause_reason() {
            activity.pause(Job::Retag, reason);
            if !sleep_while_open(session, generation, poll) {
                return Err(RetagError::Locked);
            }
            continue;
        }
        if !session.is_open(generation) {
            return Err(RetagError::Locked);
        }
        let page = session.with_vault_quiet(generation, |v| v.retag_page(after, PAGE))?;
        result.checked += page.checked;
        result.changed += page.changed;
        let Some(next) = page.next else { break };
        after = next;
        activity.progress(
            Job::Retag,
            format!("re-deriving tags, {} notes changed so far", count(result.changed as u64)),
            result.checked as u64,
            Some(total),
        );
        if !sleep_while_open(session, generation, PAGE_PAUSE) {
            return Err(RetagError::Locked);
        }
    }
    result.end = session.with_vault_quiet(generation, |v| v.finish_retag())?;
    activity.finish(
        Job::Retag,
        format!(
            "re-derived tags under new tag rules: {} notes checked, {} changed, {} tags and {} classifiers removed",
            count(result.checked as u64),
            count(result.changed as u64),
            count(result.end.tags_dropped as u64),
            count(result.end.heads_dropped as u64)
        ),
    );
    Ok(Some(result))
}
