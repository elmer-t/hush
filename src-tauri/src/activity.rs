//! What the background jobs are doing, for the Activity panel. Every job reports into
//! one [`Activity`] (held by the [`crate::session::Session`]) instead of emitting its
//! own events; [`Activity::run_emitter`] sends the whole [`Snapshot`] to the frontend,
//! at most once per interval however busy the jobs are, and always the final state.
//!
//! Nothing here is ever written anywhere: no file, no config, no stderr. Messages hold
//! counts, durations, tag names and error kinds, never note text, and everything is
//! cleared (and zeroed) on lock.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use zeroize::Zeroize;

use crate::db::now_ms;

/// Event log entries kept; older ones are dropped.
pub const LOG_CAPACITY: usize = 500;
/// The rate is measured over this much of the most recent progress.
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Below this much progress history there is no rate (or ETA) yet.
const RATE_MIN_SPAN: Duration = Duration::from_secs(5);

/// The background jobs, in the order the panel lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Job {
    /// Embedding queued notes (`embed::job`).
    Embedding,
    /// The in-memory related-notes index (`embed::related`).
    Related,
    /// Tag classifier heads (`classify::job`).
    Training,
    /// The once-per-unlock check for notes no write queued (`embed::job::safety_net`).
    SafetyNet,
    Import,
    /// Discover tags (Settings → Tags).
    Discover,
    /// Schema upgrades run while the vault opens.
    Migration,
    /// Re-deriving tags after the tag rules changed (`retag`).
    Retag,
}

impl Job {
    pub const ALL: [Job; 8] = [
        Job::Embedding,
        Job::Related,
        Job::Training,
        Job::SafetyNet,
        Job::Import,
        Job::Discover,
        Job::Migration,
        Job::Retag,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|j| *j == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Nothing to do.
    Idle,
    /// Has work (or may soon) but holds off; [`JobStatus::reason`] says why.
    Waiting,
    Running,
    /// Stopped part-way and carries on later; [`JobStatus::reason`] says why.
    Paused,
    /// Cancelled, or won't run again this unlock.
    Stopped,
    Failed,
}

/// The last finished run of a job.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    /// Unix milliseconds.
    pub finished_at: i64,
    pub duration_ms: u64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub job: Job,
    pub state: State,
    /// What it is doing right now, e.g. `embedding batch 3 of queue page 2`.
    pub doing: Option<String>,
    /// Why it is waiting, paused or failed.
    pub reason: Option<String>,
    pub done: Option<u64>,
    /// `None` when the end isn't known.
    pub total: Option<u64>,
    /// Items per second over the last minute.
    pub rate: Option<f64>,
    pub eta_secs: Option<u64>,
    /// Unix milliseconds the current run started.
    pub started_at: Option<i64>,
    pub last_run: Option<LastRun>,
}

impl JobStatus {
    fn new(job: Job) -> Self {
        Self {
            job,
            state: State::Idle,
            doing: None,
            reason: None,
            done: None,
            total: None,
            rate: None,
            eta_secs: None,
            started_at: None,
            last_run: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Grows by one per entry, so the frontend can key them.
    pub seq: u64,
    /// Unix milliseconds.
    pub at: i64,
    pub job: Job,
    pub error: bool,
    pub message: String,
}

/// A trained tag head.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadInfo {
    pub tag: String,
    pub model: String,
    /// Unix milliseconds.
    pub trained_at: i64,
}

/// Numbers the jobs already have or that are counted on a timer while the panel is
/// open (see `commands::activity`); `None` until first counted.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Unix milliseconds of the last count.
    pub counted_at: Option<i64>,
    /// Live notes.
    pub notes: Option<u64>,
    /// Live notes with a vector for the active model.
    pub with_vector: Option<u64>,
    /// Rows in the embedding queue (`embedding_dirty`).
    pub queued: Option<u64>,
    /// The active model, and the model whose vectors are complete.
    pub model: Option<String>,
    pub ready_model: Option<String>,
    /// Whether the active model is loaded in memory.
    pub model_loaded: bool,
    pub cores_used: Option<u64>,
    pub cores_total: Option<u64>,
    /// Notes in the related-notes index and the memory it holds.
    pub related_notes: Option<u64>,
    pub related_bytes: Option<u64>,
    pub heads: Vec<HeadInfo>,
    pub schema_version: Option<u64>,
}

