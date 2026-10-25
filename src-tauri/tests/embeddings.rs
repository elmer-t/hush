mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use hush_lib::db::{MetadataEdit, NewNote, Vault};
use hush_lib::embed::job::{self, EmbedderSource, IndexStatus};
use hush_lib::embed::onnx::OnnxEmbedder;
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A fast, deterministic embedder: words hashed into `dims` buckets. Counts its calls.
struct Fake {
    id: String,
    dims: usize,
    delay: Duration,
    calls: AtomicUsize,
}

impl Fake {
    fn new(dims: usize) -> Arc<Self> {
        Self::slow(dims, Duration::ZERO)
    }

    fn slow(dims: usize, delay: Duration) -> Arc<Self> {
        Arc::new(Self { id: format!("fake-{dims}"), dims, delay, calls: AtomicUsize::new(0) })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let mut v = vec![0.0; self.dims];
        for word in text.split_whitespace() {
            let h = word.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
            v[h as usize % self.dims] += 1.0;
        }
        v[0] += 0.01;
        embed::normalize(&mut v);
        Ok(v)
    }
}

struct Fakes(Vec<Arc<Fake>>);

impl EmbedderSource for Fakes {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let fake = self.0.iter().find(|f| f.id == model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(fake.clone())
    }
}

/// Loads the bundled model from a folder, like the app does.
struct Bundled(PathBuf);

impl EmbedderSource for Bundled {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let d = embed::descriptor(model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(Arc::new(OnnxEmbedder::load(&self.0, d)?))
    }
}

/// An unlocked session on a fresh vault that embeds with `model`.
fn session(model: &str) -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(model).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

/// Runs passes until one finds nothing to do.
fn index(session: &Session, generation: u64, source: &dyn EmbedderSource) {
    for _ in 0..10 {
        if job::pass(session, generation, source, &mut |_| {}).unwrap().stale == 0 {
            return;
        }
    }
    panic!("indexing never finished");
}

fn vector(session: &Session, id: i64) -> Option<Vec<f32>> {
    session.with_vault(|v| v.note_embedding(id)).unwrap()
}

fn count(session: &Session, model: &str) -> usize {
    session.with_vault(|v| v.embedding_count(model)).unwrap()
}

/// Waits until `f` holds, for at most a few seconds.
fn wait_for(mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn notes_are_embedded_and_re_embedded_only_when_their_text_changes() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Walk", "---\ntags: [a]\n---\nA walk in the park")).unwrap();
    let empty = session.with_vault(|v| v.create_note("", "Empty", "---\ntags: [a]\n---\n")).unwrap();

    index(&session, g, &source);
    assert_eq!(fake.calls(), 1, "the empty note isn't embedded");
    let first = vector(&session, id).expect("new note gets a vector");
    assert_eq!(first.len(), 4);
    assert_eq!(vector(&session, empty), None);
    assert_eq!(session.with_vault(|v| v.embedding_model_ready()).unwrap().as_deref(), Some("fake-4"));

    // Saving the same text again, or changing only the frontmatter, doesn't re-embed.
    session.with_vault(|v| v.update_body(id, "---\ntags: [a]\n---\nA walk in the park")).unwrap();
    session.with_vault(|v| v.update_body(id, "---\ntags: [a, b]\nauthor: Ada\n---\nA walk in the park")).unwrap();
    index(&session, g, &source);
    assert_eq!(fake.calls(), 1);

    // A body change does.
    session.with_vault(|v| v.update_body(id, "---\ntags: [a]\n---\nRain all day")).unwrap();
    index(&session, g, &source);
    assert_eq!(fake.calls(), 2);
    assert_ne!(vector(&session, id).unwrap(), first);

    // So does a rename: the title is part of the text.
    session.with_vault(|v| v.rename_note(id, "", "Rain")).unwrap();
    index(&session, g, &source);
    assert_eq!(fake.calls(), 3);

    // A note emptied later loses its vector.
    session.with_vault(|v| v.update_body(id, "")).unwrap();
    index(&session, g, &source);
    assert_eq!(vector(&session, id), None);
    assert_eq!(count(&session, "fake-4"), 0);
}

