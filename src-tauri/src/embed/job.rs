//! The background job that keeps every note's vector up to date. It starts after
//! unlock, embeds notes with no vector for the active model (or a stale one), then
//! waits for the vault to change. It reaches the vault only through
//! [`Session::with_vault_quiet`], so it never postpones auto-lock, and only to read a
//! batch or write its results: never while the model runs. Once a model is ready it
//! also embeds the tag descriptions that zero-shot suggestions compare notes with, and
//! after each pass it brings the related-notes index ([`super::related`]) up to date.
//!
//! While an import runs ([`Session::begin_import`]) or the vault is compacted
//! ([`Session::compact`]) it starts no work and stops a pass between batches; the queue
//! keeps filling, and it carries on once that ends.
//!
//! It works from the vault's embedding queue (`embedding_dirty`), which triggers fill
//! in the same transaction as every write that can change a note's embedded text, so
//! a save costs one queued note, not a scan of the vault.
//!
//! Indexing runs in two phases (see [`pass`]): every queued note first gets a vector
//! from its first chunk, then long notes get their full vector. On the CPU a chunk
//! takes about the same time whatever the note, so a backlog of long notes becomes
//! usable after one chunk's time per note instead of up to 64.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{EmbedError, Embedder, FirstChunk, related};
use crate::activity::{Job, count, duration_text};
use crate::db::{self, QueuePhase};
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
/// A pass embeds at least this many notes before its speed is passed on as the
/// model's rate: fewer is mostly start-up.
const RATE_MIN: usize = 50;

/// Gives the job an embedder for a model ID.
pub trait EmbedderSource: Send + Sync {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError>;

    /// Whether [`EmbedderSource::embedder`] would return at once, without loading
    /// `model` (for the Activity panel).
    fn is_loaded(&self, _model: &str) -> bool {
        true
    }

    /// The job read the vault's model settings: at the start of every pass, and when
    /// `model` became the ready model. For Settings → Model.
    fn vault_models(&self, _model: &str, _ready: Option<&str>) {}

