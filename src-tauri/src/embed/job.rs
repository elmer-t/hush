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
use crate::activity::{Job, Reporter, count, duration_text};
use crate::db::{self, QueueCounts, QueuePhase};
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
    let setup = start_pass(session, generation, source)?;
    let mut state = PassState::new(&setup)?;
    let end = loop {
        if let Some(end) = state.page(&setup, report)? {
            break end;
        }
    };
    end_pass(&setup, source, &state, end, report)
}

/// What a pass settles at its start and only reads after that.
struct PassSetup<'a> {
    session: &'a Session,
    generation: u64,
    activity: Reporter<'a>,
    model: String,
    ready: Option<String>,
    counts: QueueCounts,
    phase: QueuePhase,
    /// Notes queued for `phase` when the pass started.
    work: usize,
    embedder: Arc<dyn Embedder>,
    started: Instant,
    /// After the model loaded.
    embedding_started: Instant,
    /// Whether the pass shows a progress line, and of how many notes.
    show: bool,
    total: usize,
}

/// How a pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassEnd {
    /// It went through the whole queue for its phase.
    Finished,
    /// An import started or the vault is being compacted (see [`Pass::paused`]).
    Paused,
    /// In the second phase, a save queued a note for its first chunk.
    Preempted,
}

/// Reads the queue counts, picks the phase and loads the model.
fn start_pass<'a>(session: &'a Session, generation: u64, source: &dyn EmbedderSource) -> Result<PassSetup<'a>, JobError> {
    let activity = session.activity().reporter(generation);
    let started = Instant::now();
    let (model, ready, counts) = session.with_vault_quiet(generation, |v| {
        Ok((v.embedding_model()?, v.embedding_model_ready()?, v.embedding_queue_counts()?))
    })?;
    let phase = if counts.first > 0 || counts.rest == 0 { QueuePhase::First } else { QueuePhase::Rest };
    source.vault_models(&model, ready.as_deref());
    let embedder = load_embedder(&activity, source, &model, started)?;
    let work = match phase {
        QueuePhase::First => counts.first,
        QueuePhase::Rest => counts.rest,
    };
    if work > 0 {
        activity.start(Job::Embedding, start_text(phase, work, &model));
    }
    let embedding_started = Instant::now();
    let show = counts.total() >= PROGRESS_MIN;
    // Counting every note costs a scan of `notes`: only done for the progress line.
    let total = if show { session.with_vault_quiet(generation, |v| v.count_in_folder(""))? } else { 0 };
    Ok(PassSetup {
        session,
        generation,
        activity,
        model,
        ready,
        counts,
        phase,
        work,
        embedder,
        started,
        embedding_started,
        show,
        total,
    })
}

fn load_embedder(
    activity: &Reporter<'_>,
    source: &dyn EmbedderSource,
    model: &str,
    started: Instant,
) -> Result<Arc<dyn Embedder>, JobError> {
    let loading = !source.is_loaded(model);
    if loading {
        activity.start(Job::Embedding, format!("loading model {model}"));
    }
    let embedder = source.embedder(model)?;
    if loading {
        activity.log(Job::Embedding, format!("model {model} loaded ({})", duration_text(started.elapsed())));
    }
    Ok(embedder)
}

fn start_text(phase: QueuePhase, work: usize, model: &str) -> String {
    match phase {
        QueuePhase::First => format!("embedding the first chunk of {} queued notes with {model}", count(work as u64)),
        QueuePhase::Rest => format!("completing the vectors of {} notes with {model}", count(work as u64)),
    }
}

/// Where a pass is in the queue, and what it did so far.
struct PassState {
    /// Notes with a vector of their text, from the first chunk or all of it; and those
    /// with the full vector.
    done: usize,
    complete: usize,
    stale: usize,
    partial: usize,
    /// Notes whose vector changed or went away, for the related-notes index.
    changed: Vec<i64>,
    eta: Eta,
    after: db::QueueCursor,
    /// The vault's change counter after the job's own last write: in the second phase,
    /// only a change by someone else is worth looking for a note queued for its first
    /// chunk. A change found just before the job's own write is remembered by leaving
    /// `seen_changes` behind, so the next check still looks.
    seen_changes: u64,
    /// The queue as this pass works through it, for the Activity panel.
    handled: usize,
    page_number: usize,
}