#[test]
fn trash_drops_the_vector_and_restore_embeds_again() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "T", "Some text")).unwrap();
    index(&session, g, &source);
    assert_eq!(count(&session, "fake-4"), 1);

    session.with_vault(|v| v.trash_note(id)).unwrap();
    assert_eq!(count(&session, "fake-4"), 0, "trash removes the row");
    index(&session, g, &source);
    assert_eq!(count(&session, "fake-4"), 0, "trashed notes aren't embedded");

    session.with_vault(|v| v.restore_note(id).map(|_| ())).unwrap();
    index(&session, g, &source);
    assert_eq!(fake.calls(), 2);
    assert!(vector(&session, id).is_some());
}

#[test]
fn imported_and_bulk_edited_notes_are_picked_up_by_the_running_job() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let source = Fakes(vec![fake.clone()]);
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| {
            job::run(&session, g, &source, Duration::from_millis(10), |_| {});
            done_tx.send(()).unwrap();
        });
        let texts: Vec<(String, String)> =
            (0..50).map(|i| (format!("Note {i}"), format!("Imported text number {i}"))).collect();
        let notes: Vec<NewNote> = texts
            .iter()
            .map(|(title, body)| NewNote { folder: "imp", title, body, created_at: 1, updated_at: 1 })
            .collect();
        session.with_vault(|v| v.import_notes(&notes).map(|_| ())).unwrap();
        wait_for(|| count(&session, "fake-4") == 50);

        let ids: Vec<i64> = session.with_vault(|v| v.folder_notes("imp", 0, 10)).unwrap().iter().map(|n| n.id).collect();
        let calls = fake.calls();
        // A bulk tag edit only touches frontmatter: nothing to re-embed.
        let edit = MetadataEdit::AddTag("x".into());
        session.with_vault(|v| v.edit_notes_metadata(&ids, &edit).map(|_| ())).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fake.calls(), calls);
        // A save that changes the text is.
        session.with_vault(|v| v.update_body(ids[0], "Different words now")).unwrap();
        wait_for(|| fake.calls() == calls + 1);

        session.lock();
        done_rx.recv_timeout(Duration::from_secs(5)).expect("job stops on lock");
    });
}

#[test]
fn the_job_does_not_count_as_activity() {
    let (_dir, _path, session, g) = session("fake-4");
    let source = Fakes(vec![Fake::new(4)]);
    session.with_vault(|v| {
        for i in 0..20 {
            v.create_note("", &format!("N{i}"), "words here")?;
        }
        Ok(())
    })
    .unwrap();
    let start = Instant::now();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(5), |_| {}));
        // Polled through the quiet accessor too, so only the job could reset the idle time.
        wait_for(|| session.with_vault_quiet(g, |v| v.embedding_count("fake-4")).unwrap() == 20);
        std::thread::sleep(Duration::from_millis(100));
        let idle = session.idle_for().unwrap();
        assert!(idle >= start.elapsed(), "idle {idle:?} reset by the job");
        session.lock();
    });
}

/// The model runs without the vault lock, so the UI's commands never wait for it.
#[test]
fn the_vault_stays_available_while_the_model_runs() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::slow(4, Duration::from_millis(50));
    let source = Fakes(vec![fake.clone()]);
    let id = session
        .with_vault(|v| (0..40).map(|i| v.create_note("", &format!("N{i}"), "some words")).last().unwrap())
        .unwrap();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(10), |_| {}));
        wait_for(|| fake.calls() >= 1);
        let mut slowest = Duration::ZERO;
        while fake.calls() < 20 {
            let start = Instant::now();
            session.with_vault(|v| v.note(id)).unwrap();
            slowest = slowest.max(start.elapsed());
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(slowest < Duration::from_millis(40), "a vault call waited {slowest:?}");
        session.lock();
    });
}

