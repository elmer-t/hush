//! The background job that keeps every note's vector up to date. It starts after
//! unlock, embeds notes with no vector for the active model (or a stale one), then
//! waits for the vault to change. It reaches the vault only through
//! [`Session::with_vault_quiet`], so it never postpones auto-lock, and only to read a
//! batch or write its results: never while the model runs.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::{EmbedError, Embedder};
use crate::db;
use crate::session::{Session, SessionError};

/// Notes read, embedded and written together.
pub const BATCH: usize = 32;
/// Notes checked per vault access while looking for stale vectors.
const SCAN_PAGE: u32 = 500;
/// Fewer stale notes than this are embedded without a progress line, so an autosave
/// doesn't flash one.
const PROGRESS_MIN: usize = 10;
/// How often a waiting job looks at the lock state (cheap, no vault access).
const STOP_CHECK: Duration = Duration::from_millis(100);

/// Gives the job an embedder for a model ID.
pub trait EmbedderSource: Send + Sync {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError>;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum IndexStatus {
    Idle,
    Indexing { done: usize, total: usize },
    /// The job stopped; the rest of Hush keeps working.
    Failed { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The vault was locked (or another one unlocked): stop quietly.
    #[error("vault is locked")]
    Locked,
    #[error(transparent)]
    Embed(#[from] EmbedError),
    #[error(transparent)]
    Db(#[from] db::Error),
}

impl From<SessionError> for JobError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Db(e) => Self::Db(e),
            _ => Self::Locked,
        }
    }
}

/// What one pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    /// Notes found without an up-to-date vector (all embedded during the pass).
    pub stale: usize,
    /// Notes with text to embed.
    pub total: usize,
}

/// Runs the job for unlock `generation` until the vault locks or the job fails.
/// `poll` is how often it checks whether the vault changed; `report` gets every
/// status change.
pub fn run(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    poll: Duration,
    report: impl Fn(&IndexStatus),
) {
    let mut last: Option<IndexStatus> = None;
    let mut report = |status: IndexStatus| {
        if last.as_ref() != Some(&status) {
            report(&status);
            last = Some(status);
        }
    };
    let mut seen = None;
    loop {
        let changes = match session.with_vault_quiet(generation, |v| Ok(v.change_count())) {
            Ok(c) => c,
            Err(_) => return,
        };
        if seen == Some(changes) {
            if !sleep_while_open(session, generation, poll) {
                return;
            }
            continue;
        }
        // Taken before the pass, so changes made during it trigger another one.
        seen = Some(changes);
        match pass(session, generation, source, &mut report) {
            Ok(_) => {}
            Err(JobError::Locked) => return,
            Err(e) => {
                // No note content in the message: only file names and error kinds.
                eprintln!("hush: indexing stopped: {e}");
                report(IndexStatus::Failed { message: e.to_string() });
                return;
            }
        }
    }
}

/// One pass: finds every note whose vector for the active model is missing or stale
/// and embeds it. When a pass finds nothing to do, the active model's vectors are
/// complete: it becomes the ready model and other models' vectors are deleted.
pub fn pass(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<Pass, JobError> {
    let (model, ready) =
        session.with_vault_quiet(generation, |v| Ok((v.embedding_model()?, v.embedding_model_ready()?)))?;
    let embedder = source.embedder(&model)?;

    let mut stale = Vec::new();
    let mut total = 0;
    let mut after = 0;
    loop {
        let page = session.with_vault_quiet(generation, |v| v.scan_embeddings(&model, after, SCAN_PAGE))?;
        total += page.embeddable;
        stale.extend(page.stale);
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }

    if stale.is_empty() {
        if ready.as_deref() != Some(model.as_str()) {
            session.with_vault_quiet(generation, |v| v.finish_embedding_model(&model))?;
        }
        report(IndexStatus::Idle);
        return Ok(Pass { stale: 0, total });
    }

    let show = stale.len() >= PROGRESS_MIN;
    let mut done = total - stale.len();
    for batch in stale.chunks(BATCH) {
        if show {
            report(IndexStatus::Indexing { done, total });
        }
        let texts = session.with_vault_quiet(generation, |v| v.embedding_texts(batch))?;
        let mut vectors = Vec::with_capacity(texts.len());
        for note in &texts {
            if !session.is_open(generation) {
                return Err(JobError::Locked);
            }
            vectors.push((note.id, note.hash, embedder.embed(&note.text)?));
        }
        drop(texts);
        session.with_vault_quiet(generation, |v| v.write_embeddings(&model, &vectors))?;
        done += batch.len();
    }
    if show {
        report(IndexStatus::Indexing { done: total, total });
    }
    Ok(Pass { stale: stale.len(), total })
}

/// Sleeps for `duration`, waking early if the vault locks. Returns whether it is still open.
fn sleep_while_open(session: &Session, generation: u64, duration: Duration) -> bool {
    let mut left = duration;
    while !left.is_zero() {
        if !session.is_open(generation) {
            return false;
        }
        let step = left.min(STOP_CHECK);
        std::thread::sleep(step);
        left -= step;
    }
    session.is_open(generation)
}