impl PassState {
    fn new(s: &PassSetup) -> Result<Self, JobError> {
        let changed = s.session.with_vault_quiet(s.generation, |v| v.take_deleted_notes())?;
        Ok(Self {
            done: s.total.saturating_sub(s.counts.first),
            complete: s.total.saturating_sub(s.counts.total()),
            stale: 0,
            partial: 0,
            changed,
            eta: Eta::default(),
            after: db::QueueCursor::default(),
            seen_changes: 0,
            handled: 0,
            page_number: 0,
        })
    }

    /// Embeds one page of the queue. `None`: go on with the next page.
    fn page(&mut self, s: &PassSetup, report: &mut dyn FnMut(IndexStatus)) -> Result<Option<PassEnd>, JobError> {
        let page = self.read_page(s)?;
        let batches = page.stale.len().div_ceil(BATCH);
        // The first page note not embedded yet. The first phase takes `BATCH` notes at a
        // time; the second as many as the model takes per call (`Embedder::embed_group`).
        let mut start = 0;
        while start < page.stale.len() {
            self.report_progress(s, page.stale.len(), start, batches, report);
            if let Some(end) = self.interruption(s)? {
                return Ok(Some(end));
            }
            match self.embed_batch(s, &page.stale, start)? {
                Some(embedded) => start += embedded,
                None => return Ok(Some(PassEnd::Paused)),
            }
        }
        drop(page.stale);
        match page.next {
            Some(next) => {
                self.after = next;
                Ok(None)
            }
            None => Ok(Some(PassEnd::Finished)),
        }
    }

    fn read_page(&mut self, s: &PassSetup) -> Result<db::QueuePage, JobError> {
        let (after, seen_changes) = (&self.after, &mut self.seen_changes);
        let page = s.session.with_vault_quiet(s.generation, |v| {
            let foreign = v.change_count() != *seen_changes;
            let page = v.queued_embeddings(&s.model, after, SCAN_PAGE, s.phase)?;
            *seen_changes = if foreign { u64::MAX } else { v.change_count() };
            Ok(page)
        })?;
        self.page_number += 1;
        match s.phase {
            QueuePhase::First => {
                self.done += page.settled + page.partial;
                self.complete += page.settled;
            }
            QueuePhase::Rest => self.complete += page.settled,
        }
        self.handled += page.settled + page.partial + page.dropped.len();
        self.changed.extend(&page.dropped);
        Ok(page)
    }

    fn report_progress(
        &mut self,
        s: &PassSetup,
        page_len: usize,
        start: usize,
        batches: usize,
        report: &mut dyn FnMut(IndexStatus),
    ) {
        if s.show {
            let (done, complete, total) = (self.done.min(s.total), self.complete.min(s.total), s.total);
            let progress = if s.phase == QueuePhase::First { done } else { complete };
            report(IndexStatus::Indexing { done, complete, total, eta_secs: self.eta.update(Instant::now(), progress, total) });
        }
        let page_number = self.page_number;
        let doing = match s.phase {
            QueuePhase::First => format!("first chunks: batch {} of {batches} on queue page {page_number}", start / BATCH + 1),
            QueuePhase::Rest => format!("remaining chunks: note {} of {page_len} on queue page {page_number}", start + 1),
        };
        s.activity.progress(Job::Embedding, doing, self.handled as u64, Some(s.work.max(self.handled) as u64));
    }