#[test]
fn the_job_stops_promptly_on_lock() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::slow(4, Duration::from_millis(20));
    let source = Fakes(vec![fake.clone()]);
    session.with_vault(|v| {
        for i in 0..200 {
            v.create_note("", &format!("N{i}"), "some words")?;
        }
        Ok(())
    })
    .unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let statuses = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        s.spawn(|| {
            job::run(&session, g, &source, Duration::from_millis(10), |st| statuses.lock().unwrap().push(st.clone()));
            done_tx.send(()).unwrap();
        });
        wait_for(|| fake.calls() >= 3);
        let locked_at = Instant::now();
        session.lock();
        done_rx.recv_timeout(Duration::from_secs(2)).expect("job stops on lock");
        assert!(locked_at.elapsed() < Duration::from_millis(500));
    });
    assert!(fake.calls() < 200);
    assert!(
        statuses.lock().unwrap().iter().any(|s| matches!(s, IndexStatus::Indexing { total: 200, .. })),
        "progress is reported"
    );
}

#[test]
fn switching_models_keeps_the_old_vectors_until_the_new_ones_are_complete() {
    let (_dir, path, session, g) = session("fake-4");
    let four = Fake::new(4);
    let eight = Fake::slow(8, Duration::from_millis(15));
    let source = Fakes(vec![four.clone(), eight.clone()]);
    let ids: Vec<i64> = session
        .with_vault(|v| (0..40).map(|i| v.create_note("", &format!("N{i}"), &format!("text {i}"))).collect())
        .unwrap();
    index(&session, g, &source);
    assert_eq!(session.with_vault(|v| v.embedding_model_ready()).unwrap().as_deref(), Some("fake-4"));

    // Switch, and lock partway through the new model.
    session.with_vault(|v| v.set_embedding_model("fake-8")).unwrap();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(10), |_| {}));
        wait_for(|| session.with_vault_quiet(g, |v| v.embedding_count("fake-8")).unwrap() >= 1);
        for &id in &ids {
            assert_eq!(vector(&session, id).unwrap().len(), 4, "reads still use the old model");
        }
        session.lock();
    });
    let vault = Vault::open(&path, common::PASS).unwrap();
    let partial = vault.embedding_count("fake-8").unwrap();
    assert!(partial >= 1 && partial < ids.len(), "{partial} of {}", ids.len());
    assert_eq!(vault.embedding_model_ready().unwrap().as_deref(), Some("fake-4"));
    assert_eq!(vault.note_embedding(ids[0]).unwrap().unwrap().len(), 4);

    // Unlocking resumes the switch where it stopped.
    let calls = eight.calls();
    let g = session.unlock(vault, path).unwrap();
    index(&session, g, &source);
    assert_eq!(eight.calls() - calls, ids.len() - partial, "only the rest is embedded");
    assert_eq!(session.with_vault(|v| v.embedding_model_ready()).unwrap().as_deref(), Some("fake-8"));
    for &id in &ids {
        assert_eq!(vector(&session, id).unwrap().len(), 8);
    }
    assert_eq!(count(&session, "fake-4"), 0, "the old model's rows are gone");
}

#[test]
fn a_model_with_a_wrong_checksum_fails_cleanly_and_notes_still_open() {
    let models = tempfile::tempdir().unwrap();
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    std::fs::write(models.path().join(d.tokenizer.name), b"{}").unwrap();
    std::fs::write(models.path().join(d.model.name), b"not a model").unwrap();
    assert!(matches!(OnnxEmbedder::load(models.path(), d), Err(EmbedError::Checksum(_))));

    let (_dir, _path, session, g) = session(embed::DEFAULT_MODEL);
    let id = session.with_vault(|v| v.create_note("", "T", "Text")).unwrap();
    let source = Bundled(models.path().to_path_buf());
    let statuses = std::sync::Mutex::new(Vec::new());
    job::run(&session, g, &source, Duration::from_millis(10), |s| statuses.lock().unwrap().push(s.clone()));
    let statuses = statuses.into_inner().unwrap();
    assert!(matches!(statuses.as_slice(), [IndexStatus::Failed { message }] if message.contains("checksum")));

    assert_eq!(session.with_vault(|v| v.note(id)).unwrap().body, "Text");
    assert_eq!(vector(&session, id), None);
}

