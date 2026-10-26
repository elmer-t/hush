mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hush_lib::db::{NewNote, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::related;
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::session::Session;
use tempfile::TempDir;

/// An embedder with controlled vectors: a note whose body is `v 1 0 0 0` gets exactly
/// that vector (normalised). Any other text gets a fixed vector.
struct Controlled {
    id: String,
    dims: usize,
}

impl Embedder for Controlled {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0; self.dims];
        v[self.dims - 1] = 1.0;
        if let Some(spec) = text.lines().find_map(|l| l.strip_prefix("v ")) {
            for (slot, x) in v.iter_mut().zip(spec.split_whitespace()) {
                *slot = x.parse().unwrap();
            }
        }
        embed::normalize(&mut v);
        Ok(v)
    }
}

struct Source(Vec<Arc<Controlled>>);

impl EmbedderSource for Source {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let e = self.0.iter().find(|e| e.id == model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(e.clone())
    }
}

fn source(dims: &[usize]) -> Source {
    Source(dims.iter().map(|&dims| Arc::new(Controlled { id: format!("ctl-{dims}"), dims })).collect())
}

fn session(model: &str) -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(model).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

/// Runs passes until one finds nothing to do, as the job does after every change.
fn index(session: &Session, generation: u64, source: &dyn EmbedderSource) {
    for _ in 0..10 {
        if job::pass(session, generation, source, &mut |_| {}).unwrap().stale == 0 {
            return;
        }
    }
    panic!("indexing never finished");
}

fn note(session: &Session, title: &str, vector: &str) -> i64 {
    session.with_vault(|v| v.create_note("", title, &format!("v {vector}"))).unwrap()
}

/// Titles of the notes related to `id`, or `None` if it isn't indexed.
fn related(session: &Session, id: i64, limit: usize) -> Option<Vec<String>> {
    related::related_notes(session, id, limit).unwrap().map(|notes| notes.into_iter().map(|n| n.note.title).collect())
}

fn indexed(session: &Session) -> usize {
    session.related_index().as_ref().map_or(0, |i| i.len())
}

