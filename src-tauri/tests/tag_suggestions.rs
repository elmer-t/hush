mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hush_lib::db::{SearchQuery, TagFeedback, TagInfo, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::onnx::OnnxEmbedder;
use hush_lib::embed::{self, EmbedError, Embedder, ModelDescriptor};
use hush_lib::{import, markdown};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A fast, deterministic embedder: words hashed into `dims` buckets. Counts its calls.
struct Fake {
    id: String,
    dims: usize,
    calls: AtomicUsize,
}

impl Fake {
    fn new(dims: usize) -> Arc<Self> {
        Arc::new(Self { id: format!("fake-{dims}"), dims, calls: AtomicUsize::new(0) })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The score a note with this title and body gets against `description`.
    fn score(&self, title: &str, body: &str, description: &str) -> f32 {
        let text = embed::note_text(title, body).unwrap();
        embed::cosine(&self.vector(&text), &self.vector(description))
    }

    fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0; self.dims];
        for word in text.split_whitespace() {
            let h = word.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
            v[h as usize % self.dims] += 1.0;
        }
        v[0] += 0.01;
        embed::normalize(&mut v);
        v
    }
}

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.vector(text))
    }
}

struct Fakes(Vec<Arc<Fake>>);

impl EmbedderSource for Fakes {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let fake = self.0.iter().find(|f| f.id == model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(fake.clone())
    }
}

/// An unlocked session on a fresh vault that embeds with `model`, which suggests at
/// `threshold`.
fn session(model: &str, threshold: f32) -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(model).unwrap();
    vault.set_suggestion_threshold(model, Some(threshold)).unwrap();
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

fn suggested(session: &Session, id: i64) -> Vec<String> {
    session.with_vault(|v| v.tag_suggestions(id)).unwrap().into_iter().map(|s| s.tag).collect()
}

fn feedback(session: &Session, id: i64) -> Vec<(String, bool)> {
    session.with_vault(|v| v.tag_feedback(id)).unwrap().into_iter().map(|f: TagFeedback| (f.tag, f.accepted)).collect()
}

fn describe(session: &Session, tag: &str, description: &str) {
    session.with_vault(|v| v.set_tag_description(tag, description)).unwrap();
}

const BODY: &str = "dream sleep night moon";

