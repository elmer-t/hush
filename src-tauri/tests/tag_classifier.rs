mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hush_lib::classify::{self, job as training};
use hush_lib::db::{ScoreMethod, TagTraining, TrainingStatus, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A fast, deterministic embedder: words hashed into `dims` buckets.
struct Fake {
    id: String,
    dims: usize,
}

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0; self.dims];
        for word in text.split_whitespace() {
            let h = word.bytes().fold(2166136261u32, |h, b| {
                (h ^ u32::from(b)).wrapping_mul(16777619)
            });
            v[h as usize % self.dims] += 1.0;
        }
        v[0] += 0.01;
        embed::normalize(&mut v);
        Ok(v)
    }
}

struct Fakes(Vec<Arc<Fake>>);

impl Fakes {
    fn new(dims: &[usize]) -> Self {
        Self(
            dims.iter()
                .map(|d| {
                    Arc::new(Fake {
                        id: format!("fake-{d}"),
                        dims: *d,
                    })
                })
                .collect(),
        )
    }
}

impl EmbedderSource for Fakes {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let fake = self
            .0
            .iter()
            .find(|f| f.id == model)
            .ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(fake.clone())
    }
}

fn session(model: &str, threshold: f32) -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(model).unwrap();
    vault
        .set_suggestion_threshold(model, Some(threshold))
        .unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

/// Runs embedding passes until one finds nothing to do.
fn index(session: &Session, generation: u64, source: &dyn EmbedderSource) {
    for _ in 0..10 {
        if job::pass(session, generation, source, &mut |_| {})
            .unwrap()
            .stale
            == 0
        {
            return;
        }
    }
    panic!("indexing never finished");
}

fn create(session: &Session, title: &str, body: &str) -> i64 {
    session
        .with_vault(|v| v.create_note("", title, body))
        .unwrap()
}

/// `n` notes about dreams tagged `tag`, and `others` notes about money without it.
fn notes(session: &Session, tag: &str, n: usize, others: usize) -> Vec<i64> {
    let mut ids = Vec::new();
    for i in 0..n {
        ids.push(create(
            session,
            &format!("{tag} {i}"),
            &format!("---\ntags: [{tag}]\n---\ndream sleep night moon w{i}"),
        ));
    }
    for i in 0..others {
        create(
            session,
            &format!("other {tag} {i}"),
            &format!("invoice budget meeting office x{i}"),
        );
    }
    ids
}

fn training_of(session: &Session, tag: &str) -> TagTraining {
    session
        .with_vault(|v| v.tag_training())
        .unwrap()
        .into_iter()
        .find(|t| t.tag == tag)
        .unwrap()
}

fn suggestions(session: &Session, id: i64) -> Vec<(String, ScoreMethod)> {
    session
        .with_vault(|v| v.tag_suggestions(id))
        .unwrap()
        .into_iter()
        .map(|s| (s.tag, s.method))
        .collect()
}