    /// The job embedded enough notes with `model` to measure its speed on this machine.
    fn measured_rate(&self, _model: &str, _notes_per_sec: f64) {}
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum IndexStatus {
    Idle,
    /// `done` of `total` notes have a vector of their text, `complete` of them from
    /// all of it rather than just the first chunk (see [`pass`]). `eta_secs`: the time
    /// left for the phase running (until `done`, then `complete`, reaches `total`) at
    /// the rate of the last minute, once there are ten seconds of it to go by.
    Indexing { done: usize, complete: usize, total: usize, eta_secs: Option<u64> },
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
/// The related-notes index is built from the stored vectors first, before the model
/// loads or anything queued is embedded: those vectors are valid without the model, so
/// Related works right after unlock. The first pass runs at once; after that a pass
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
    // A failure here is left to the first pass, whose `related::update` tries again.
    if let Ok(true) = related::sync(session, generation) {
        related_changed();
    }
    loop {
        if let Some(reason) = session.pause_reason() {
            paused = true;
            report(IndexStatus::Paused);
            activity.pause(Job::Embedding, reason);
            if scan.is_some() {
                activity.wait(Job::SafetyNet, reason);
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

/// One pass over one indexing phase ([`db::QueuePhase`]). While any queued note waits
/// for a vector, the pass embeds the first chunk of each such note (a note of one chunk
/// gets its full vector and leaves the queue); otherwise it replaces first-chunk vectors
/// with full ones, a note at a time (a group of notes per call when the model runs
/// several notes together, see [`Embedder::embed_group`]), and ends early between calls
/// as soon as a save queues a note for its first chunk, so that note goes first. Queued notes whose vector is already up to
/// date just leave the queue.
///
/// Once no note waits for a first vector, the active model becomes the ready model
/// (its other models' vectors are deleted): every note has a usable vector, even if
/// some only from their first chunk. Ends by syncing the related-notes index. Stops
/// between batches once an import runs (see [`Pass::paused`]), leaving the rest queued.
pub fn pass(
    session: &Session,
    generation: u64,
    source: &dyn EmbedderSource,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<Pass, JobError> {
    let activity = session.activity().reporter(generation);
    let started = Instant::now();
    let (model, ready, counts) = session.with_vault_quiet(generation, |v| {
        Ok((v.embedding_model()?, v.embedding_model_ready()?, v.embedding_queue_counts()?))
    })?;
    let phase = if counts.first > 0 || counts.rest == 0 { QueuePhase::First } else { QueuePhase::Rest };
    source.vault_models(&model, ready.as_deref());
    let loading = !source.is_loaded(&model);
    if loading {
        activity.start(Job::Embedding, format!("loading model {model}"));
    }
    let embedder = source.embedder(&model)?;
    if loading {
        activity.log(Job::Embedding, format!("model {model} loaded ({})", duration_text(started.elapsed())));
    }
    let work = match phase {
        QueuePhase::First => counts.first,
        QueuePhase::Rest => counts.rest,
    };
    if work > 0 {
        activity.start(
            Job::Embedding,
            match phase {
                QueuePhase::First => format!("embedding the first chunk of {} queued notes with {model}", count(work as u64)),
                QueuePhase::Rest => format!("completing the vectors of {} notes with {model}", count(work as u64)),
            },
        );
    }
    let embedding_started = Instant::now();

    let show = counts.total() >= PROGRESS_MIN;
    // Counting every note costs a scan of `notes`: only done for the progress line.
    let total = if show { session.with_vault_quiet(generation, |v| v.count_in_folder(""))? } else { 0 };
    // Notes with a vector of their text, from the first chunk or all of it; and those
    // with the full vector.
    let mut done = total.saturating_sub(counts.first);
    let mut complete = total.saturating_sub(counts.total());
    let mut stale = 0;
    let mut partial = 0;
    // Notes whose vector changed or went away, for the related-notes index.
    let mut changed = session.with_vault_quiet(generation, |v| v.take_deleted_notes())?;
    let mut eta = Eta::default();
    let mut after = db::QueueCursor::default();
    let mut paused = false;
    let mut preempted = false;
    // The vault's change counter after the job's own last write: in the second phase,
    // only a change by someone else is worth looking for a note queued for its first
    // chunk. A change found just before the job's own write is remembered by leaving
    // `seen_changes` behind, so the next check still looks.
    let mut seen_changes = 0;
    // The queue as this pass works through it, for the Activity panel.
    let mut handled = 0;
    let mut page_number = 0;
    'pages: loop {
        let page = session.with_vault_quiet(generation, |v| {
            let foreign = v.change_count() != seen_changes;
            let page = v.queued_embeddings(&model, &after, SCAN_PAGE, phase)?;
            seen_changes = if foreign { u64::MAX } else { v.change_count() };
            Ok(page)
        })?;
        page_number += 1;
        match phase {
            QueuePhase::First => {
                done += page.settled + page.partial;
                complete += page.settled;
            }
            QueuePhase::Rest => complete += page.settled,
        }
        handled += page.settled + page.partial + page.dropped.len();
        changed.extend(&page.dropped);
        let batches = page.stale.len().div_ceil(BATCH);
        // The first page note not embedded yet. The first phase takes `BATCH` notes at a
        // time; the second as many as the model takes per call (`Embedder::embed_group`).
        let mut start = 0;
        while start < page.stale.len() {
            if show {
                let (done, complete) = (done.min(total), complete.min(total));
                let progress = if phase == QueuePhase::First { done } else { complete };
                report(IndexStatus::Indexing { done, complete, total, eta_secs: eta.update(Instant::now(), progress, total) });
            }
            activity.progress(
                Job::Embedding,
                match phase {
                    QueuePhase::First => {
                        format!("first chunks: batch {} of {batches} on queue page {page_number}", start / BATCH + 1)
                    }
                    QueuePhase::Rest => format!(
                        "remaining chunks: note {} of {} on queue page {page_number}",
                        start + 1,
                        page.stale.len()
                    ),
                },
                handled as u64,
                Some(work.max(handled) as u64),
            );
            if !session.is_open(generation) {
                return Err(JobError::Locked);
            }
            if session.pause_reason().is_some() {
                paused = true;
                break 'pages;
            }
            if phase == QueuePhase::Rest {
                // Once per call, so once per group of notes when the model groups them.
                let first_waiting = session.with_vault_quiet(generation, |v| {
                    let changes = v.change_count();
                    if changes == seen_changes {
                        return Ok(false);
                    }
                    seen_changes = changes;
                    v.embedding_first_pending()
                })?;
                if first_waiting {
                    preempted = true;
                    break 'pages;
                }
            }
            let candidates = match phase {
                QueuePhase::First => &page.stale[start..page.stale.len().min(start + BATCH)],
                QueuePhase::Rest => &page.stale[start..],
            };
            let texts: Vec<&str> = candidates.iter().map(|q| q.note.text.as_str()).collect();
            let stop = || !session.is_open(generation) || session.pause_reason().is_some();
            // One call: the model may run several notes' chunks together.
            let embedded = match phase {
                QueuePhase::First => embedder.embed_first_batch(&texts, &stop),
                QueuePhase::Rest => embedder
                    .embed_group(&texts, &stop)
                    .map(|vectors| vectors.into_iter().map(|vector| FirstChunk { vector, complete: true }).collect()),
            };
            let embedded = match embedded {
                Err(EmbedError::Stopped) if session.is_open(generation) => {
                    paused = true;
                    break 'pages;
                }
                Err(EmbedError::Stopped) => return Err(JobError::Locked),
                result => result?,
            };
            if embedded.is_empty() || embedded.len() > candidates.len() {
                return Err(EmbedError::Inference(format!(
                    "the model returned {} vectors for {} notes",
                    embedded.len(),
                    candidates.len()
                ))
                .into());
            }
            let batch = &candidates[..embedded.len()];
            let vectors: Vec<db::QueuedVector> = batch
                .iter()
                .zip(embedded)
                .map(|(queued, chunk)| db::QueuedVector {
                    id: queued.note.id,
                    version: queued.version,
                    hash: queued.note.hash,
                    vector: chunk.vector,
                    partial: !chunk.complete,
                })
                .collect();
            seen_changes = session.with_vault_quiet(generation, |v| {
                let foreign = v.change_count() != seen_changes;
                v.write_queued_embeddings(&model, &vectors)?;
                Ok(if foreign { u64::MAX } else { v.change_count() })
            })?;
            changed.extend(vectors.iter().map(|v| v.id));
            let partial_now = vectors.iter().filter(|v| v.partial).count();
            partial += partial_now;
            if phase == QueuePhase::First {
                done += batch.len();
            }
            complete += batch.len() - partial_now;
            stale += batch.len();
            handled += batch.len();
            start += batch.len();
        }
        drop(page.stale);
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    if paused {
        report(IndexStatus::Paused);
        let reason = session.pause_reason().unwrap_or("import running");
        activity.pause(Job::Embedding, reason);
        activity.log(Job::Embedding, format!("paused ({reason}) after embedding {} notes", count(stale as u64)));
        // The notes embedded so far still go into the related-notes index.
        let related_changed = related::update(session, generation, &changed)?;
        return Ok(Pass { stale, total, descriptions: 0, related_changed, paused });
    }
    if preempted {
        activity.log(
            Job::Embedding,
            format!("completed {} vectors; a saved note goes first", count(stale as u64)),
        );
        let related_changed = related::update(session, generation, &changed)?;
        return Ok(Pass { stale, total, descriptions: 0, related_changed, paused });
    }

    let left = session.with_vault_quiet(generation, |v| v.embedding_queue_counts())?;
    let empty = left.total() == 0;
    if show {
        let complete = if empty { total } else { complete.min(total) };
        let done = if left.first == 0 { total } else { done.min(total) };
        report(IndexStatus::Indexing { done, complete, total, eta_secs: empty.then_some(0) });
    }
    let mut ready_now = ready.as_deref() == Some(model.as_str());
    if left.first == 0 && !ready_now {
        session.with_vault_quiet(generation, |v| v.finish_embedding_model(&model))?;
        ready_now = true;
        source.vault_models(&model, Some(&model));
        activity.log(Job::Embedding, format!("every note has a {model} vector: it is now the ready model"));
    }
    // Mid-switch the descriptions keep the ready model's vectors until the switch is done.
    let descriptions = if ready_now { embed_descriptions(session, generation, &model, embedder.as_ref())? } else { 0 };
    if descriptions > 0 {
        activity.log(Job::Embedding, format!("embedded {} tag descriptions", count(descriptions as u64)));
    }
    // The rate of the first phase is what a switch waits for (until the model is ready).
    if phase == QueuePhase::First && stale >= RATE_MIN {
        source.measured_rate(&model, stale as f64 / embedding_started.elapsed().as_secs_f64().max(1e-3));
    }
    if stale > 0 || work > 0 {
        let secs = started.elapsed().as_secs_f64();
        let rate = if stale > 0 && secs > 0.0 { format!(" ({:.1} notes/s)", stale as f64 / secs) } else { String::new() };
        let summary = match phase {
            QueuePhase::First if partial > 0 => format!(
                "embedded {} notes{rate}; {} of them from their first chunk, completed next",
                count(stale as u64),
                count(partial as u64)
            ),
            QueuePhase::First => format!("embedded {} notes{rate}", count(stale as u64)),
            QueuePhase::Rest => format!("completed the vectors of {} notes{rate}", count(stale as u64)),
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