    /// Whether the pass stops before the next batch; an error once the vault locked.
    fn interruption(&mut self, s: &PassSetup) -> Result<Option<PassEnd>, JobError> {
        if !s.session.is_open(s.generation) {
            return Err(JobError::Locked);
        }
        if s.session.pause_reason().is_some() {
            return Ok(Some(PassEnd::Paused));
        }
        if s.phase == QueuePhase::Rest {
            // Once per call, so once per group of notes when the model groups them.
            let seen_changes = &mut self.seen_changes;
            let first_waiting = s.session.with_vault_quiet(s.generation, |v| {
                let changes = v.change_count();
                if changes == *seen_changes {
                    return Ok(false);
                }
                *seen_changes = changes;
                v.embedding_first_pending()
            })?;
            if first_waiting {
                return Ok(Some(PassEnd::Preempted));
            }
        }
        Ok(None)
    }

    /// Embeds and writes the next batch from `stale[start..]`: how many notes it took,
    /// or `None` when the model stopped for a pause.
    fn embed_batch(&mut self, s: &PassSetup, stale: &[db::QueuedText], start: usize) -> Result<Option<usize>, JobError> {
        let candidates = match s.phase {
            QueuePhase::First => &stale[start..stale.len().min(start + BATCH)],
            QueuePhase::Rest => &stale[start..],
        };
        let Some(embedded) = embed_candidates(s, candidates)? else {
            return Ok(None);
        };
        let batch = &candidates[..embedded.len()];
        self.write_batch(s, batch, embedded)?;
        Ok(Some(batch.len()))
    }

    fn write_batch(&mut self, s: &PassSetup, batch: &[db::QueuedText], embedded: Vec<FirstChunk>) -> Result<(), JobError> {
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
        let seen_changes = self.seen_changes;
        self.seen_changes = s.session.with_vault_quiet(s.generation, |v| {
            let foreign = v.change_count() != seen_changes;
            v.write_queued_embeddings(&s.model, &vectors)?;
            Ok(if foreign { u64::MAX } else { v.change_count() })
        })?;
        self.changed.extend(vectors.iter().map(|v| v.id));
        let partial_now = vectors.iter().filter(|v| v.partial).count();
        self.partial += partial_now;
        if s.phase == QueuePhase::First {
            self.done += batch.len();
        }
        self.complete += batch.len() - partial_now;
        self.stale += batch.len();
        self.handled += batch.len();
        Ok(())
    }
}