#[test]
fn suggestions_respect_threshold_limit_existing_tags_and_rejections() {
    let fake = Fake::new(64);
    let (_dir, path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Night", &format!("---\ntags: [has]\n---\n{BODY}"))).unwrap();
    let descriptions = [
        ("aa", "dream sleep night moon"),
        ("bb", "dream sleep night"),
        ("cc", "dream sleep"),
        ("dd", "dream"),
        ("far", "invoice budget spreadsheet"),
        ("has", "dream sleep night moon"),
    ];
    for (tag, d) in descriptions {
        describe(&session, tag, d);
    }
    index(&session, g, &source);

    let score = |d: &str| fake.score("Night", BODY, d);
    assert!(score("invoice budget spreadsheet") < 0.3 && score("dream") >= 0.3, "fixture scores");
    let all = session.with_vault(|v| v.tag_suggestions(id)).unwrap();
    assert_eq!(all.iter().map(|s| s.tag.as_str()).collect::<Vec<_>>(), ["aa", "bb", "cc"], "best 3, without `has`");
    assert!(all.windows(2).all(|w| w[0].score >= w[1].score), "best first");
    assert!((all[0].score - score("dream sleep night moon")).abs() < 1e-6);

    // At the threshold counts; just above it doesn't.
    let c = score("dream sleep");
    session.with_vault(|v| v.set_suggestion_threshold("fake-64", Some(c))).unwrap();
    assert_eq!(suggested(&session, id), ["aa", "bb", "cc"]);
    session.with_vault(|v| v.set_suggestion_threshold("fake-64", Some(c + 1e-4))).unwrap();
    assert_eq!(suggested(&session, id), ["aa", "bb"]);
    session.with_vault(|v| v.set_suggestion_threshold("fake-64", Some(0.3))).unwrap();

    // A rejection removes the suggestion for good, also after lock and unlock.
    session.with_vault(|v| v.reject_tag_suggestion(id, "aa")).unwrap();
    assert_eq!(suggested(&session, id), ["bb", "cc", "dd"]);
    session.lock();
    let g = session.unlock(Vault::open(&path, common::PASS).unwrap(), path).unwrap();
    index(&session, g, &source);
    assert_eq!(suggested(&session, id), ["bb", "cc", "dd"]);
    assert_eq!(feedback(&session, id), [("aa".to_string(), false)]);

    // No vector for the note yet: nothing to suggest.
    let fresh = session.with_vault(|v| v.create_note("", "Fresh", BODY)).unwrap();
    assert!(suggested(&session, fresh).is_empty());
}

#[test]
fn accepting_writes_frontmatter_like_the_metadata_panel_and_the_tag_filter_finds_the_note() {
    let fake = Fake::new(64);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    let body = format!("---\r\nauthor: Ada\r\ntag: old\r\n---\r\n{BODY}\r\n");
    let id = session.with_vault(|v| v.create_note("", "Night", &body)).unwrap();
    describe(&session, "dream", "dream sleep night");
    index(&session, g, &source);
    assert_eq!(suggested(&session, id), ["dream"]);

    session.with_vault(|v| v.accept_tag_suggestion(id, "dream")).unwrap();
    // The same frontmatter edit as adding a tag elsewhere: `tag` folded into `tags`, CRLF kept.
    let expected = markdown::set_frontmatter_field(
        &markdown::set_frontmatter_field(&body, "tag", &[]),
        "tags",
        &["old".to_string(), "dream".to_string()],
    );
    let note = session.with_vault(|v| v.note(id)).unwrap();
    assert_eq!(note.body, expected);
    assert!(note.body.contains("tags: [old, dream]\r\n") && !note.body.contains("\ntag:"), "{:?}", note.body);
    assert_eq!(note.tags, ["dream", "old"]);
    let q = SearchQuery { tags: vec!["dream".into()], ..Default::default() };
    let found: Vec<i64> = session.with_vault(|v| v.search(&q)).unwrap().into_iter().map(|h| h.note.id).collect();
    assert_eq!(found, [id]);
    assert_eq!(feedback(&session, id), [("dream".to_string(), true)]);
    assert!(suggested(&session, id).is_empty(), "the note has it now");

    // Accepting again changes nothing; a note with unreadable frontmatter is left alone.
    session.with_vault(|v| v.accept_tag_suggestion(id, "dream")).unwrap();
    assert_eq!(session.with_vault(|v| v.note(id)).unwrap().body, expected);
    let broken = session.with_vault(|v| v.create_note("", "Broken", "---\nauthor: [Ada\n---\nText")).unwrap();
    assert!(session.with_vault(|v| v.accept_tag_suggestion(broken, "dream")).is_err());
    assert!(feedback(&session, broken).is_empty());
}

#[test]
fn removing_a_tag_by_hand_records_a_rejection() {
    let fake = Fake::new(64);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Night", BODY)).unwrap();
    describe(&session, "dream", "dream sleep night");
    index(&session, g, &source);
    session.with_vault(|v| v.accept_tag_suggestion(id, "dream")).unwrap();

    // The user takes the tag off again in the editor: the latest decision wins.
    session.with_vault(|v| v.update_body(id, &format!("---\ntags: []\n---\n{BODY}"))).unwrap();
    assert_eq!(feedback(&session, id), [("dream".to_string(), false)]);
    assert!(suggested(&session, id).is_empty(), "not suggested again");

    // Adding it back by hand clears the rejection; removing it with a bulk edit records it again.
    session.with_vault(|v| v.update_body(id, &format!("---\ntags: [dream, misc]\n---\n{BODY}"))).unwrap();
    assert!(feedback(&session, id).is_empty());
    let remove = hush_lib::db::MetadataEdit::RemoveTag("dream".into());
    session.with_vault(|v| v.edit_notes_metadata(&[id], &remove).map(|_| ())).unwrap();
    assert_eq!(feedback(&session, id), [("dream".to_string(), false)]);

    // A tag that can't be suggested (no description, no feedback) leaves no row:
    // half-typed tags aren't decisions.
    session.with_vault(|v| v.update_body(id, BODY)).unwrap();
    assert_eq!(feedback(&session, id), [("dream".to_string(), false)]);
}

#[test]
fn editing_a_description_re_embeds_it() {
    let fake = Fake::new(64);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    session.with_vault(|v| v.create_note("", "Night", BODY)).unwrap();
    describe(&session, "dream", "dream sleep");
    index(&session, g, &source);
    let (model, first) = session.with_vault(|v| v.tag_description_vector("dream")).unwrap().unwrap();
    assert_eq!(model, "fake-64");
    assert_eq!(first, fake.vector("dream sleep"));

    // The same text again keeps the vector.
    let calls = fake.calls();
    describe(&session, "dream", "  dream sleep ");
    index(&session, g, &source);
    assert_eq!(fake.calls(), calls);

    describe(&session, "dream", "nightmares and waking up");
    assert_eq!(session.with_vault(|v| v.tag_description_vector("dream")).unwrap(), None);
    index(&session, g, &source);
    let (_, second) = session.with_vault(|v| v.tag_description_vector("dream")).unwrap().unwrap();
    assert_eq!(second, fake.vector("nightmares and waking up"));
    assert_eq!(fake.calls(), calls + 1);

    // An empty description removes it.
    describe(&session, "dream", " ");
    assert_eq!(session.with_vault(|v| v.tag_description_vector("dream")).unwrap(), None);
    assert!(session.with_vault(|v| v.tag_list()).unwrap().iter().all(|t| t.tag != "dream"));
}

/// The running job picks up a description edit like any other change.
#[test]
fn the_running_job_embeds_a_new_description() {
    let fake = Fake::new(64);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Night", BODY)).unwrap();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, Duration::from_millis(10), |_| {}));
        describe(&session, "dream", "dream sleep night");
        let start = std::time::Instant::now();
        while suggested(&session, id).is_empty() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        session.lock();
    });
}