/// Everything the panel shows, sent as one event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Grows with every change.
    pub revision: u64,
    pub jobs: Vec<JobStatus>,
    pub stats: Stats,
    /// Newest first.
    pub log: Vec<LogEntry>,
}

/// Progress samples of the running job, for its rate.
#[derive(Default)]
struct Rate {
    samples: VecDeque<(Instant, u64)>,
}

impl Rate {
    fn update(&mut self, now: Instant, done: u64) -> Option<f64> {
        if self.samples.back().is_some_and(|s| done < s.1) {
            self.samples.clear();
        }
        self.samples.push_back((now, done));
        while self.samples.len() > 2 && self.samples.get(1).is_some_and(|s| now.duration_since(s.0) >= RATE_WINDOW) {
            self.samples.pop_front();
        }
        let &(start, from) = self.samples.front()?;
        let span = now.duration_since(start);
        (span >= RATE_MIN_SPAN).then(|| (done - from) as f64 / span.as_secs_f64())
    }
}

struct Inner {
    /// The unlock reports are taken from; 0 while locked.
    generation: u64,
    revision: u64,
    jobs: Vec<JobStatus>,
    /// When each job's current run started, for its duration.
    started: Vec<Option<Instant>>,
    rates: Vec<Rate>,
    stats: Stats,
    log: VecDeque<LogEntry>,
    seq: u64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            generation: 0,
            revision: 0,
            jobs: Job::ALL.iter().map(|j| JobStatus::new(*j)).collect(),
            started: vec![None; Job::ALL.len()],
            rates: Job::ALL.iter().map(|_| Rate::default()).collect(),
            stats: Stats::default(),
            log: VecDeque::with_capacity(LOG_CAPACITY),
            seq: 0,
        }
    }
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            jobs: self.jobs.clone(),
            stats: self.stats.clone(),
            log: self.log.iter().rev().cloned().collect(),
        }
    }

    fn push_log(&mut self, job: Job, error: bool, message: String) {
        if self.log.len() == LOG_CAPACITY
            && let Some(mut old) = self.log.pop_front()
        {
            old.message.zeroize();
        }
        self.seq += 1;
        self.log.push_back(LogEntry { seq: self.seq, at: now_ms(), job, error, message });
    }

    fn wipe(&mut self) {
        for entry in self.log.iter_mut() {
            entry.message.zeroize();
        }
        for head in self.stats.heads.iter_mut() {
            head.tag.zeroize();
        }
        for job in self.jobs.iter_mut() {
            job.doing.zeroize();
            job.reason.zeroize();
            if let Some(last) = job.last_run.as_mut() {
                last.summary.zeroize();
            }
        }
        // Revisions keep growing across locks, so the emitter sends the cleared state.
        let revision = self.revision + 1;
        *self = Self { generation: self.generation, revision, seq: self.seq, ..Self::default() };
    }
}

/// See the module docs.
#[derive(Default)]
pub struct Activity {
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl Activity {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reports for unlock `generation`; they are dropped once another unlock (or a lock)
    /// has cleared the activity, so a job outliving its vault can't report into the
    /// next one, or into the locked app.
    pub fn reporter(&self, generation: u64) -> Reporter<'_> {
        Reporter { activity: self, generation }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.lock().snapshot()
    }

    /// The current status of `job`.
    pub fn status(&self, job: Job) -> JobStatus {
        self.lock().jobs[job.index()].clone()
    }