#[test]
fn related_notes_are_ordered_by_similarity_and_skip_self_and_unindexed_notes() {
    let (_dir, _path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = note(&session, "A", "1 0 0 0");
    note(&session, "Near", "0.9 0.1 0 0");
    note(&session, "Mid", "0.5 0.5 0 0");
    note(&session, "Far", "0 1 0 0");
    note(&session, "Opposite", "-1 0 0 0");
    let empty = session.with_vault(|v| v.create_note("", "Empty", "---\ntags: [x]\n---\n")).unwrap();

    // Nothing is ready before the first pass.
    assert_eq!(related(&session, a, 10), None);
    index(&session, g, &source);

    assert_eq!(related(&session, a, 10).unwrap(), ["Near", "Mid", "Far", "Opposite"]);
    assert_eq!(related(&session, a, 2).unwrap(), ["Near", "Mid"], "limit honoured");
    assert_eq!(related(&session, a, 0).unwrap(), Vec::<String>::new());
    assert_eq!(related(&session, empty, 10), None, "a note without a vector isn't indexed yet");
    assert_eq!(indexed(&session), 5);

    // A note added later isn't related to anything until the job has embedded it.
    let late = note(&session, "Late", "1 0 0 0");
    assert_eq!(related(&session, late, 10), None);
    assert_eq!(related(&session, a, 1).unwrap(), ["Near"]);
    index(&session, g, &source);
    assert_eq!(related(&session, a, 1).unwrap(), ["Late"]);
}

#[test]
fn related_notes_carry_the_exact_score_and_reasons() {
    let (_dir, _path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = session.with_vault(|v| v.create_note("f", "A", "v 1 0 0 0\n#shared [[Near]]")).unwrap();
    session.with_vault(|v| v.create_note("f", "Near", "v 0.6 0.8 0 0\n#shared")).unwrap();
    note(&session, "Far", "0 1 0 0");
    index(&session, g, &source);

    let notes = related::related_notes(&session, a, 10).unwrap().unwrap();
    assert_eq!(notes[0].note.title, "Near");
    assert!((notes[0].score - 0.6).abs() < 1e-6, "exact cosine, not the i8 estimate: {}", notes[0].score);
    assert!(notes[0].reasons.links_to && notes[0].reasons.same_folder);
    assert_eq!(notes[0].reasons.shared_tags, ["shared"]);
    assert!(notes[1].score.abs() < 1e-6);
    assert!(!notes[1].reasons.same_folder);
}

#[test]
fn the_index_follows_saves_trash_restore_and_delete() {
    let (_dir, _path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = note(&session, "A", "1 0 0 0");
    let b = note(&session, "B", "0.9 0.1 0 0");
    let c = note(&session, "C", "0 1 0 0");
    index(&session, g, &source);
    assert_eq!(related(&session, a, 10).unwrap(), ["B", "C"]);

    // Save: C is re-embedded closer to A than B.
    session.with_vault(|v| v.update_body(c, "v 1 0.01 0 0")).unwrap();
    assert!(job::pass(&session, g, &source, &mut |_| {}).unwrap().related_changed);
    assert_eq!(related(&session, a, 10).unwrap(), ["C", "B"]);

    // Trash: gone from the results at once, and from the index after the next pass.
    session.with_vault(|v| v.trash_note(c)).unwrap();
    assert_eq!(related(&session, a, 1).unwrap(), ["B"], "trashed notes are skipped before the job runs");
    assert_eq!(indexed(&session), 3);
    index(&session, g, &source);
    assert_eq!(indexed(&session), 2);
    assert!(!session.related_index().as_ref().unwrap().contains(c));
    assert_eq!(related(&session, c, 10), None);

    // Restore: re-embedded and back.
    session.with_vault(|v| v.restore_note(c).map(|_| ())).unwrap();
    index(&session, g, &source);
    assert_eq!(related(&session, a, 10).unwrap(), ["C", "B"]);

    // Delete forever, from the trash and of a live note.
    session.with_vault(|v| v.trash_note(c)).unwrap();
    session.with_vault(|v| v.empty_trash()).unwrap();
    session.with_vault(|v| v.delete_note(b)).unwrap();
    assert_eq!(related(&session, a, 10).unwrap(), Vec::<String>::new());
    index(&session, g, &source);
    assert_eq!(indexed(&session), 1);

    // Emptying a note drops its vector.
    let d = note(&session, "D", "1 0 0 0");
    index(&session, g, &source);
    assert_eq!(related(&session, a, 10).unwrap(), ["D"]);
    session.with_vault(|v| v.update_body(d, "")).unwrap();
    index(&session, g, &source);
    assert_eq!(related(&session, a, 10).unwrap(), Vec::<String>::new());
    assert_eq!(related(&session, d, 10), None);

    // A pass with nothing new leaves the index alone.
    assert!(!job::pass(&session, g, &source, &mut |_| {}).unwrap().related_changed);
}

#[test]
fn job_passes_update_only_the_notes_they_changed() {
    let (_dir, path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = note(&session, "A", "1 0 0 0");
    index(&session, g, &source);
    // A vector written behind the job's back, with the hash of the note's current text
    // (so nothing will re-embed it), and the queue row its creation made dropped on a
    // second connection: only a full comparison of the vault could find it.
    let hidden = note(&session, "Hidden", "0.9 0.1 0 0");
    session
        .with_vault(|v| {
            let text = v.embedding_texts(&[hidden])?.pop().unwrap();
            v.write_embeddings("ctl-4", &[(hidden, text.hash, vec![0.9, 0.1, 0.0, 0.0])])
        })
        .unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "key", common::PASS).unwrap();
    raw.execute("DELETE FROM embedding_dirty", []).unwrap();
    drop(raw);
    let b = note(&session, "B", "0.5 0.5 0 0");
    index(&session, g, &source);
    let contains = |id| session.related_index().as_ref().unwrap().contains(id);
    assert!(contains(b), "the note the pass embedded is loaded");
    assert!(!contains(hidden), "the pass handed the index only its own notes, no full diff");
    assert_eq!(related(&session, a, 10).unwrap(), ["B"]);
    // A full sync does find it.
    assert!(related::sync(&session, g).unwrap());
    assert_eq!(related(&session, a, 10).unwrap(), ["Hidden", "B"]);
}

#[test]
fn the_index_is_empty_after_lock_and_rebuilt_after_unlock() {
    let (_dir, path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = note(&session, "A", "1 0 0 0");
    note(&session, "B", "0 1 0 0");
    index(&session, g, &source);
    assert_eq!(indexed(&session), 2);

    session.lock();
    assert!(session.related_index().is_none(), "dropped on lock");
    // A job of the old unlock can't put it back.
    assert!(session.update_related_index(g, |_| ()).is_none());
    assert!(related::related_notes(&session, a, 10).is_err());

    let g2 = session.unlock(Vault::open(&path, common::PASS).unwrap(), path.clone()).unwrap();
    assert!(session.related_index().is_none());
    assert!(related::sync(&session, g).is_err(), "the old generation is refused");
    index(&session, g2, &source);
    assert_eq!(related(&session, a, 10).unwrap(), ["B"]);
}

#[test]
fn the_index_is_rebuilt_when_the_ready_model_changes() {
    let (_dir, path, session, g) = session("ctl-4");
    let source = source(&[4, 8]);
    let a = note(&session, "A", "1 0 0 0 0 0 0 0");
    note(&session, "B", "0 1 0 0 0 0 0 0");
    note(&session, "C", "0 0 0 0 1 0 0 0");
    index(&session, g, &source);
    let model = |s: &Session| s.related_index().as_ref().and_then(|i| i.model().map(str::to_string));
    assert_eq!(model(&session).as_deref(), Some("ctl-4"));
    // In 4 dims C's vector is all zeros, so B and C tie at 0.
    assert_eq!(related(&session, a, 10).unwrap().len(), 2);

    // Switch: the old vectors keep serving until the new model is complete.
    // (A pass that drains the queue completes the new model, so check before it.)
    session.with_vault(|v| v.set_embedding_model("ctl-8")).unwrap();
    related::sync(&session, g).unwrap();
    assert_eq!(model(&session).as_deref(), Some("ctl-4"));
    index(&session, g, &source);
    assert_eq!(model(&session).as_deref(), Some("ctl-8"));
    assert_eq!(indexed(&session), 3);

    // Lock and unlock: rebuilt for the ready model.
    session.lock();
    let g2 = session.unlock(Vault::open(&path, common::PASS).unwrap(), path).unwrap();
    index(&session, g2, &source);
    assert_eq!(model(&session).as_deref(), Some("ctl-8"));
    assert_eq!(related(&session, a, 10).unwrap().len(), 2);
}

#[test]
fn the_running_job_reports_index_changes() {
    let (_dir, _path, session, g) = session("ctl-4");
    let source = source(&[4]);
    let a = note(&session, "A", "1 0 0 0");
    let changes = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        s.spawn(|| {
            job::run_with(&session, g, &source, Duration::from_millis(5), |_| {}, || {
                changes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        });
        let wait = |f: &dyn Fn() -> bool| {
            let start = Instant::now();
            while !f() {
                assert!(start.elapsed() < Duration::from_secs(10), "timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(&|| session.related_index().as_ref().is_some_and(|i| i.contains(a)));
        let before = changes.load(std::sync::atomic::Ordering::SeqCst);
        assert!(before >= 1);
        note(&session, "B", "1 0 0 0");
        wait(&|| changes.load(std::sync::atomic::Ordering::SeqCst) > before);
        assert_eq!(related(&session, a, 10).unwrap(), ["B"]);
        session.lock();
    });
}

/// `related_notes` on a 10k-note vault with 384-dim vectors. Ignored by default (it is a
/// timing, and slow in a debug build); run with
/// `cargo test --release --test related -- --ignored --nocapture`.
#[test]
#[ignore]
fn related_notes_on_10k_notes_is_fast() {
    const N: usize = 10_000;
    const DIMS: usize = 384;
    let (_dir, _path, session, g) = session("ctl-384");
    // Tags and links, so the reasons for the shown notes have something to find.
    let bodies: Vec<(String, String)> = (0..N)
        .map(|i| (format!("Note {i}"), format!("body {i} #t{} [[Note {}]] [[Note {}]]", i % 50, (i * 7) % N, (i + 1) % N)))
        .collect();
    let notes: Vec<NewNote> = bodies
        .iter()
        .map(|(t, b)| NewNote { folder: "f", title: t, body: b, created_at: 0, updated_at: 0 })
        .collect();
    let ids: Vec<i64> =
        session.with_vault(|v| v.import_notes(&notes)).unwrap().into_iter().map(|r| r.unwrap()).collect();
    // Pseudo-random vectors, written directly: embedding 10k notes isn't what's measured.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let vectors: Vec<(i64, [u8; 32], Vec<f32>)> = ids
        .iter()
        .map(|&id| {
            let mut v: Vec<f32> = (0..DIMS)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed % 2000) as f32 / 1000.0 - 1.0
                })
                .collect();
            embed::normalize(&mut v);
            (id, [0; 32], v)
        })
        .collect();
    session
        .with_vault(|v| {
            v.write_embeddings("ctl-384", &vectors)?;
            v.finish_embedding_model("ctl-384")
        })
        .unwrap();

    let start = Instant::now();
    assert!(related::sync(&session, g).unwrap());
    let load = start.elapsed();
    assert_eq!(indexed(&session), N);

    let mut times = Vec::new();
    for &id in ids.iter().step_by(N / 50) {
        let start = Instant::now();
        let notes = related::related_notes(&session, id, 10).unwrap().unwrap();
        times.push(start.elapsed());
        assert_eq!(notes.len(), 10);
    }
    times.sort();
    let median = times[times.len() / 2];
    let worst = *times.last().unwrap();
    println!("10k x {DIMS}: index load {load:?}, related_notes median {median:?}, worst {worst:?}");
    assert!(worst < Duration::from_millis(50), "related_notes took {worst:?}");
}