#[test]
fn a_tag_crossing_ten_examples_switches_from_its_description_to_a_head() {
    let source = Fakes::new(&[64]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let dreams = notes(&session, "dream", 9, 15);
    let target = create(&session, "Last night", "dream sleep night star");
    session
        .with_vault(|v| v.set_tag_description("dream", "dream sleep night"))
        .unwrap();
    index(&session, g, &source);

    // 9 examples: description blended with their centroid; no head yet.
    let pass = training::pass(&session, g).unwrap();
    assert!(pass.trained.is_empty(), "{pass:?}");
    let t = training_of(&session, "dream");
    assert_eq!(
        (t.positives, t.negatives, t.status, t.needed),
        (9, 16, TrainingStatus::DescriptionOnly, 1)
    );
    assert_eq!((t.precision, t.recall, t.trained_at), (None, None, None));
    assert_eq!(
        suggestions(&session, target),
        [("dream".to_string(), ScoreMethod::DescriptionAndExamples)]
    );

    // The 10th example: waiting for the job, then trained.
    create(
        &session,
        "dream 9",
        "---\ntags: [dream]\n---\ndream sleep night moon w9",
    );
    index(&session, g, &source);
    assert_eq!(
        training_of(&session, "dream").status,
        TrainingStatus::Waiting
    );
    let pass = training::pass(&session, g).unwrap();
    assert_eq!(pass.trained, ["dream"]);
    assert_eq!(pass.durations.len(), 1);
    let t = training_of(&session, "dream");
    assert_eq!(
        (t.positives, t.status, t.needed),
        (10, TrainingStatus::Trained, 0)
    );
    // Every tagged note is found; the untagged dream note counts as a wrong guess.
    assert_eq!(t.recall, Some(1.0));
    assert!(t.precision.unwrap() >= 10.0 / 11.0, "{t:?}");
    assert!(t.trained_at.is_some());
    let head = session
        .with_vault(|v| v.tag_head("dream"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            head.model.as_str(),
            head.head.weights.len(),
            head.positives,
            head.negatives
        ),
        ("fake-64", 64, 10, 16)
    );
    assert_eq!(
        session
            .with_vault(|v| v.tag_score(target, "dream"))
            .unwrap()
            .unwrap()
            .1,
        ScoreMethod::Classifier
    );

    // Nothing changed: nothing is retrained.
    let pass = training::pass(&session, g).unwrap();
    assert!(
        pass.trained.is_empty() && pass.current == 1 && pass.dropped == 0,
        "{pass:?}"
    );

    // A new note gets the tag suggested by the head.
    let tonight = create(&session, "Tonight", "dream sleep night moon again");
    index(&session, g, &source);
    let suggested = session.with_vault(|v| v.tag_suggestions(tonight)).unwrap();
    assert_eq!(suggested.len(), 1);
    assert_eq!(
        (suggested[0].tag.as_str(), suggested[0].method),
        ("dream", ScoreMethod::Classifier)
    );
    assert!(suggested[0].score >= classify::PROBABILITY_THRESHOLD);

    // A head doesn't need a description; a tag with neither isn't suggested.
    session
        .with_vault(|v| v.set_tag_description("dream", ""))
        .unwrap();
    assert_eq!(
        suggestions(&session, tonight),
        [("dream".to_string(), ScoreMethod::Classifier)]
    );
    create(
        &session,
        "Tagged once",
        "---\ntags: [rare]\n---\ndream sleep night",
    );
    index(&session, g, &source);
    let t = training_of(&session, "rare");
    assert_eq!((t.status, t.needed), (TrainingStatus::NeedsExamples, 9));
    assert!(
        suggestions(&session, tonight)
            .iter()
            .all(|(tag, _)| tag != "rare")
    );

    // Removing a tag that has a head by hand records a rejection (it can be suggested).
    session
        .with_vault(|v| v.update_body(dreams[0], "dream sleep night moon w0"))
        .unwrap();
    let feedback = session.with_vault(|v| v.tag_feedback(dreams[0])).unwrap();
    assert_eq!(
        feedback
            .iter()
            .map(|f| (f.tag.as_str(), f.accepted))
            .collect::<Vec<_>>(),
        [("dream", false)]
    );

    // Back under 10 examples: the head is dropped and the tag falls back.
    index(&session, g, &source);
    let pass = training::pass(&session, g).unwrap();
    assert_eq!((pass.trained.len(), pass.dropped), (0, 1), "{pass:?}");
    assert_eq!(session.with_vault(|v| v.tag_head("dream")).unwrap(), None);
    assert_eq!(
        training_of(&session, "dream").status,
        TrainingStatus::NeedsExamples
    );
    assert!(
        suggestions(&session, target).is_empty(),
        "no head and no description"
    );
}

#[test]
fn a_rejection_lowers_that_notes_score_after_retraining() {
    let source = Fakes::new(&[64]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 12, 15);
    let unsure = create(&session, "Mixed", "dream sleep invoice budget");
    index(&session, g, &source);
    assert_eq!(training::pass(&session, g).unwrap().trained, ["dream"]);
    let (before, method) = session
        .with_vault(|v| v.tag_score(unsure, "dream"))
        .unwrap()
        .unwrap();
    assert_eq!(method, ScoreMethod::Classifier);
    let first = session
        .with_vault(|v| v.tag_head("dream"))
        .unwrap()
        .unwrap();

    session
        .with_vault(|v| v.reject_tag_suggestion(unsure, "dream"))
        .unwrap();
    assert_eq!(
        session
            .with_vault(|v| v.tag_score(unsure, "dream"))
            .unwrap()
            .unwrap()
            .0,
        before,
        "not retrained yet"
    );
    assert_eq!(training_of(&session, "dream").rejected, 1);
    assert_eq!(
        training::pass(&session, g).unwrap().trained,
        ["dream"],
        "the feedback changed its inputs"
    );
    let (after, _) = session
        .with_vault(|v| v.tag_score(unsure, "dream"))
        .unwrap()
        .unwrap();
    assert!(after < before, "{after} < {before}");
    let second = session
        .with_vault(|v| v.tag_head("dream"))
        .unwrap()
        .unwrap();
    assert_ne!(second.inputs, first.inputs);
    assert!(second.trained_at >= first.trained_at);
    assert!(
        suggestions(&session, unsure).is_empty(),
        "rejected on this note"
    );
}

#[test]
fn heads_are_dropped_and_retrained_when_the_model_changes() {
    let source = Fakes::new(&[64, 32]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 10, 12);
    let target = create(&session, "Last night", "dream sleep night moon");
    session
        .with_vault(|v| v.set_tag_description("dream", "dream sleep night"))
        .unwrap();
    index(&session, g, &source);
    assert_eq!(training::pass(&session, g).unwrap().trained, ["dream"]);

    session
        .with_vault(|v| v.set_suggestion_threshold("fake-32", Some(0.3)))
        .unwrap();
    session
        .with_vault(|v| v.set_embedding_model("fake-32"))
        .unwrap();
    index(&session, g, &source);
    assert_eq!(
        session
            .with_vault(|v| v.embedding_model_ready())
            .unwrap()
            .as_deref(),
        Some("fake-32")
    );
    // The old model's head is ignored at once: the tag falls back to its description.
    assert_eq!(
        session
            .with_vault(|v| v.tag_head("dream"))
            .unwrap()
            .unwrap()
            .model,
        "fake-64"
    );
    assert_eq!(
        suggestions(&session, target),
        [("dream".to_string(), ScoreMethod::DescriptionAndExamples)]
    );
    assert_eq!(
        training_of(&session, "dream").status,
        TrainingStatus::Waiting
    );

    let pass = training::pass(&session, g).unwrap();
    assert_eq!(
        (pass.trained.as_slice(), pass.dropped),
        (["dream".to_string()].as_slice(), 1)
    );
    let head = session
        .with_vault(|v| v.tag_head("dream"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (head.model.as_str(), head.head.weights.len()),
        ("fake-32", 32)
    );
    assert_eq!(
        session
            .with_vault(|v| v.tag_score(target, "dream"))
            .unwrap()
            .unwrap()
            .1,
        ScoreMethod::Classifier
    );
    let tonight = create(&session, "Tonight", "dream sleep night moon again");
    index(&session, g, &source);
    assert_eq!(
        suggestions(&session, tonight),
        [("dream".to_string(), ScoreMethod::Classifier)]
    );

    // A head for a model that is no longer ready is refused.
    let mut stale = head.clone();
    stale.model = "fake-64".into();
    assert!(!session.with_vault(|v| v.write_tag_head(&stale)).unwrap());
    assert_eq!(
        session
            .with_vault(|v| v.tag_head("dream"))
            .unwrap()
            .unwrap()
            .model,
        "fake-32"
    );
}

#[test]
fn training_does_not_bump_activity_and_stops_on_lock() {
    let source = Fakes::new(&[64]);
    let (_dir, path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 10, 12);
    index(&session, g, &source);

    std::thread::sleep(Duration::from_millis(50));
    let idle = session.idle_for().unwrap();
    assert_eq!(training::pass(&session, g).unwrap().trained, ["dream"]);
    assert!(
        session.idle_for().unwrap() >= idle,
        "training counted as activity"
    );

    // Locked: a pass stops at once, and a running job ends by itself.
    session.lock();
    assert!(matches!(
        training::pass(&session, g),
        Err(training::TrainError::Locked)
    ));
    let g = session
        .unlock(Vault::open(&path, common::PASS).unwrap(), path)
        .unwrap();
    std::thread::scope(|s| {
        let job = s.spawn(|| {
            training::run(
                &session,
                g,
                Duration::from_millis(10),
                Duration::ZERO,
                |_| {},
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        session.lock();
        while !job.is_finished() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "the job didn't stop"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    // It never touches a vault unlocked later.
    assert!(matches!(
        training::pass(&session, g),
        Err(training::TrainError::Locked)
    ));
}

/// The running job trains on unlock, then again once the vault has been quiet.
#[test]
fn the_running_job_retrains_after_changes() {
    let source = Fakes::new(&[64]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 10, 12);
    let unsure = create(&session, "Mixed", "dream sleep invoice budget");
    index(&session, g, &source);
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    std::thread::scope(|s| {
        s.spawn(|| {
            training::run(
                &session,
                g,
                Duration::from_millis(10),
                Duration::from_millis(100),
                |_| {},
            )
        });
        wait_for("the first head", &|| {
            session
                .with_vault(|v| v.tag_head("dream"))
                .unwrap()
                .is_some()
        });
        let first = session
            .with_vault(|v| v.tag_head("dream"))
            .unwrap()
            .unwrap();
        session
            .with_vault(|v| v.reject_tag_suggestion(unsure, "dream"))
            .unwrap();
        wait_for("retraining", &|| {
            session
                .with_vault(|v| v.tag_head("dream"))
                .unwrap()
                .unwrap()
                .inputs
                != first.inputs
        });
        session.lock();
    });
}

/// Every batch the embedding job writes changes every head's inputs: the job waits for
/// the queue to empty instead of retraining between batches.
#[test]
fn the_running_job_waits_while_notes_are_queued_for_embedding() {
    let source = Fakes::new(&[64]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 10, 12);
    index(&session, g, &source);
    create(&session, "Queued", "dream sleep night moon");
    let head = || session.with_vault(|v| v.tag_head("dream")).unwrap();
    std::thread::scope(|s| {
        // A stall counts after 10 quiet periods: 1 s.
        s.spawn(|| {
            training::run(
                &session,
                g,
                Duration::from_millis(5),
                Duration::from_millis(100),
                |_| {},
            )
        });
        let _stop = LockOnDrop(&session);
        std::thread::sleep(Duration::from_millis(400));
        assert!(head().is_none(), "trained while a note was queued");
        let status = session.activity().status(hush_lib::activity::Job::Training);
        assert_eq!(
            status.reason.as_deref(),
            Some("notes being embedded (1 queued)")
        );

        index(&session, g, &source);
        let start = Instant::now();
        while head().is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "never trained after the queue emptied"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            head().unwrap().negatives,
            13,
            "trained on the embedded note too"
        );
    });
}

/// A queue that stops shrinking (the model can't load) doesn't hold training back for good.
#[test]
fn the_running_job_trains_anyway_once_the_queue_stalls() {
    let source = Fakes::new(&[64]);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    notes(&session, "dream", 10, 12);
    index(&session, g, &source);
    create(&session, "Queued", "dream sleep night moon");
    std::thread::scope(|s| {
        s.spawn(|| {
            training::run(
                &session,
                g,
                Duration::from_millis(5),
                Duration::from_millis(20),
                |_| {},
            )
        });
        let _stop = LockOnDrop(&session);
        let start = Instant::now();
        while session
            .with_vault(|v| v.tag_head("dream"))
            .unwrap()
            .is_none()
        {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "a stalled queue blocked training"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            start.elapsed() >= Duration::from_millis(200),
            "didn't wait for the stall"
        );
    });
}

/// Locks the vault when dropped, so a failed assertion stops the job instead of hanging the test.
struct LockOnDrop<'a>(&'a Session);

impl Drop for LockOnDrop<'_> {
    fn drop(&mut self) {
        self.0.lock();
    }
}

/// How long training takes and how good heads get, with the real model. See
/// `docs/decisions.md` (Tag classifiers). Needs the bundled model (`npm run fetch-model`),
/// or `HUSH_MEASURE_ONNX=<file in src-tauri/models>` for another export of it. Run in
/// release:
///
/// `cargo test --release --manifest-path src-tauri/Cargo.toml --test tag_classifier -- --ignored --nocapture`
#[test]
#[ignore = "needs the real model; prints measurements"]
fn measure_training_time_and_quality() {
    measure::run();
}

mod measure {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use hush_lib::classify::{self, Config, Example, job as training};
    use hush_lib::embed::onnx::OnnxEmbedder;
    use hush_lib::embed::{self, Embedder, ModelDescriptor};
    use hush_lib::import;
    use hush_lib::session::Session;

    use super::common;

    fn model() -> OnnxEmbedder {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
        let mut descriptor: ModelDescriptor = *embed::descriptor(embed::DEFAULT_MODEL).unwrap();
        if let Ok(file) = std::env::var("HUSH_MEASURE_ONNX") {
            use sha2::Digest;
            let bytes = std::fs::read(&file).unwrap();
            let path = Path::new(&file);
            assert_eq!(
                path.parent().unwrap().canonicalize().unwrap(),
                dir.canonicalize().unwrap(),
                "put it in {}",
                dir.display()
            );
            descriptor.model.name = Box::leak(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
                    .into_boxed_str(),
            );
            let sha: String = sha2::Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            descriptor.model.sha256 = Box::leak(sha.into_boxed_str());
        }
        OnnxEmbedder::load(&dir, Box::leak(Box::new(descriptor)))
            .expect("model files missing; run `npm run fetch-model`")
    }

    /// A tiny deterministic generator, so every run uses the same synthetic notes.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }
    }

    const DREAM: &[&str] = &[
        "I dreamt I was flying over the old town at night.",
        "In the dream my teeth kept falling out and nobody noticed.",
        "Another nightmare: the house had rooms I had never seen.",
        "I woke up at four, heart racing, still half inside the dream.",
        "The sea came into the bedroom, warm and quiet, and I could breathe under water.",
        "My grandmother was alive again in the dream and we baked bread.",
        "I was late for an exam in a school that kept changing shape.",
        "Strange dream about a train that never stopped at any station.",
        "Lucid for a moment: I knew I was dreaming and tried to fly.",
        "Slept badly; the same chase dream as last week.",
    ];
    const JOURNAL: &[&str] = &[
        "Walked the dog along the canal this morning; cold but bright.",
        "Long day at work, then soup and an early night.",
        "Met Anna for coffee, talked about her new job for hours.",
        "Rain all afternoon. Read on the sofa and felt lazy about it.",
        "Cycled to the market, bought tulips and far too much cheese.",
        "Today I finally fixed the leaking tap in the kitchen.",
        "Called my brother; he is moving to Utrecht next month.",
        "Quiet Sunday. Cleaned the flat and wrote a few letters.",
        "Felt tired and grumpy all day, not sure why.",
        "Took the ferry to the island and walked the dunes until sunset.",
    ];
    const OTHER: &[&str] = &[
        "Agenda for the budget meeting: Q3 figures, hiring, office move.",
        "Recipe: two onions, a tin of tomatoes, lentils, cumin, simmer 30 minutes.",
        "Ferry timetable changes from 1 May; first boat leaves 06:15.",
        "Invoice 2021-044 paid by bank transfer, reference attached.",
        "Notes on the router setup: channel 6, WPA2, guest network off.",
        "Packing list: passport, chargers, rain jacket, walking boots.",
        "Book notes: the second half drags but the ending is strong.",
        "Dear Jan, thank you for your letter and the photos of the garden.",
        "Minutes: the committee agreed to repaint the hall in spring.",
        "Car service due at 60,000 km; tyres need replacing soon.",
    ];

    fn note(rng: &mut Rng, pool: &[&str], i: usize) -> String {
        let n = 2 + rng.below(3);
        let mut parts: Vec<&str> = (0..n).map(|_| rng.pick(pool)).collect();
        // A sentence from another pool now and then, as real notes wander.
        if rng.below(3) == 0 {
            parts.push(rng.pick(OTHER));
        }
        format!("Note {i}\n\n{}", parts.join(" "))
    }

    pub fn run() {
        let model = model();
        let config = Config::default();

        // 1. The fixture vault imported with the default rules: journal/dream come from
        //    the file-name label rule.
        let (vault_dir, vault) = common::vault();
        let session = Session::default();
        session
            .unlock(vault, vault_dir.path().join("vault.hush"))
            .unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
        import::run(
            &session,
            &fixture,
            "",
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let fixture_notes: Vec<(Vec<String>, Vec<f32>)> = session
            .with_vault(|v| {
                let mut out = Vec::new();
                for folder in v.folders()? {
                    for n in v.folder_notes(&folder, 0, 1000)? {
                        let note = v.note(n.id)?;
                        if let Some(text) = embed::note_text(&note.title, &note.body) {
                            out.push((note.tags.clone(), model.embed(&text).unwrap()));
                        }
                    }
                }
                Ok(out)
            })
            .unwrap();
        for tag in ["journal", "dream"] {
            let pos = fixture_notes
                .iter()
                .filter(|(t, _)| t.iter().any(|x| x == tag))
                .count();
            println!(
                "fixture: {} notes with vectors, {pos} tagged {tag}",
                fixture_notes.len()
            );
            if pos >= 2 {
                let ex: Vec<Example> = fixture_notes
                    .iter()
                    .map(|(t, x)| Example {
                        x,
                        positive: t.iter().any(|x| x == tag),
                        weight: 1.0,
                    })
                    .collect();
                let q = classify::cross_validate(
                    &ex,
                    pos.min(classify::FOLDS),
                    1,
                    None,
                    &config,
                    &|| true,
                )
                .unwrap();
                println!(
                    "fixture {tag}: cross-validated precision {:?}, recall {:?}",
                    q.precision, q.recall
                );
            }
        }

        // 2. Synthetic notes written from journal-, dream- and other-like sentences.
        let mut rng = Rng(7);
        let mut texts: Vec<(&str, String)> = Vec::new();
        for i in 0..120 {
            texts.push(("dream", note(&mut rng, DREAM, i)));
            texts.push(("journal", note(&mut rng, JOURNAL, i)));
            texts.push(("other", note(&mut rng, OTHER, i)));
        }
        let start = Instant::now();
        let vectors: Vec<(&str, Vec<f32>)> = texts
            .iter()
            .map(|(t, s)| (*t, model.embed(s).unwrap()))
            .collect();
        println!(
            "embedded {} synthetic notes in {:.1?}",
            vectors.len(),
            start.elapsed()
        );

        // Learning curve: train on k positives, 3k negatives and k/2 (at least 2) notes
        // that belong to the tag but weren't tagged (labelled negative, as in a real
        // vault); test on every other note. "untagged found" is how many of those the
        // head then suggests: the notes suggestions exist for. 5 random draws per k.
        for l2 in [1e-4, 1e-3, 1e-2, 3e-2, 1e-1] {
            let config = Config { l2, ..config };
            println!(
                "-- L2 {l2}{}",
                if l2 == Config::default().l2 {
                    " (default)"
                } else {
                    ""
                }
            );
            for tag in ["journal", "dream"] {
                for k in [5, 10, 20, 50] {
                    let (mut tp, mut fp, mut fn_, mut found, mut hidden_total) = (0, 0, 0, 0, 0);
                    for rep in 0..5u64 {
                        let mut rng = Rng(100 + rep);
                        let mut pos: Vec<usize> = (0..vectors.len())
                            .filter(|&i| vectors[i].0 == tag)
                            .collect();
                        let mut neg: Vec<usize> = (0..vectors.len())
                            .filter(|&i| vectors[i].0 != tag)
                            .collect();
                        for v in [&mut pos, &mut neg] {
                            for i in (1..v.len()).rev() {
                                let j = rng.below(i + 1);
                                v.swap(i, j);
                            }
                        }
                        let hidden = (k / 2).max(2);
                        let mut ex: Vec<Example> = pos[..k]
                            .iter()
                            .map(|&i| Example {
                                x: &vectors[i].1,
                                positive: true,
                                weight: 1.0,
                            })
                            .chain(pos[k..k + hidden].iter().chain(&neg[..3 * k]).map(|&i| {
                                Example {
                                    x: &vectors[i].1,
                                    positive: false,
                                    weight: 1.0,
                                }
                            }))
                            .collect();
                        classify::balance(&mut ex);
                        let head = classify::train(&ex, None, &config).head;
                        let suggests = |i: usize| {
                            head.probability(&vectors[i].1) >= classify::PROBABILITY_THRESHOLD
                        };
                        found += pos[k..k + hidden].iter().filter(|&&i| suggests(i)).count();
                        hidden_total += hidden;
                        for &i in pos[k + hidden..].iter().chain(&neg[3 * k..]) {
                            match (suggests(i), vectors[i].0 == tag) {
                                (true, true) => tp += 1,
                                (true, false) => fp += 1,
                                (false, true) => fn_ += 1,
                                _ => {}
                            }
                        }
                    }
                    let precision = tp as f32 / (tp + fp).max(1) as f32;
                    let recall = tp as f32 / (tp + fn_).max(1) as f32;
                    println!(
                        "synthetic {tag:8} {k:3} positives: precision {precision:.2}, recall {recall:.2}, untagged found {found}/{hidden_total}"
                    );
                }
            }
        }

        // 3. Training time on 10k notes: the synthetic vectors repeated with a little
        //    noise, a tag on 2% of them (like a real tag), training plus 5-fold CV.
        //    The whole job pass runs on a real (encrypted) vault, reading the vectors from it.
        let (big_dir, vault) = common::vault();
        let big = Session::default();
        let g = big
            .unlock(vault, big_dir.path().join("vault.hush"))
            .unwrap();
        let model_id = embed::DEFAULT_MODEL;
        let mut rng = Rng(11);
        let start = Instant::now();
        let ids = big
            .with_vault(|v| {
                let mut rows = Vec::new();
                for i in 0..10_000usize {
                    let (class, vector) = &vectors[i % vectors.len()];
                    let mut tags = Vec::new();
                    // Tags on about 2%, 2%, 5%, 11% and 33% of the notes.
                    if *class == "dream" && i % 17 == 0 {
                        tags.push("t2a")
                    }
                    if *class == "journal" && i % 17 == 0 {
                        tags.push("t2b")
                    }
                    if *class == "dream" && i % 7 == 0 {
                        tags.push("t5")
                    }
                    if *class == "other" && i % 3 == 0 {
                        tags.push("t11")
                    }
                    if *class == "dream" {
                        tags.push("t33")
                    }
                    let id = v.create_note(
                        "",
                        &format!("n{i}"),
                        &format!("---\ntags: [{}]\n---\nx{i}", tags.join(", ")),
                    )?;
                    let mut vector = vector.clone();
                    vector
                        .iter_mut()
                        .for_each(|x| *x += (rng.below(1000) as f32 / 1000.0 - 0.5) * 0.01);
                    embed::normalize(&mut vector);
                    rows.push((id, [0u8; 32], vector));
                }
                v.write_embeddings(model_id, &rows)?;
                v.finish_embedding_model(model_id)?;
                Ok(rows.into_iter().map(|r| r.0).collect::<Vec<i64>>())
            })
            .unwrap();
        println!("built a 10k-note vault in {:.1?}", start.elapsed());
        let start = Instant::now();
        let pass = training::pass(&big, g).unwrap();
        println!(
            "10k notes, first training pass: {:.2?} for {} tags",
            start.elapsed(),
            pass.trained.len()
        );
        for (tag, d) in pass.trained.iter().zip(&pass.durations) {
            let head = big.with_vault(|v| v.tag_head(tag)).unwrap().unwrap();
            println!(
                "  {tag:4} {:5} positives: {d:.2?} (train + 5-fold CV); cross-validated precision {:?}, recall {:?}",
                head.positives, head.precision, head.recall
            );
        }
        let start = Instant::now();
        let pass = training::pass(&big, g).unwrap();
        println!(
            "nothing changed: {:.2?}, {} retrained",
            start.elapsed(),
            pass.trained.len()
        );
        // Note 3 is a dream note without `t2a`.
        big.with_vault(|v| v.reject_tag_suggestion(ids[3], "t2a"))
            .unwrap();
        let start = Instant::now();
        let pass = training::pass(&big, g).unwrap();
        println!(
            "one rejection: {:.2?}, retrained {:?} (warm start; the rejection changes no other tag's inputs)",
            start.elapsed(),
            pass.trained
        );
    }
}