#[test]
fn switching_the_ready_model_re_embeds_descriptions_and_uses_its_threshold() {
    let four = Fake::new(4);
    let eight = Fake::new(8);
    let (_dir, _path, session, g) = session("fake-4", 0.0);
    let source = Fakes(vec![four.clone(), eight.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Night", BODY)).unwrap();
    describe(&session, "dream", "dream sleep night");
    index(&session, g, &source);
    assert_eq!(suggested(&session, id), ["dream"]);

    // The new model's threshold is set too high for any suggestion.
    session.with_vault(|v| v.set_suggestion_threshold("fake-8", Some(1.0))).unwrap();
    session.with_vault(|v| v.set_embedding_model("fake-8")).unwrap();
    // Mid-switch (a pass that drains the queue completes it): the old model's
    // description and threshold still serve.
    let (model, _) = session.with_vault(|v| v.tag_description_vector("dream")).unwrap().unwrap();
    assert_eq!(model, "fake-4");
    assert_eq!(suggested(&session, id), ["dream"]);

    index(&session, g, &source);
    assert_eq!(session.with_vault(|v| v.embedding_model_ready()).unwrap().as_deref(), Some("fake-8"));
    let (model, vector) = session.with_vault(|v| v.tag_description_vector("dream")).unwrap().unwrap();
    assert_eq!((model.as_str(), vector.len()), ("fake-8", 8));
    assert!(suggested(&session, id).is_empty(), "fake-8's threshold applies");
    session.with_vault(|v| v.set_suggestion_threshold("fake-8", Some(0.0))).unwrap();
    assert_eq!(suggested(&session, id), ["dream"]);
    // Back to the default: fake-8 has none, so nothing is suggested.
    session.with_vault(|v| v.set_suggestion_threshold("fake-8", None)).unwrap();
    assert_eq!(session.with_vault(|v| v.suggestion_threshold("fake-8")).unwrap(), None);
    assert!(suggested(&session, id).is_empty());
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    assert_eq!(session.with_vault(|v| v.suggestion_threshold(d.id)).unwrap(), Some(d.suggestion_threshold));
    assert!(session.with_vault(|v| v.set_suggestion_threshold("fake-8", Some(1.5))).is_err());
}

#[test]
fn case_sensitive_tags_are_kept_apart() {
    let fake = Fake::new(64);
    let (_dir, _path, session, g) = session("fake-64", 0.3);
    let source = Fakes(vec![fake.clone()]);
    let id = session.with_vault(|v| v.create_note("", "Night", &format!("---\ntags: [jr]\n---\n{BODY}"))).unwrap();
    describe(&session, "jr", "dream sleep night");
    describe(&session, "Jr", "dream sleep night moon");
    describe(&session, "dr", "dream sleep");
    index(&session, g, &source);
    assert_eq!(suggested(&session, id), ["Jr", "dr"], "`jr` on the note doesn't hide `Jr`");

    session.with_vault(|v| v.reject_tag_suggestion(id, "Jr")).unwrap();
    session.with_vault(|v| v.accept_tag_suggestion(id, "dr")).unwrap();
    assert_eq!(feedback(&session, id), [("Jr".to_string(), false), ("dr".to_string(), true)]);
    let note = session.with_vault(|v| v.note(id)).unwrap();
    assert!(note.body.starts_with("---\ntags: [jr, dr]\n---\n"), "{:?}", note.body);

    // Accepting `Jr` on a note with `jr` adds it next to it.
    session.with_vault(|v| v.accept_tag_suggestion(id, "Jr")).unwrap();
    assert_eq!(session.with_vault(|v| v.note(id)).unwrap().tags, ["dr", "Jr", "jr"]);

    let tags = session.with_vault(|v| v.tag_list()).unwrap();
    let info = |t: &str| tags.iter().find(|i| i.tag == t).cloned();
    assert_eq!(info("jr"), Some(TagInfo { tag: "jr".into(), notes: 1, description: "dream sleep night".into() }));
    assert_eq!(info("Jr"), Some(TagInfo { tag: "Jr".into(), notes: 1, description: "dream sleep night moon".into() }));
}

#[test]
fn tags_can_be_described_before_any_note_uses_them() {
    let (_dir, _path, session, _g) = session("fake-64", 0.3);
    session.with_vault(|v| v.create_note("", "A", "---\ntags: [used]\n---\nx")).unwrap();
    describe(&session, "#new", "something new");
    assert!(session.with_vault(|v| v.set_tag_description("two words", "x")).is_err());
    let tags = session.with_vault(|v| v.tag_list()).unwrap();
    assert_eq!(
        tags,
        [
            TagInfo { tag: "new".into(), notes: 0, description: "something new".into() },
            TagInfo { tag: "used".into(), notes: 1, description: String::new() },
        ]
    );
}

#[test]
fn feedback_is_kept_in_the_trash_hidden_and_deleted_with_the_note() {
    let (_dir, _path, session, _g) = session("fake-64", 0.3);
    let id = session.with_vault(|v| v.create_note("", "A", "text")).unwrap();
    session.with_vault(|v| v.reject_tag_suggestion(id, "xx")).unwrap();
    session.with_vault(|v| v.trash_note(id)).unwrap();
    assert!(feedback(&session, id).is_empty(), "hidden while trashed");
    assert!(session.with_vault(|v| v.reject_tag_suggestion(id, "yy")).is_err());
    session.with_vault(|v| v.restore_note(id).map(|_| ())).unwrap();
    assert_eq!(feedback(&session, id), [("xx".to_string(), false)], "kept through the trash");

    session.with_vault(|v| v.delete_note(id)).unwrap();
    let other = session.with_vault(|v| v.create_note("", "B", "text")).unwrap();
    assert!(feedback(&session, other).is_empty());
}

/// Measures how well tag descriptions find the right fixture notes with the real model,
/// and prints the threshold that separates them best. Run it with the bundled model
/// (`npm run fetch-model` first):
///
/// `cargo test --manifest-path src-tauri/Cargo.toml --test tag_suggestions -- --ignored --nocapture`
///
/// `HUSH_MEASURE_ONNX=<file>` measures another ONNX export of the same model instead
/// (its checksum isn't checked; the tokenizer still is).
#[test]
#[ignore = "needs the real model; prints measurements"]
fn measure_suggestion_threshold_on_the_fixture_vault() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let base = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    let mut descriptor: ModelDescriptor = *base;
    if let Ok(file) = std::env::var("HUSH_MEASURE_ONNX") {
        let bytes = std::fs::read(&file).unwrap();
        let sha: String = sha256(&bytes);
        let path = Path::new(&file);
        assert_eq!(path.parent().unwrap().canonicalize().unwrap(), dir.canonicalize().unwrap(), "put it in {}", dir.display());
        descriptor.model.name = Box::leak(path.file_name().unwrap().to_string_lossy().into_owned().into_boxed_str());
        descriptor.model.sha256 = Box::leak(sha.into_boxed_str());
    }
    if !dir.join(descriptor.model.name).is_file() || !dir.join(descriptor.tokenizer.name).is_file() {
        panic!("model files missing in {}; run `npm run fetch-model`", dir.display());
    }
    let descriptor: &'static ModelDescriptor = Box::leak(Box::new(descriptor));
    let model = OnnxEmbedder::load(&dir, descriptor).unwrap();

    // The fixture vault imported with the default rules, as a user would.
    let (vault_dir, vault) = common::vault();
    let session = Session::default();
    session.unlock(vault, vault_dir.path().join("vault.hush")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
    import::run(&session, &fixture, "", false, &AtomicBool::new(false), |_| {}).unwrap();
    let notes: Vec<(String, String)> = session
        .with_vault(|v| {
            let mut out = Vec::new();
            for folder in v.folders()? {
                for n in v.folder_notes(&folder, 0, 1000)? {
                    out.push((n.title.clone(), v.note(n.id)?.body));
                }
            }
            Ok(out)
        })
        .unwrap();

    // Hand-labelled: which notes each description should find (by title).
    let labels: &[(&str, &str, &[&str])] = &[
        ("dream", "dreams, nightmares, sleep", &["2004-03-14 Flying", "Dream of the sea"]),
        ("journal", "diary entries about my day", &["2004-03-12 Morning walk", "2004-04-01 Rain", "2005-01-02 New year"]),
        ("ferry", "ferry timetables and boat crossings", &["thread-0001", "thread-0002", "ferry"]),
        ("letter", "letters to and from family", &["letter to Jan", "letter to Jan (txt)", "Brief (concept) 2"]),
        ("work", "work meetings and budgets", &["meeting 20210202"]),
    ];
    let titles: Vec<&str> = notes.iter().map(|(t, _)| t.as_str()).collect();
    for (_, _, positives) in labels {
        for p in *positives {
            assert!(titles.contains(p), "no fixture note titled {p:?}; titles: {titles:?}");
        }
    }
    let vectors: Vec<(&str, Vec<f32>)> = notes
        .iter()
        .filter_map(|(title, body)| Some((title.as_str(), model.embed(&embed::note_text(title, body)?).ok()?)))
        .collect();

    // The first is what Hush does; the others are the alternatives it was chosen from.
    const BGE_QUERY: &str = "Represent this sentence for searching relevant passages: ";
    let variants: [(&str, Box<dyn Fn(&str, &str) -> Vec<f32>>); 5] = [
        ("as Hush embeds it (Embedder::embed_query)", Box::new(|_, d| model.embed_query(d).unwrap())),
        ("description", Box::new(|_, d| model.embed(d).unwrap())),
        ("description, bge query prefix", Box::new(|_, d| model.embed(&format!("{BGE_QUERY}{d}")).unwrap())),
        ("tag: description", Box::new(|t, d| model.embed(&format!("{t}: {d}")).unwrap())),
        ("tag: description, bge query prefix", Box::new(|t, d| model.embed(&format!("{BGE_QUERY}{t}: {d}")).unwrap())),
    ];
    for (i, (name, embed_description)) in variants.iter().enumerate() {
        let mut scored: Vec<(f32, bool)> = Vec::new();
        println!("\n== {name}");
        for (tag, description, positives) in labels {
            let q = embed_description(tag, description);
            let mut row: Vec<(f32, &str, bool)> = vectors
                .iter()
                .map(|(title, v)| (embed::cosine(v, &q), *title, positives.contains(title)))
                .collect();
            row.sort_by(|a, b| b.0.total_cmp(&a.0));
            let shown: Vec<String> =
                row.iter().take(6).map(|(s, t, p)| format!("{s:.3}{} {t}", if *p { "*" } else { "" })).collect();
            println!("{tag:8} {}", shown.join(" | "));
            scored.extend(row.iter().map(|(s, _, p)| (*s, *p)));
        }
        if i == 0 {
            let (f1, precision, recall) = rates(&scored, descriptor.suggestion_threshold);
            println!(
                "at the current default {:.3}: F1 {f1:.2}, precision {precision:.2}, recall {recall:.2}",
                descriptor.suggestion_threshold
            );
        }
        let (best, f1, precision, recall) = best_threshold(&scored);
        let pos: Vec<f32> = scored.iter().filter(|s| s.1).map(|s| s.0).collect();
        let neg: Vec<f32> = scored.iter().filter(|s| !s.1).map(|s| s.0).collect();
        let max = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
        let min = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
        println!(
            "positives {:.3}..{:.3}, negatives {:.3}..{:.3}; best threshold {best:.3}: F1 {f1:.2}, precision {precision:.2}, recall {recall:.2}",
            min(&pos),
            max(&pos),
            min(&neg),
            max(&neg)
        );
    }
}

/// The threshold with the best F1 over `(score, is positive)`, preferring the higher
/// threshold on a tie (fewer wrong suggestions). Returns it with F1, precision, recall.
fn best_threshold(scored: &[(f32, bool)]) -> (f32, f32, f32, f32) {
    let mut best = (1.0, 0.0, 0.0, 0.0);
    for &(t, _) in scored {
        let (f1, p, r) = rates(scored, t);
        if f1 > best.1 || (f1 == best.1 && t > best.0) {
            best = (t, f1, p, r);
        }
    }
    best
}

/// F1, precision and recall of suggesting every score at or above `threshold`.
fn rates(scored: &[(f32, bool)], threshold: f32) -> (f32, f32, f32) {
    let positives = scored.iter().filter(|s| s.1).count() as f32;
    let hits = scored.iter().filter(|s| s.0 >= threshold).count() as f32;
    let right = scored.iter().filter(|s| s.0 >= threshold && s.1).count() as f32;
    let p = if hits > 0.0 { right / hits } else { 0.0 };
    let r = right / positives;
    let f1 = if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 };
    (f1, p, r)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