    /// Forgets everything, zeroing the text, and takes reports for unlock `generation`
    /// from now on (0: none). Called on unlock and lock.
    pub fn clear(&self, generation: u64) {
        let mut inner = self.lock();
        inner.wipe();
        inner.generation = generation;
        drop(inner);
        self.changed.notify_all();
    }

    /// Sends a snapshot to `send` after every change, but never twice within
    /// `interval`: changes made meanwhile go out together in the next one, and the last
    /// change always goes out. Returns once `stop` says so (checked at least once per
    /// `interval`).
    pub fn run_emitter(&self, interval: Duration, stop: impl Fn() -> bool, mut send: impl FnMut(Snapshot)) {
        let mut sent = self.lock().revision;
        let mut last_send: Option<Instant> = None;
        loop {
            {
                let mut inner = self.lock();
                while inner.revision == sent {
                    if stop() {
                        return;
                    }
                    inner = self.changed.wait_timeout(inner, interval).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
            if let Some(at) = last_send {
                let wait = interval.saturating_sub(at.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
            }
            let snapshot = self.snapshot();
            sent = snapshot.revision;
            last_send = Some(Instant::now());
            send(snapshot);
        }
    }
}

/// Reports of one unlock's jobs; see [`Activity::reporter`]. Each call takes the
/// activity's own mutex briefly (never the vault's).
#[derive(Clone, Copy)]
pub struct Reporter<'a> {
    activity: &'a Activity,
    generation: u64,
}

impl Reporter<'_> {
    /// Runs `f` unless the activity belongs to another unlock; `peek` first says
    /// whether there is anything to change.
    fn change(&self, peek: impl FnOnce(&Inner) -> bool, f: impl FnOnce(&mut Inner)) {
        let mut inner = self.activity.lock();
        if inner.generation != self.generation || !peek(&inner) {
            return;
        }
        f(&mut inner);
        inner.revision += 1;
        drop(inner);
        self.activity.changed.notify_all();
    }

    /// Starts a run of `job`: running, with no progress yet.
    pub fn start(&self, job: Job, doing: impl Into<String>) {
        let doing = doing.into();
        self.change(|_| true, |inner| {
            let i = job.index();
            inner.started[i] = Some(Instant::now());
            inner.rates[i] = Rate::default();
            let s = &mut inner.jobs[i];
            s.state = State::Running;
            s.doing = Some(doing);
            s.reason = None;
            s.done = None;
            s.total = None;
            s.rate = None;
            s.eta_secs = None;
            s.started_at = Some(now_ms());
        });
    }

    /// Progress of the running `job` (started if it wasn't): `done` of `total` (or of an
    /// unknown number), with the rate and time left worked out from the last minute.
    pub fn progress(&self, job: Job, doing: impl Into<String>, done: u64, total: Option<u64>) {
        let doing = doing.into();
        self.change(|_| true, |inner| {
            let i = job.index();
            let now = Instant::now();
            if inner.started[i].is_none() {
                inner.started[i] = Some(now);
                inner.rates[i] = Rate::default();
                inner.jobs[i].started_at = Some(now_ms());
            }
            let rate = inner.rates[i].update(now, done);
            let s = &mut inner.jobs[i];
            s.state = State::Running;
            s.doing = Some(doing);
            s.reason = None;
            s.done = Some(done);
            s.total = total;
            s.rate = rate;
            s.eta_secs = match (rate, total) {
                (_, Some(t)) if done >= t => Some(0),
                (Some(r), Some(t)) if r > 0.0 => Some((t.saturating_sub(done) as f64 / r).ceil() as u64),
                _ => None,
            };
        });
    }

    /// Ends the run of `job`: idle, with `summary` as its last run and in the log.
    pub fn finish(&self, job: Job, summary: impl Into<String>) {
        self.end(job, State::Idle, None, summary.into(), Some(false));
    }

    /// [`Reporter::finish`] without a log entry, for small routine runs (the pass after
    /// an autosave) that would push everything else out of the log.
    pub fn finish_quietly(&self, job: Job, summary: impl Into<String>) {
        self.end(job, State::Idle, None, summary.into(), None);
    }

    /// Ends the run of `job` as stopped (cancelled).
    pub fn stop(&self, job: Job, summary: impl Into<String>) {
        self.end(job, State::Stopped, None, summary.into(), Some(false));
    }

    /// Ends the run of `job` as failed; `message` names no note content.
    pub fn fail(&self, job: Job, message: impl Into<String>) {
        let message = message.into();
        self.end(job, State::Failed, Some(message.clone()), format!("failed: {message}"), Some(true));
    }

    /// `log`: whether to log it, and as an error or not.
    fn end(&self, job: Job, state: State, reason: Option<String>, summary: String, log: Option<bool>) {
        self.change(|_| true, |inner| {
            let i = job.index();
            let duration = inner.started[i].take().map(|t| t.elapsed()).unwrap_or_default();
            let s = &mut inner.jobs[i];
            s.state = state;
            s.reason = reason;
            s.doing = None;
            s.rate = None;
            s.eta_secs = None;
            s.started_at = None;
            s.last_run = Some(LastRun {
                finished_at: now_ms(),
                duration_ms: duration.as_millis() as u64,
                summary: summary.clone(),
            });
            if let Some(error) = log {
                inner.push_log(job, error, format!("{summary} ({})", duration_text(duration)));
            }
        });
    }

    /// Records a run that ended before reports could be taken (a migration runs while
    /// the vault opens): its last run, and a log entry.
    pub fn record(&self, job: Job, duration: Duration, summary: impl Into<String>) {
        let summary = summary.into();
        self.change(|_| true, |inner| {
            inner.jobs[job.index()].last_run = Some(LastRun {
                finished_at: now_ms(),
                duration_ms: duration.as_millis() as u64,
                summary: summary.clone(),
            });
            inner.push_log(job, false, format!("{summary} ({})", duration_text(duration)));
        });
    }

    /// `job` holds off for `reason`. A run in progress keeps its start, so its duration
    /// counts the wait.
    pub fn wait(&self, job: Job, reason: impl Into<String>) {
        self.hold(job, State::Waiting, reason.into());
    }

    /// `job` stopped part-way for `reason` and carries on later.
    pub fn pause(&self, job: Job, reason: impl Into<String>) {
        self.hold(job, State::Paused, reason.into());
    }

    // Unchanged holds are skipped, so a waiting job polling once a second doesn't wake
    // the emitter.
    fn hold(&self, job: Job, state: State, reason: String) {
        let probe = reason.clone();
        self.change(
            |inner| {
                let s = &inner.jobs[job.index()];
                s.state != state || s.reason.as_deref() != Some(probe.as_str())
            },
            |inner| {
                let s = &mut inner.jobs[job.index()];
                s.state = state;
                s.reason = Some(reason);
                s.rate = None;
                s.eta_secs = None;
            },
        );
    }

    /// `job` has nothing to do, for `reason` if given (`nothing queued`). Its last run
    /// is kept.
    pub fn idle(&self, job: Job, reason: Option<&str>) {
        self.change(
            |inner| {
                let s = &inner.jobs[job.index()];
                s.state != State::Idle || s.reason.as_deref() != reason || s.doing.is_some()
            },
            |inner| {
                let i = job.index();
                inner.started[i] = None;
                let s = &mut inner.jobs[i];
                s.state = State::Idle;
                s.reason = reason.map(str::to_string);
                s.doing = None;
                s.done = None;
                s.total = None;
                s.rate = None;
                s.eta_secs = None;
                s.started_at = None;
            },
        );
    }

    /// Adds an entry to the event log.
    pub fn log(&self, job: Job, message: impl Into<String>) {
        let message = message.into();
        self.change(|_| true, |inner| inner.push_log(job, false, message));
    }

    /// Adds an error to the event log.
    pub fn log_error(&self, job: Job, message: impl Into<String>) {
        let message = message.into();
        self.change(|_| true, |inner| inner.push_log(job, true, message));
    }

    /// Changes the counted numbers.
    pub fn stats(&self, f: impl FnOnce(&mut Stats)) {
        self.change(|_| true, |inner| f(&mut inner.stats));
    }
}

/// `1,234,567`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `850 ms`, `12.3 s`, `4 min 05 s`, `2 h 03 min`.
pub fn duration_text(d: Duration) -> String {
    let ms = d.as_millis();
    let secs = d.as_secs();
    if ms < 1_000 {
        format!("{ms} ms")
    } else if secs < 60 {
        format!("{:.1} s", d.as_secs_f64())
    } else if secs < 3_600 {
        format!("{} min {:02} s", secs / 60, secs % 60)
    } else {
        format!("{} h {:02} min", secs / 3_600, (secs % 3_600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn the_log_keeps_the_newest_entries_only() {
        let a = Activity::default();
        for i in 0..LOG_CAPACITY + 120 {
            a.reporter(0).log(Job::Embedding, format!("entry {i}"));
        }
        let log = a.snapshot().log;
        assert_eq!(log.len(), LOG_CAPACITY);
        assert_eq!(log[0].message, format!("entry {}", LOG_CAPACITY + 119), "newest first");
        assert_eq!(log[LOG_CAPACITY - 1].message, "entry 120");
        assert!(log.windows(2).all(|w| w[0].seq == w[1].seq + 1));
    }

    #[test]
    fn reports_of_another_unlock_are_dropped() {
        let a = Activity::default();
        a.clear(2);
        a.reporter(1).log(Job::Embedding, "from the vault before");
        assert!(a.snapshot().log.is_empty());
        a.reporter(2).log(Job::Embedding, "from this one");
        assert_eq!(a.snapshot().log.len(), 1);
        a.clear(0);
        a.reporter(2).log(Job::Embedding, "after the lock");
        assert!(a.snapshot().log.is_empty());
    }

    #[test]
    fn clear_forgets_everything() {
        let a = Activity::default();
        a.reporter(0).progress(Job::Training, "training head journal", 1, Some(3));
        a.reporter(0).log(Job::Training, "head journal trained");
        a.reporter(0).stats(|s| s.notes = Some(10));
        let before = a.snapshot().revision;
        a.clear(0);
        let s = a.snapshot();
        assert!(s.revision > before, "a clear is a change, so it is sent");
        assert!(s.log.is_empty());
        assert_eq!(s.stats, Stats::default());
        assert!(s.jobs.iter().all(|j| j.state == State::Idle && j.doing.is_none() && j.last_run.is_none()));
    }

    #[test]
    fn a_run_records_its_last_run_and_logs_it() {
        let a = Activity::default();
        a.reporter(0).start(Job::Discover, "grouping notes");
        assert_eq!(a.status(Job::Discover).state, State::Running);
        a.reporter(0).finish(Job::Discover, "12 groups from 400 notes");
        let s = a.status(Job::Discover);
        assert_eq!(s.state, State::Idle);
        assert_eq!(s.doing, None);
        assert_eq!(s.last_run.unwrap().summary, "12 groups from 400 notes");
        assert!(a.snapshot().log[0].message.starts_with("12 groups from 400 notes ("));

        a.reporter(0).fail(Job::Import, "could not read the folder");
        let s = a.status(Job::Import);
        assert_eq!(s.state, State::Failed);
        assert_eq!(s.reason.as_deref(), Some("could not read the folder"));
        assert!(a.snapshot().log[0].error);
    }

    #[test]
    fn waiting_again_for_the_same_reason_is_not_a_change() {
        let a = Activity::default();
        a.reporter(0).wait(Job::Training, "import running");
        let r = a.snapshot().revision;
        a.reporter(0).wait(Job::Training, "import running");
        a.reporter(0).idle(Job::Embedding, None);
        assert_eq!(a.snapshot().revision, r);
        a.reporter(0).wait(Job::Training, "vault changed, waiting for 30 s of quiet");
        assert_eq!(a.snapshot().revision, r + 1);
    }

    #[test]
    fn the_rate_and_time_left_come_from_recent_progress() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut rate = Rate::default();
        assert_eq!(rate.update(at(0), 0), None);
        assert_eq!(rate.update(at(2), 20), None, "under five seconds of progress");
        assert_eq!(rate.update(at(10), 100), Some(10.0));
        // Progress started again (a new pass): the old samples don't count.
        assert_eq!(rate.update(at(11), 5), None);
    }

    /// Many changes in a burst reach the frontend as a few snapshots, never more than
    /// one per interval, and the last one always has the final state.
    #[test]
    fn snapshots_are_throttled_and_the_final_state_is_always_sent() {
        let a = Arc::new(Activity::default());
        let interval = Duration::from_millis(50);
        let stop = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(Mutex::new(Vec::<(Instant, Snapshot)>::new()));
        let emitter = {
            let (a, stop, sent) = (a.clone(), stop.clone(), sent.clone());
            std::thread::spawn(move || {
                a.run_emitter(interval, || stop.load(Ordering::SeqCst), |s| {
                    sent.lock().unwrap().push((Instant::now(), s));
                })
            })
        };

        let started = Instant::now();
        let total = 20_000;
        for done in 1..=total {
            a.reporter(0).progress(Job::Embedding, "embedding", done, Some(total));
        }
        a.reporter(0).finish(Job::Embedding, "embedded 20,000 notes");
        let elapsed = started.elapsed();
        // Give the emitter one interval (and some) to send the final state.
        std::thread::sleep(interval * 4);
        stop.store(true, Ordering::SeqCst);
        emitter.join().unwrap();

        let sent = sent.lock().unwrap();
        let most = (elapsed.as_millis() / interval.as_millis()) as usize + 3;
        assert!(!sent.is_empty());
        assert!(sent.len() <= most, "{} snapshots for {} changes in {elapsed:?}", sent.len(), total + 1);
        for pair in sent.windows(2) {
            // A little slack for the clock reads around `send`.
            assert!(pair[1].0.duration_since(pair[0].0) + Duration::from_millis(2) >= interval);
            assert!(pair[1].1.revision > pair[0].1.revision);
        }
        let last = &sent.last().unwrap().1;
        assert_eq!(last.revision, a.snapshot().revision);
        let job = &last.jobs[Job::Embedding.index()];
        assert_eq!(job.state, State::Idle);
        assert_eq!(job.last_run.as_ref().unwrap().summary, "embedded 20,000 notes");
    }

    #[test]
    fn a_change_after_a_quiet_spell_is_sent_at_once() {
        let a = Arc::new(Activity::default());
        let stop = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(Mutex::new(Vec::<Instant>::new()));
        let emitter = {
            let (a, stop, sent) = (a.clone(), stop.clone(), sent.clone());
            std::thread::spawn(move || {
                a.run_emitter(Duration::from_millis(250), || stop.load(Ordering::SeqCst), |_| {
                    sent.lock().unwrap().push(Instant::now());
                })
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        let at = Instant::now();
        a.reporter(0).log(Job::Import, "import started");
        std::thread::sleep(Duration::from_millis(100));
        stop.store(true, Ordering::SeqCst);
        emitter.join().unwrap();
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].duration_since(at) < Duration::from_millis(50));
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1,000");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(Duration::from_millis(850)), "850 ms");
        assert_eq!(duration_text(Duration::from_millis(12_340)), "12.3 s");
        assert_eq!(duration_text(Duration::from_secs(245)), "4 min 05 s");
        assert_eq!(duration_text(Duration::from_secs(7_380)), "2 h 03 min");
    }
}
