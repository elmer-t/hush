//! The Activity panel's data: what the jobs report while they run, and a note's
//! internals.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hush_lib::activity::{Job, State};
use hush_lib::classify::job as training;
use hush_lib::db::{TagSource, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A fast, deterministic embedder: words hashed into `dims` buckets.
struct Fake;

const MODEL: &str = "fake-8";

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        MODEL
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0; 8];
        for word in text.split_whitespace() {
            let h = word.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
            v[h as usize % 8] += 1.0;
        }
        v[0] += 0.01;
        embed::normalize(&mut v);
        Ok(v)
    }
}

struct Source;

impl EmbedderSource for Source {
    fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(Arc::new(Fake))
    }
}

fn session() -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(MODEL).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

fn wait_for(mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_jobs_report_their_runs_and_lock_clears_them() {
    let (_dir, _path, session, g) = session();
    session
        .with_vault(|v| {
            for i in 0..40 {
                v.create_note("", &format!("N{i}"), "some words here")?;
            }
            Ok(())
        })
        .unwrap();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &Source, Duration::from_millis(10), |_| {}));
        let activity = session.activity();
        wait_for(|| activity.status(Job::SafetyNet).last_run.is_some());

        let embedding = activity.status(Job::Embedding);
        assert_eq!(embedding.state, State::Idle);
        assert_eq!(embedding.reason.as_deref(), Some("nothing queued"));
        assert!(embedding.last_run.unwrap().summary.starts_with("embedded 40 notes"));
        let related = activity.status(Job::Related);
        assert!(related.last_run.unwrap().summary.starts_with("rebuilt index: 40 vectors"));
        let net = activity.status(Job::SafetyNet).last_run.unwrap();
        assert_eq!(net.summary, "checked 40 notes, queued 0 that no write had queued");

        let log = activity.snapshot().log;
        assert!(log.iter().any(|e| e.job == Job::Embedding && e.message.contains("is now the ready model")));
        // Newest first.
        assert!(log.windows(2).all(|w| w[0].seq > w[1].seq));

        // While an import runs the job says why it waits.
        let import = session.begin_import();
        wait_for(|| activity.status(Job::Embedding).state == State::Paused);
        assert_eq!(activity.status(Job::Embedding).reason.as_deref(), Some("import running"));
        drop(import);

        session.lock();
        let snapshot = activity.snapshot();
        assert!(snapshot.log.is_empty(), "the log is cleared on lock");
        assert!(snapshot.jobs.iter().all(|j| j.last_run.is_none()));
    });
}

#[test]
fn training_reports_each_head() {
    let (_dir, _path, session, g) = session();
    session
        .with_vault(|v| {
            for i in 0..12 {
                v.create_note("", &format!("Dream {i}"), &format!("#dream flying falling night {i}"))?;
            }
            for i in 0..30 {
                v.create_note("", &format!("Other {i}"), &format!("groceries list milk bread {i}"))?;
            }
            Ok(())
        })
        .unwrap();
    while job::pass(&session, g, &Source, &mut |_| {}).unwrap().stale > 0 {}
    let pass = training::pass(&session, g).unwrap();
    assert_eq!(pass.trained, ["dream"]);
    let status = session.activity().status(Job::Training);
    assert_eq!(status.state, State::Idle);
    assert_eq!(status.last_run.unwrap().summary, "1 heads trained, 0 dropped, 0 up to date");
    let log = session.activity().snapshot().log;
    assert!(log.iter().any(|e| e.job == Job::Training && e.message.starts_with("head dream trained (")));
}

#[test]
fn a_notes_internals() {
    let (_dir, _path, session, g) = session();
    let (a, b) = session
        .with_vault(|v| {
            let a = v.create_note("work", "Alpha", "---\ntopic: x\n---\nOne two three #manual [[Beta]]")?;
            let b = v.create_note("", "Beta", "Links back to [[Alpha]] and [[Gamma]]")?;
            Ok((a, b))
        })
        .unwrap();
    let before = session.with_vault(|v| v.note_internals(a)).unwrap();
    assert_eq!(before.folder, "work");
    assert_eq!((before.words, before.chars), (5, 30));
    assert!(before.queued);
    assert!(before.vectors.is_empty());
    assert_eq!(before.content_hash.as_ref().map(String::len), Some(64));
    assert_eq!((before.backlinks, before.outgoing), (1, 1));
    assert_eq!(before.model, MODEL);

    while job::pass(&session, g, &Source, &mut |_| {}).unwrap().stale > 0 {}
    session.with_vault(|v| v.accept_tag_suggestion(a, "suggested")).unwrap();
    let after = session.with_vault(|v| v.note_internals(a)).unwrap();
    assert_eq!(after.vectors.len(), 1);
    assert_eq!(after.vectors[0].model, MODEL);
    let tags: Vec<(&str, TagSource)> = after.tags.iter().map(|t| (t.tag.as_str(), t.source)).collect();
    assert_eq!(tags, [("manual", TagSource::Manual), ("suggested", TagSource::Suggestion)]);

    let beta = session.with_vault(|v| v.note_internals(b)).unwrap();
    assert!(!beta.queued);
    assert!(beta.vectors[0].current);
    assert_eq!(beta.vectors[0].hash, beta.content_hash.unwrap());
    assert_eq!((beta.backlinks, beta.outgoing), (1, 2));
}
