//! The background job that keeps every note's vector up to date. It starts after
//! unlock, embeds notes with no vector for the active model (or a stale one), then
//! waits for the vault to change. It reaches the vault only through
//! [`Session::with_vault_quiet`], so it never postpones auto-lock, and only to read a
//! batch or write its results: never while the model runs. Once a model is ready it
//! also embeds the tag descriptions that zero-shot suggestions compare notes with, and
//! after each pass it brings the related-notes index ([`super::related`]) up to date.
//!
//! It works from the vault's embedding queue (`embedding_dirty`), which triggers fill
//! in the same transaction as every write that can change a note's embedded text, so
//! a save costs one queued note, not a scan of the vault.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::{EmbedError, Embedder, related};
use crate::db;
use crate::session::{Session, SessionError};

/// Notes read, embedded and written together.
pub const BATCH: usize = 32;
/// Queued (or, in the safety-net scan, all) notes checked per vault access.
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
    /// Notes embedded during the pass (queued, and without an up-to-date vector).
    pub stale: usize,
    /// Live notes in the vault, when the pass showed progress (else 0).
    pub total: usize,
    /// Tag descriptions embedded during the pass.
    pub descriptions: usize,
    /// Whether the related-notes index changed.
    pub related_changed: bool,
}

/// Runs the job for unlock `generation` until the vault locks or the job fails.
/// `poll` is how often it checks whether there is work; `report` gets every status
/// change.
pub fn run(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    poll: Duration,
    report: impl Fn(&IndexStatus),
) {
    run_with(session, generation, source, poll, report, || {});
}

/// [`run`], also calling `related_changed` whenever the related-notes index changed.
///
/// The first pass runs at once (it builds the related-notes index); after that a pass
/// runs only when [`crate::db::Vault::embedding_pending`] says so, so the job's own
/// writes never wake it. Once the queue is empty it runs the safety-net scan
/// ([`safety_net`]) once per unlock, a page at a time, and lets queued notes go first.
pub fn run_with(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    poll: Duration,
    report: impl Fn(&IndexStatus),
    related_changed: impl Fn(),
) {
    let mut last: Option<IndexStatus> = None;
    let mut report = |status: IndexStatus| {
        if last.as_ref() != Some(&status) {
            report(&status);
            last = Some(status);
        }
    };
    let fail = |e: JobError, report: &mut dyn FnMut(IndexStatus)| {
        if !matches!(e, JobError::Locked) {
            // No note content in the message: only file names and error kinds.
            eprintln!("hush: indexing stopped: {e}");
            report(IndexStatus::Failed { message: e.to_string() });
        }
    };
    let mut first = true;
    // Where the safety-net scan continues; `None` once it is done.
    let mut scan = Some(0);
    let mut found = 0;
    loop {
        if !first {
            let pending = match session.with_vault_quiet(generation, |v| v.embedding_pending()) {
                Ok(p) => p,
                Err(_) => return,
            };
            if !pending {
                if let Some(after) = scan {
                    match safety_net_page(session, generation, after) {
                        Ok((stale, next)) => {
                            found += stale;
                            scan = next;
                            if scan.is_none() && found > 0 {
                                eprintln!("hush: safety-net scan queued {found} notes that no write had queued");
                            }
                        }
                        Err(e) => return fail(e, &mut report),
                    }
                    continue;
                }
                if !sleep_while_open(session, generation, poll) {
                    return;
                }
                continue;
            }
        }
        first = false;
        match pass(session, generation, source, &mut report) {
            Ok(p) if p.related_changed => related_changed(),
            Ok(_) => {}
            Err(e) => return fail(e, &mut report),
        }
    }
}

/// The whole safety-net scan at once: checks every live note's stored hash against its
/// text and queues the notes some code path changed without queuing them. Returns how
/// many it queued.
pub fn safety_net(session: &Session, generation: u64) -> Result<usize, JobError> {
    let mut found = 0;
    let mut after = Some(0);
    while let Some(a) = after {
        let (stale, next) = safety_net_page(session, generation, a)?;
        found += stale;
        after = next;
    }
    Ok(found)
}