/// Runs the real bundled model. Needs the files from `npm run fetch-model`.
#[test]
fn bundled_model_puts_similar_notes_closer_than_unrelated_ones() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    if !dir.join(d.model.name).is_file() || !dir.join(d.tokenizer.name).is_file() {
        eprintln!("SKIPPED: model files missing in {}; run `npm run fetch-model`", dir.display());
        return;
    }
    let model = OnnxEmbedder::load(&dir, d).unwrap();
    let text = |title: &str, body: &str| embed::note_text(title, body).unwrap();
    let a = model.embed(&text("Groceries", "Buy milk, eggs and bread at the supermarket tomorrow.")).unwrap();
    let b = model.embed(&text("Shopping", "Tomorrow: get bread, eggs and milk from the store.")).unwrap();
    let c = model.embed(&text("Kernel panic", "The server crashed after the driver update; roll back the firmware.")).unwrap();
    assert_eq!(a.len(), d.dims);
    let (near, far) = (embed::cosine(&a, &b), embed::cosine(&a, &c));
    assert!(near > far, "similar {near} vs unrelated {far}");

    // A note longer than one chunk still gets one unit-length vector.
    let long = text("Long", &"The quick brown fox jumps over the lazy dog. ".repeat(400));
    let v = model.embed(&long).unwrap();
    assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3);
}

/// Batched and one-at-a-time embedding give the same vectors with the real model:
/// texts of different lengths, one longer than a chunk, run together. Needs the files
/// from `npm run fetch-model`.
#[test]
fn bundled_model_gives_the_same_vectors_batched() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    if !dir.join(d.model.name).is_file() || !dir.join(d.tokenizer.name).is_file() {
        eprintln!("SKIPPED: model files missing in {}; run `npm run fetch-model`", dir.display());
        return;
    }
    let model = OnnxEmbedder::load(&dir, d).unwrap();
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(150);
    let texts = ["Short note.", "Groceries\n\nBuy milk, eggs and bread at the supermarket tomorrow.", long.as_str(), "A", "Kernel panic after the driver update; roll back the firmware."];
    let batched = model.embed_batch(&texts, &|| false).unwrap();
    for (text, b) in texts.iter().zip(&batched) {
        let single = model.embed(text).unwrap();
        let worst = single.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{worst} for {:?}", &text[..text.len().min(20)]);
    }
    assert!(matches!(model.embed_batch(&texts, &|| true), Err(EmbedError::Stopped)));
}

/// Records the order notes are embedded in, by the first word of their text (title).
struct Recorder {
    seen: std::sync::Mutex<Vec<String>>,
}

impl Embedder for Recorder {
    fn model_id(&self) -> &str {
        "fake-4"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.seen.lock().unwrap().push(text.split_whitespace().next().unwrap_or("").to_string());
        Ok(vec![1.0, 0.0, 0.0, 0.0])
    }
}

/// Imports notes titled `n<updated_at>` with the given `updated_at`s.
fn import_dated(session: &Session, dates: &[i64]) {
    let titles: Vec<String> = dates.iter().map(|d| format!("n{d}")).collect();
    let notes: Vec<NewNote> = dates
        .iter()
        .zip(&titles)
        .map(|(&d, t)| NewNote { folder: "", title: t, body: "words", created_at: 0, updated_at: d })
        .collect();
    for r in session.with_vault(|v| v.import_notes(&notes)).unwrap() {
        r.unwrap();
    }
}