/// Runs the model on `candidates` in one call (it may run several notes' chunks
/// together): a vector for at least the first of them, or `None` when it stopped for a
/// pause.
fn embed_candidates(s: &PassSetup, candidates: &[db::QueuedText]) -> Result<Option<Vec<FirstChunk>>, JobError> {
    let texts: Vec<&str> = candidates.iter().map(|q| q.note.text.as_str()).collect();
    let stop = || !s.session.is_open(s.generation) || s.session.pause_reason().is_some();
    let embedded = match s.phase {
        QueuePhase::First => s.embedder.embed_first_batch(&texts, &stop),
        QueuePhase::Rest => s
            .embedder
            .embed_group(&texts, &stop)
            .map(|vectors| vectors.into_iter().map(|vector| FirstChunk { vector, complete: true }).collect()),
    };
    let embedded = match embedded {
        Err(EmbedError::Stopped) if s.session.is_open(s.generation) => return Ok(None),
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
    Ok(Some(embedded))
}

/// Reports how the pass ended and brings the related-notes index up to date.
fn end_pass(
    s: &PassSetup,
    source: &dyn EmbedderSource,
    state: &PassState,
    end: PassEnd,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<Pass, JobError> {
    let descriptions = match end {
        PassEnd::Paused => {
            report(IndexStatus::Paused);
            let reason = s.session.pause_reason().unwrap_or("import running");
            s.activity.pause(Job::Embedding, reason);
            s.activity.log(Job::Embedding, format!("paused ({reason}) after embedding {} notes", count(state.stale as u64)));
            0
        }
        PassEnd::Preempted => {
            s.activity.log(
                Job::Embedding,
                format!("completed {} vectors; a saved note goes first", count(state.stale as u64)),
            );
            0
        }
        PassEnd::Finished => finish_queue(s, source, state, report)?,
    };
    // Only the notes this pass touched (also when it stopped early); a full rebuild when
    // the ready model changed or the index isn't built yet.
    let related_changed = related::update(s.session, s.generation, &state.changed)?;
    Ok(Pass { stale: state.stale, total: s.total, descriptions, related_changed, paused: end == PassEnd::Paused })
}

/// The end of a pass that went through the whole queue for its phase: readiness, the
/// tag descriptions, the measured rate and the summary. Returns how many descriptions
/// it embedded.
fn finish_queue(
    s: &PassSetup,
    source: &dyn EmbedderSource,
    state: &PassState,
    report: &mut dyn FnMut(IndexStatus),
) -> Result<usize, JobError> {
    let left = s.session.with_vault_quiet(s.generation, |v| v.embedding_queue_counts())?;
    report_final_progress(s, state, left, report);
    let ready_now = settle_ready_model(s, source, left)?;
    // Mid-switch the descriptions keep the ready model's vectors until the switch is done.
    let descriptions =
        if ready_now { embed_descriptions(s.session, s.generation, &s.model, s.embedder.as_ref())? } else { 0 };
    if descriptions > 0 {
        s.activity.log(Job::Embedding, format!("embedded {} tag descriptions", count(descriptions as u64)));
    }
    record_rate(s, source, state.stale);
    log_summary(s, state);
    if left.total() == 0 {
        report(IndexStatus::Idle);
        s.activity.idle(Job::Embedding, Some("nothing queued"));
    }
    Ok(descriptions)
}

fn report_final_progress(s: &PassSetup, state: &PassState, left: QueueCounts, report: &mut dyn FnMut(IndexStatus)) {
    if s.show {
        let (total, empty) = (s.total, left.total() == 0);
        let complete = if empty { total } else { state.complete.min(total) };
        let done = if left.first == 0 { total } else { state.done.min(total) };
        report(IndexStatus::Indexing { done, complete, total, eta_secs: empty.then_some(0) });
    }
}

/// Makes the active model the ready model once no note waits for a first vector.
/// Returns whether it is the ready model.
fn settle_ready_model(s: &PassSetup, source: &dyn EmbedderSource, left: QueueCounts) -> Result<bool, JobError> {
    let ready_now = s.ready.as_deref() == Some(s.model.as_str());
    if left.first == 0 && !ready_now {
        s.session.with_vault_quiet(s.generation, |v| v.finish_embedding_model(&s.model))?;
        source.vault_models(&s.model, Some(&s.model));
        s.activity.log(Job::Embedding, format!("every note has a {} vector: it is now the ready model", s.model));
        return Ok(true);
    }
    Ok(ready_now)
}

/// The rate of the first phase is what a switch waits for (until the model is ready).
fn record_rate(s: &PassSetup, source: &dyn EmbedderSource, stale: usize) {
    if s.phase == QueuePhase::First && stale >= RATE_MIN {
        source.measured_rate(&s.model, stale as f64 / s.embedding_started.elapsed().as_secs_f64().max(1e-3));
    }
}

fn log_summary(s: &PassSetup, state: &PassState) {
    let stale = state.stale;
    if stale == 0 && s.work == 0 {
        return;
    }
    let secs = s.started.elapsed().as_secs_f64();
    let rate = if stale > 0 && secs > 0.0 { format!(" ({:.1} notes/s)", stale as f64 / secs) } else { String::new() };
    let summary = summary_text(s.phase, stale, state.partial, &rate);
    if stale >= PROGRESS_MIN {
        s.activity.finish(Job::Embedding, summary);
    } else {
        s.activity.finish_quietly(Job::Embedding, summary);
    }
}

fn summary_text(phase: QueuePhase, stale: usize, partial: usize, rate: &str) -> String {
    match phase {
        QueuePhase::First if partial > 0 => format!(
            "embedded {} notes{rate}; {} of them from their first chunk, completed next",
            count(stale as u64),
            count(partial as u64)
        ),
        QueuePhase::First => format!("embedded {} notes{rate}", count(stale as u64)),
        QueuePhase::Rest => format!("completed the vectors of {} notes{rate}", count(stale as u64)),
    }
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
