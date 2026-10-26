//! The background job that keeps every note's vector up to date. It starts after
//! unlock, embeds notes with no vector for the active model (or a stale one), then
//! waits for the vault to change. It reaches the vault only through
//! [`Session::with_vault_quiet`], so it never postpones auto-lock, and only to read a
//! batch or write its results: never while the model runs. Once a model is ready it
//! also embeds the tag descriptions that zero-shot suggestions compare notes with, and
//! after each pass it brings the related-notes index ([`super::related`]) up to date.
//!
//! While an import runs ([`Session::begin_import`]) it starts no work and stops a pass
//! between batches; the queue keeps filling, and it carries on once the import ends.
//!
//! It works from the vault's embedding queue (`embedding_dirty`), which triggers fill
//! in the same transaction as every write that can change a note's embedded text, so
//! a save costs one queued note, not a scan of the vault.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{EmbedError, Embedder, related};
use crate::activity::{Job, count, duration_text};
use crate::db;
use crate::session::{Session, SessionError};

/// Notes read, embedded and written together.
pub const BATCH: usize = 32;
/// Queued (or, in the safety-net scan, all) notes checked per vault access.
const SCAN_PAGE: u32 = 500;
/// Fewer stale notes than this are embedded without a progress line, so an autosave
/// doesn't flash one.
const PROGRESS_MIN: usize = 10;
/// Pause between safety-net pages. The vault mutex isn't fair: without a gap the job
/// takes it back before a waiting lookup wakes, and at 1M notes related notes waited
/// up to 3 s for the 33 s the scan takes.
const SCAN_PAUSE: Duration = Duration::from_millis(5);
/// How often a waiting job looks at the lock state (cheap, no vault access).
const STOP_CHECK: Duration = Duration::from_millis(100);

/// Gives the job an embedder for a model ID.
pub trait EmbedderSource: Send + Sync {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError>;

    /// Whether [`EmbedderSource::embedder`] would return at once, without loading
    /// `model` (for the Activity panel).
    fn is_loaded(&self, _model: &str) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum IndexStatus {
    Idle,
    /// `eta_secs`: the time left at the rate of the last minute, once there are ten
    /// seconds of it to go by.
    Indexing { done: usize, total: usize, eta_secs: Option<u64> },
    /// An import is running; indexing carries on when it ends.
    Paused,
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
    /// Whether the pass stopped early because an import started.
    pub paused: bool,
}

/// The rate over the last [`Eta::WINDOW`], for the time left.
#[derive(Default)]
pub struct Eta {
    samples: VecDeque<(Instant, usize)>,
}

impl Eta {
    const WINDOW: Duration = Duration::from_secs(60);
    const MIN_SPAN: Duration = Duration::from_secs(10);