#[test]
fn the_queue_drains_most_recently_updated_first() {
    struct Shared(Arc<Recorder>);
    impl EmbedderSource for Shared {
        fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
            Ok(self.0.clone())
        }
    }
    // A short queue (sorted) and one long enough to be read through the index on
    // `updated_at` instead, across several pages.
    for n in [6usize, 5_100] {
        let (_dir, _path, session, g) = session("fake-4");
        let dates: Vec<i64> = (0..n as i64).map(|i| (i * 7919) % n as i64 + 1).collect();
        import_dated(&session, &dates);
        let recorder = Arc::new(Recorder { seen: std::sync::Mutex::new(Vec::new()) });
        index(&session, g, &Shared(recorder.clone()));
        let seen: Vec<i64> = recorder.seen.lock().unwrap().iter().map(|t| t[1..].parse().unwrap()).collect();
        let expected: Vec<i64> = (1..=n as i64).rev().collect();
        assert_eq!(seen.len(), n);
        assert!(seen == expected, "{n} notes: embedded out of order, starting {:?}", &seen[..seen.len().min(10)]);
    }
}

fn queued(session: &Session) -> usize {
    session.with_vault(|v| v.embedding_queue_len()).unwrap()
}

/// Every write that can change a note's embedded text queues the note, in its own
/// transaction (triggers on `notes`), and a pass empties the queue again.
#[test]
fn every_write_path_queues_the_note() {
    let (_dir, _path, session, g) = session("fake-4");
    let source = Fakes(vec![Fake::new(4)]);
    let id = session.with_vault(|v| v.create_note("a", "Walk", "A walk in the park")).unwrap();
    assert_eq!(queued(&session), 1, "create");
    index(&session, g, &source);
    assert_eq!(queued(&session), 0);

    let check = |what: &str, f: &dyn Fn(&mut Vault)| {
        session.with_vault(|v| {
            f(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(queued(&session), 1, "{what}");
        index(&session, g, &source);
        assert_eq!(queued(&session), 0, "{what}: the pass empties the queue");
    };
    check("update_body", &|v| v.update_body(id, "Rain all day").unwrap());
    check("rename", &|v| v.rename_note(id, "a", "Rain").unwrap());
    check("bulk metadata edit", &|v| {
        v.edit_notes_metadata(&[id], &MetadataEdit::AddTag("x".into())).unwrap();
    });
    check("accept suggestion", &|v| v.accept_tag_suggestion(id, "y").unwrap());
    check("create tag from group", &|v| {
        v.create_tag_from_group("z", "", &[id]).unwrap();
    });
    check("trash", &|v| v.trash_note(id).unwrap());
    check("restore", &|v| {
        v.restore_note(id).unwrap();
    });
    check("import", &|v| {
        let notes = [NewNote { folder: "imp", title: "Imported", body: "Imported words", created_at: 1, updated_at: 1 }];
        v.import_notes(&notes).unwrap();
    });

    // Moving to another folder doesn't change the embedded text, so nothing is queued.
    session.with_vault(|v| v.move_notes(&[id], "b")).unwrap();
    assert_eq!(queued(&session), 0);
}

/// A save made while the model runs keeps its queue row, so the new text is embedded next.
#[test]
fn a_save_during_embedding_is_not_lost() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let id = session.with_vault(|v| v.create_note("", "T", "first text")).unwrap();
    let page = session.with_vault(|v| v.queued_embeddings("fake-4", &Default::default(), 10)).unwrap();
    assert_eq!(page.stale.len(), 1);
    // Saved after the job read the text, before it wrote the vector.
    session.with_vault(|v| v.update_body(id, "second text")).unwrap();
    let q = &page.stale[0];
    let embedded = fake.embed(&q.note.text).unwrap();
    let written = hush_lib::db::QueuedVector { id, version: q.version, hash: q.note.hash, vector: embedded };
    session.with_vault(|v| v.write_queued_embeddings("fake-4", &[written])).unwrap();
    assert_eq!(queued(&session), 1, "still queued for the second text");

    let source = Fakes(vec![fake.clone()]);
    index(&session, g, &source);
    assert_eq!(fake.calls(), 2);
    assert_eq!(vector(&session, id).unwrap(), fake.embed("T\n\nsecond text").unwrap());
}

/// Counts passes: the job asks for its embedder once at the start of each.
struct CountingFakes {
    fakes: Fakes,
    passes: AtomicUsize,
}

impl EmbedderSource for CountingFakes {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        self.passes.fetch_add(1, Ordering::SeqCst);
        self.fakes.embedder(model)
    }
}

#[test]
fn the_job_runs_one_pass_per_save_and_does_not_wake_itself() {
    let (_dir, _path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let source = CountingFakes { fakes: Fakes(vec![fake.clone()]), passes: AtomicUsize::new(0) };
    let ids: Vec<i64> =
        session.with_vault(|v| (0..20).map(|i| v.create_note("", &format!("N{i}"), &format!("text {i}"))).collect()).unwrap();
    let passes = || source.passes.load(Ordering::SeqCst);
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(10), |_| {}));
        wait_for(|| count(&session, "fake-4") == 20);
        // Unlock pass, then the safety-net scan finds nothing: the job stays idle.
        std::thread::sleep(Duration::from_millis(200));
        let idle = passes();
        assert_eq!(idle, 1, "one pass after unlock embeds everything");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(passes(), idle, "its own writes don't wake it");

        for (n, &id) in ids.iter().take(3).enumerate() {
            let calls = fake.calls();
            let before = passes();
            session.with_vault(|v| v.update_body(id, &format!("changed {n}"))).unwrap();
            wait_for(|| fake.calls() == calls + 1);
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(passes() - before, 1, "save {n}");
        }
        session.lock();
    });
}