/// One page of the safety-net scan, after note `after`: how many notes it queued, and
/// where to continue.
fn safety_net_page(session: &Session, generation: u64, after: i64) -> Result<(usize, Option<i64>), JobError> {
    if !session.is_open(generation) {
        return Err(JobError::Locked);
    }
    Ok(session.with_vault_quiet(generation, |v| {
        let model = v.embedding_model()?;
        let page = v.scan_embeddings(&model, after, SCAN_PAGE)?;
        v.queue_embeddings(&page.stale)?;
        Ok((page.stale.len(), page.next))
    })?)
}

/// One pass: embeds the queued notes whose vector for the active model is missing or
/// stale (the others just leave the queue). When the queue is empty, the active
/// model's vectors are complete: it becomes the ready model and other models' vectors
/// are deleted. Ends by syncing the related-notes index.
pub fn pass(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<Pass, JobError> {
    let (model, ready, queued) = session.with_vault_quiet(generation, |v| {
        Ok((v.embedding_model()?, v.embedding_model_ready()?, v.embedding_queue_len()?))
    })?;
    let embedder = source.embedder(&model)?;

    let show = queued >= PROGRESS_MIN;
    // Counting every note costs a scan of `notes`: only done for the progress line.
    let total = if show { session.with_vault_quiet(generation, |v| v.count_in_folder(""))? } else { 0 };
    let mut done = total.saturating_sub(queued);
    let mut stale = 0;
    // Notes whose vector changed or went away, for the related-notes index.
    let mut changed = session.with_vault_quiet(generation, |v| v.take_deleted_notes())?;
    let mut after = 0;
    loop {
        let page = session.with_vault_quiet(generation, |v| v.queued_embeddings(&model, after, SCAN_PAGE))?;
        done += page.settled;
        changed.extend(&page.dropped);
        for batch in page.stale.chunks(BATCH) {
            if show {
                report(IndexStatus::Indexing { done: done.min(total), total });
            }
            let mut vectors = Vec::with_capacity(batch.len());
            for queued in batch {
                if !session.is_open(generation) {
                    return Err(JobError::Locked);
                }
                let note = &queued.note;
                vectors.push(db::QueuedVector {
                    id: note.id,
                    version: queued.version,
                    hash: note.hash,
                    vector: embedder.embed(&note.text)?,
                });
            }
            session.with_vault_quiet(generation, |v| v.write_queued_embeddings(&model, &vectors))?;
            changed.extend(vectors.iter().map(|v| v.id));
            done += batch.len();
            stale += batch.len();
        }
        drop(page.stale);
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    if show {
        report(IndexStatus::Indexing { done: total, total });
    }

    let empty = session.with_vault_quiet(generation, |v| v.embedding_queue_len())? == 0;
    let mut ready_now = ready.as_deref() == Some(model.as_str());
    if empty && !ready_now {
        session.with_vault_quiet(generation, |v| v.finish_embedding_model(&model))?;
        ready_now = true;
    }
    // Mid-switch the descriptions keep the ready model's vectors until the switch is done.
    let descriptions = if ready_now { embed_descriptions(session, generation, &model, embedder.as_ref())? } else { 0 };
    if empty {
        report(IndexStatus::Idle);
    }
    // Only the notes this pass touched; a full rebuild when the ready model changed or
    // the index isn't built yet.
    let related_changed = related::update(session, generation, &changed)?;
    Ok(Pass { stale, total, descriptions, related_changed })
}

/// Embeds the tag descriptions that have no `model` vector yet. Only called while
/// `model` is the ready model, so descriptions and the note vectors they are compared
/// with always come from the same model. Returns how many were embedded.
fn embed_descriptions(
    session: &Session,
    generation: u64,
    model: &str,
    embedder: &dyn Embedder,
) -> Result<usize, JobError> {
    let stale = session.with_vault_quiet(generation, |v| v.stale_tag_descriptions(model))?;
    let mut vectors = Vec::with_capacity(stale.len());
    for (tag, description) in stale {
        if !session.is_open(generation) {
            return Err(JobError::Locked);
        }
        let vector = embedder.embed_query(&description)?;
        vectors.push((tag, description, vector));
    }
    if !vectors.is_empty() {
        session.with_vault_quiet(generation, |v| v.write_tag_description_vectors(model, &vectors))?;
    }
    Ok(vectors.len())
}

/// Sleeps for `duration`, waking early if the vault locks. Returns whether it is still open.
pub(crate) fn sleep_while_open(session: &Session, generation: u64, duration: Duration) -> bool {
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