    /// Records that `done` of `total` are done at `now`; the seconds left, if known.
    pub fn update(&mut self, now: Instant, done: usize, total: usize) -> Option<u64> {
        self.samples.push_back((now, done));
        while self.samples.len() > 2 && self.samples.get(1).is_some_and(|s| now.duration_since(s.0) >= Self::WINDOW) {
            self.samples.pop_front();
        }
        let &(start, from) = self.samples.front()?;
        let span = now.duration_since(start);
        if span < Self::MIN_SPAN || done <= from {
            return None;
        }
        let rate = (done - from) as f64 / span.as_secs_f64();
        Some((total.saturating_sub(done) as f64 / rate).ceil() as u64)
    }
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
    let activity = session.activity().reporter(generation);
    let fail = |e: JobError, report: &mut dyn FnMut(IndexStatus)| {
        if !matches!(e, JobError::Locked) {
            // No note content in the message: only file names and error kinds.
            eprintln!("hush: indexing stopped: {e}");
            activity.fail(Job::Embedding, e.to_string());
            report(IndexStatus::Failed { message: e.to_string() });
        }
    };
    let mut first = true;
    // Where the safety-net scan continues; `None` once it is done.
    let mut scan = Some(0);
    let mut found = 0;
    let mut checked = 0;
    let mut paused = false;
    activity.wait(Job::SafetyNet, "runs once the queue is empty");
    loop {
        if session.import_running() {
            paused = true;
            report(IndexStatus::Paused);
            activity.pause(Job::Embedding, "import running");
            if scan.is_some() {
                activity.wait(Job::SafetyNet, "import running");
            }
            if !sleep_while_open(session, generation, poll) {
                return;
            }
            continue;
        }
        if paused {
            paused = false;
            report(IndexStatus::Idle);
        }
        if !first {
            let pending = match session.with_vault_quiet(generation, |v| v.embedding_pending()) {
                Ok(p) => p,
                Err(_) => return,
            };
            if !pending {
                activity.idle(Job::Embedding, Some("nothing queued"));
                if let Some(after) = scan {
                    match safety_net_page(session, generation, after) {
                        Ok((stale, embeddable, next)) => {
                            found += stale;
                            checked += embeddable;
                            scan = next;
                            if scan.is_none() {
                                if found > 0 {
                                    eprintln!("hush: safety-net scan queued {found} notes that no write had queued");
                                }
                                activity.finish(
                                    Job::SafetyNet,
                                    format!(
                                        "checked {} notes, queued {} that no write had queued",
                                        count(checked as u64),
                                        count(found as u64)
                                    ),
                                );
                            } else {
                                activity.progress(
                                    Job::SafetyNet,
                                    format!("checking stored hashes, {} queued so far", count(found as u64)),
                                    checked as u64,
                                    None,
                                );
                            }
                        }
                        Err(e) => {
                            if !matches!(e, JobError::Locked) {
                                activity.fail(Job::SafetyNet, e.to_string());
                            }
                            return fail(e, &mut report);
                        }
                    }
                    std::thread::sleep(SCAN_PAUSE);
                    continue;
                }
                if !sleep_while_open(session, generation, poll) {
                    return;
                }
                continue;
            }
            if scan.is_some_and(|a| a > 0) {
                activity.pause(Job::SafetyNet, "queued notes go first");
            }
        }
        first = false;
        match pass(session, generation, source, &mut report) {
            Ok(p) => {
                paused = p.paused;
                if p.related_changed {
                    related_changed();
                }
            }
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
        let (stale, _, next) = safety_net_page(session, generation, a)?;
        found += stale;
        after = next;
    }
    Ok(found)
}

/// One page of the safety-net scan, after note `after`: how many notes it queued, how
/// many it checked, and where to continue.
fn safety_net_page(session: &Session, generation: u64, after: i64) -> Result<(usize, usize, Option<i64>), JobError> {
    if !session.is_open(generation) {
        return Err(JobError::Locked);
    }
    Ok(session.with_vault_quiet(generation, |v| {
        let model = v.embedding_model()?;
        let page = v.scan_embeddings(&model, after, SCAN_PAGE)?;
        v.queue_embeddings(&page.stale)?;
        Ok((page.stale.len(), page.embeddable, page.next))
    })?)
}

/// One pass: embeds the queued notes whose vector for the active model is missing or
/// stale (the others just leave the queue). When the queue is empty, the active
/// model's vectors are complete: it becomes the ready model and other models' vectors
/// are deleted. Ends by syncing the related-notes index. Stops between batches once an
/// import runs (see [`Pass::paused`]), leaving the rest queued.
pub fn pass(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<Pass, JobError> {
    let activity = session.activity().reporter(generation);
    let started = Instant::now();
    let (model, ready, queued) = session.with_vault_quiet(generation, |v| {
        Ok((v.embedding_model()?, v.embedding_model_ready()?, v.embedding_queue_len()?))
    })?;
    let loading = !source.is_loaded(&model);
    if loading {
        activity.start(Job::Embedding, format!("loading model {model}"));
    }
    let embedder = source.embedder(&model)?;
    if loading {
        activity.log(Job::Embedding, format!("model {model} loaded ({})", duration_text(started.elapsed())));
    }
    if queued > 0 {
        activity.start(Job::Embedding, format!("embedding {} queued notes with {model}", count(queued as u64)));
    }

    let show = queued >= PROGRESS_MIN;
    // Counting every note costs a scan of `notes`: only done for the progress line.
    let total = if show { session.with_vault_quiet(generation, |v| v.count_in_folder(""))? } else { 0 };
    let mut done = total.saturating_sub(queued);
    let mut stale = 0;
    // Notes whose vector changed or went away, for the related-notes index.
    let mut changed = session.with_vault_quiet(generation, |v| v.take_deleted_notes())?;
    let mut eta = Eta::default();
    let mut after = db::QueueCursor::default();
    let mut paused = false;
    // The queue as this pass works through it, for the Activity panel.
    let mut handled = 0;
    let mut page_number = 0;
    'pages: loop {
        let page = session.with_vault_quiet(generation, |v| v.queued_embeddings(&model, &after, SCAN_PAGE))?;
        page_number += 1;
        done += page.settled;
        handled += page.settled + page.dropped.len();
        changed.extend(&page.dropped);
        let batches = page.stale.len().div_ceil(BATCH);
        for (b, batch) in page.stale.chunks(BATCH).enumerate() {
            if show {
                let done = done.min(total);
                report(IndexStatus::Indexing { done, total, eta_secs: eta.update(Instant::now(), done, total) });
            }
            activity.progress(
                Job::Embedding,
                format!("embedding batch {} of {batches} on queue page {page_number}", b + 1),
                handled as u64,
                Some(queued.max(handled) as u64),
            );
            if !session.is_open(generation) {
                return Err(JobError::Locked);
            }
            if session.import_running() {
                paused = true;
                break 'pages;
            }
            // One call for the batch: the model runs chunks of several notes together.
            let texts: Vec<&str> = batch.iter().map(|q| q.note.text.as_str()).collect();
            let stop = || !session.is_open(generation) || session.import_running();
            let embedded = match embedder.embed_batch(&texts, &stop) {
                Err(EmbedError::Stopped) if session.is_open(generation) => {
                    paused = true;
                    break 'pages;
                }
                Err(EmbedError::Stopped) => return Err(JobError::Locked),
                result => result?,
            };
            let vectors: Vec<db::QueuedVector> = batch
                .iter()
                .zip(embedded)
                .map(|(queued, vector)| db::QueuedVector {
                    id: queued.note.id,
                    version: queued.version,
                    hash: queued.note.hash,
                    vector,
                })
                .collect();
            session.with_vault_quiet(generation, |v| v.write_queued_embeddings(&model, &vectors))?;
            changed.extend(vectors.iter().map(|v| v.id));
            done += batch.len();
            stale += batch.len();
            handled += batch.len();
        }
        drop(page.stale);
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    if paused {
        report(IndexStatus::Paused);
        activity.pause(Job::Embedding, "import running");
        activity.log(Job::Embedding, format!("paused for an import after embedding {} notes", count(stale as u64)));
        // The notes embedded so far still go into the related-notes index.
        let related_changed = related::update(session, generation, &changed)?;
        return Ok(Pass { stale, total, descriptions: 0, related_changed, paused });
    }
    if show {
        report(IndexStatus::Indexing { done: total, total, eta_secs: Some(0) });
    }

    let empty = session.with_vault_quiet(generation, |v| v.embedding_queue_len())? == 0;
    let mut ready_now = ready.as_deref() == Some(model.as_str());
    if empty && !ready_now {
        session.with_vault_quiet(generation, |v| v.finish_embedding_model(&model))?;
        ready_now = true;
        activity.log(Job::Embedding, format!("every note has a {model} vector: it is now the ready model"));
    }
    // Mid-switch the descriptions keep the ready model's vectors until the switch is done.
    let descriptions = if ready_now { embed_descriptions(session, generation, &model, embedder.as_ref())? } else { 0 };
    if descriptions > 0 {
        activity.log(Job::Embedding, format!("embedded {} tag descriptions", count(descriptions as u64)));
    }
    if stale > 0 || queued > 0 {
        let secs = started.elapsed().as_secs_f64();
        let summary = if stale > 0 && secs > 0.0 {
            format!("embedded {} notes ({:.1} notes/s)", count(stale as u64), stale as f64 / secs)
        } else {
            format!("embedded {} notes", count(stale as u64))
        };
        if stale >= PROGRESS_MIN {
            activity.finish(Job::Embedding, summary);
        } else {
            activity.finish_quietly(Job::Embedding, summary);
        }
    }
    if empty {
        report(IndexStatus::Idle);
        activity.idle(Job::Embedding, Some("nothing queued"));
    }
    // Only the notes this pass touched; a full rebuild when the ready model changed or
    // the index isn't built yet.
    let related_changed = related::update(session, generation, &changed)?;
    Ok(Pass { stale, total, descriptions, related_changed, paused })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_left_comes_from_the_last_minute() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut eta = Eta::default();
        assert_eq!(eta.update(at(0), 0, 1_000), None);
        assert_eq!(eta.update(at(5), 50, 1_000), None, "under ten seconds of rate");
        // 10 notes/s: 900 left is 90 s.
        assert_eq!(eta.update(at(10), 100, 1_000), Some(90));
        // It slows to 1 note/s; after a minute only the slow rate counts.
        for s in 11..=80 {
            eta.update(at(s), 100 + (s as usize - 10), 1_000);
        }
        assert_eq!(eta.update(at(81), 171, 1_000), Some(829));
        // Nothing done in the window: unknown.
        let mut stuck = Eta::default();
        stuck.update(at(0), 10, 100);
        assert_eq!(stuck.update(at(30), 10, 100), None);
    }
}