/// A write that bypasses the queue (a code path that forgot it) is caught by the
/// scan the job runs once per unlock.
#[test]
fn the_unlock_safety_net_finds_unqueued_changes() {
    let (_dir, path, session, g) = session("fake-4");
    let fake = Fake::new(4);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "T", "first text")).unwrap();
    index(&session, g, &source);

    // Change the text on a second connection and drop the queue row the trigger made.
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "key", common::PASS).unwrap();
    raw.execute("UPDATE notes SET body = 'other text' WHERE id = ?1", [id]).unwrap();
    raw.execute("DELETE FROM embedding_dirty", []).unwrap();
    drop(raw);
    assert_eq!(queued(&session), 0);
    index(&session, g, &source);
    assert_eq!(fake.calls(), 1, "the queue alone misses it");

    // The running job scans once the queue is empty, and embeds what it finds.
    let expected = fake.embed("T\n\nother text").unwrap();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(10), |_| {}));
        // (`expected` counted one call.)
        wait_for(|| fake.calls() == 3);
        wait_for(|| session.with_vault_quiet(g, |v| v.note_embedding(id)).unwrap().as_ref() == Some(&expected));
        session.lock();
    });

    let g = session.unlock(Vault::open(&path, common::PASS).unwrap(), path.clone()).unwrap();
    assert_eq!(job::safety_net(&session, g).unwrap(), 0);
}

#[test]
fn a_model_switch_queues_every_live_note_once() {
    let (_dir, _path, session, g) = session("fake-4");
    let source = Fakes(vec![Fake::new(4), Fake::new(8)]);
    let ids: Vec<i64> =
        session.with_vault(|v| (0..5).map(|i| v.create_note("", &format!("N{i}"), "words")).collect()).unwrap();
    session.with_vault(|v| v.trash_note(ids[0])).unwrap();
    index(&session, g, &source);
    session.with_vault(|v| v.set_embedding_model("fake-4")).unwrap();
    assert_eq!(queued(&session), 0, "setting the same model queues nothing");
    session.with_vault(|v| v.set_embedding_model("fake-8")).unwrap();
    assert_eq!(queued(&session), 4);
}
