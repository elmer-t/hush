//! The Suggestions view: one tag's suggestions across the vault in both orders, its
//! exclusions, bulk accept and reject, and retraining after decisions.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use hush_lib::classify::{self, job as training, review};
use hush_lib::db::{ReviewOrder, ScoreMethod, SuggestionReview, Vault, new_stored_head};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::session::Session;
use tempfile::TempDir;

/// Embeds a text containing a line `v 0.1 0.2 0.3` as that vector (normalised).
struct Controlled;

const MODEL: &str = "ctl-3";

impl Embedder for Controlled {
    fn model_id(&self) -> &str {
        MODEL
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0, 0.0, 1.0];
        if let Some(spec) = text.lines().find_map(|l| l.strip_prefix("v ")) {
            for (slot, x) in v.iter_mut().zip(spec.split_whitespace()) {
                *slot = x.parse().unwrap();
            }
        }
        embed::normalize(&mut v);
        Ok(v)
    }
}

struct Source(Arc<Controlled>);

impl EmbedderSource for Source {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        if model == MODEL { Ok(self.0.clone()) } else { Err(EmbedError::UnknownModel(model.into())) }
    }
}

fn session() -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(MODEL).unwrap();
    vault.set_suggestion_threshold(MODEL, Some(0.5)).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

fn index(session: &Session, generation: u64) {
    let source = Source(Arc::new(Controlled));
    for _ in 0..10 {
        if job::pass(session, generation, &source, &mut |_| {}).unwrap().stale == 0 {
            return;
        }
    }
    panic!("indexing never finished");
}

/// A note whose unit vector has first component `x` (so its cosine with `v 1 0 0` is `x`).
fn note(session: &Session, title: &str, x: f32, extra: &str) -> i64 {
    let y = (1.0 - x * x).max(0.0).sqrt();
    session.with_vault(|v| v.create_note("", title, &format!("{extra}v {x} {y} 0\nAbout {title}."))).unwrap()
}

/// Stores a head for `tag` with `probability = sigmoid(10·x − 5)`: 0.5 at x = 0.5.
fn head(session: &Session, tag: &str) {
    let stored = new_stored_head(
        tag,
        MODEL,
        classify::Head { weights: vec![10.0, 0.0, 0.0], bias: -5.0 },
        (10, 10),
        classify::Quality { precision: None, recall: None },
        [0; 32],
    );
    assert!(session.with_vault(|v| v.write_tag_head(&stored)).unwrap());
}

fn titles(r: &SuggestionReview) -> Vec<&str> {
    r.notes.iter().map(|n| n.title.as_str()).collect()
}

fn body(session: &Session, id: i64) -> String {
    session.with_vault(|v| v.note(id)).unwrap().body
}

#[test]
fn both_orders_are_sorted_and_exclusions_apply() {
    let (_dir, _path, session, g) = session();
    note(&session, "A", 0.9, "");
    note(&session, "B", 0.55, "");
    note(&session, "C", 0.5, "");
    note(&session, "D", 0.42, "");
    note(&session, "below floor", 0.3, "");
    note(&session, "tagged", 0.95, "---\ntags: [dream]\n---\n");
    note(&session, "inline", 0.93, "#dream\n");
    let other_case = note(&session, "other case", 0.92, "---\ntags: [Dream]\n---\n");
    let rejected = note(&session, "rejected", 0.85, "");
    let trashed = note(&session, "trashed", 0.8, "");
    index(&session, g);
    head(&session, "dream");
    session.with_vault(|v| v.reject_tag_suggestion(rejected, "dream")).unwrap();
    session.with_vault(|v| v.trash_note(trashed)).unwrap();

    for from_index in [true, false] {
        let get = |order| {
            if from_index {
                review::review(&session, "dream", order, 100).unwrap()
            } else {
                session.with_vault(|v| v.suggestion_review("dream", order, 100)).unwrap()
            }
        };
        let likely = get(ReviewOrder::MostLikely);
        assert_eq!(likely.method, Some(ScoreMethod::Classifier));
        assert_eq!(likely.min_score, 0.2);
        // Tags are case-sensitive: `Dream` doesn't exclude a note from `dream`.
        assert_eq!(titles(&likely), ["other case", "A", "B", "C", "D"], "from index: {from_index}");
        assert_eq!(likely.total, 5);
        assert!(likely.notes.windows(2).all(|w| w[0].score >= w[1].score));
        let review = get(ReviewOrder::NeedsReview);
        assert_eq!(titles(&review), ["C", "B", "D", "A", "other case"], "from index: {from_index}");
        assert!((review.notes[0].score - 0.5).abs() < 1e-4);
    }

    // The first `limit`, with the total; excerpts are the body without frontmatter.
    let first = review::review(&session, "dream", ReviewOrder::MostLikely, 2).unwrap();
    assert_eq!((titles(&first), first.total), (vec!["other case", "A"], 5));
    assert_eq!(first.notes[0].id, other_case);
    assert!(first.notes[0].excerpt.starts_with("v 0.92 "), "{:?}", first.notes[0].excerpt);
    assert!(first.notes[0].excerpt.ends_with("About other case."));

    // A tag that can't be scored has no method and no notes.
    let none = review::review(&session, "nothing", ReviewOrder::NeedsReview, 10).unwrap();
    assert_eq!((none.method, none.total, none.notes.len()), (None, 0, 0));
}

#[test]
fn a_description_only_tag_lists_from_its_threshold_and_doubts_around_it() {
    let (_dir, _path, session, g) = session();
    note(&session, "near", 0.52, "");
    note(&session, "far", 0.95, "");
    note(&session, "mid", 0.7, "");
    note(&session, "below", 0.45, "");
    session.with_vault(|v| v.set_tag_description("ferry", "v 1 0 0")).unwrap();
    index(&session, g);

    let likely = review::review(&session, "ferry", ReviewOrder::MostLikely, 10).unwrap();
    assert_eq!(likely.method, Some(ScoreMethod::Description));
    assert_eq!(likely.min_score, 0.5);
    assert_eq!(titles(&likely), ["far", "mid", "near"]);
    let review = review::review(&session, "ferry", ReviewOrder::NeedsReview, 10).unwrap();
    assert_eq!(titles(&review), ["near", "mid", "far"]);
}

#[test]
fn bulk_accept_writes_frontmatter_and_feedback_in_one_transaction() {
    let (_dir, _path, session, g) = session();
    let a = note(&session, "A", 0.9, "");
    let b = note(&session, "B", 0.8, "---\r\nauthor: Ann\r\ntag: walk\r\n---\r\n");
    let broken = note(&session, "broken", 0.7, "---\n: [\n---\n");
    index(&session, g);
    head(&session, "dream");

    // All or nothing: an unknown id changes nothing.
    let before = body(&session, a);
    let broken_before = body(&session, broken);
    assert!(session.with_vault(|v| v.accept_tag_suggestions(&[a, 999_999], "dream")).is_err());
    assert_eq!(body(&session, a), before);
    assert!(session.with_vault(|v| v.tag_feedback(a)).unwrap().is_empty());

    let report = session.with_vault(|v| v.accept_tag_suggestions(&[a, b, broken], "#dream")).unwrap();
    assert_eq!(report.changed, [a, b]);
    assert_eq!(report.unreadable.iter().map(|n| n.id).collect::<Vec<_>>(), [broken]);
    assert!(body(&session, a).starts_with("---\ntags: [dream]\n---\n"), "{}", body(&session, a));
    // CRLF kept, the singular `tag` folded into `tags`.
    assert!(body(&session, b).contains("tags: [walk, dream]\r\n"), "{:?}", body(&session, b));
    for id in [a, b] {
        assert!(session.with_vault(|v| v.note_has_tag(id, "dream")).unwrap());
        let fb = session.with_vault(|v| v.tag_feedback(id)).unwrap();
        assert_eq!(fb.iter().map(|f| (f.tag.as_str(), f.accepted)).collect::<Vec<_>>(), [("dream", true)]);
    }
    assert!(session.with_vault(|v| v.tag_feedback(broken)).unwrap().is_empty());
    assert_eq!(body(&session, broken), broken_before);

    // Accepted notes leave the list; the broken one stays.
    let r = review::review(&session, "dream", ReviewOrder::MostLikely, 10).unwrap();
    assert_eq!(titles(&r), ["broken"]);

    // The reloaded body is what the editor saves next: the tag stays.
    let fresh = body(&session, a);
    session.with_vault(|v| v.update_body(a, &format!("{fresh}\nMore."))).unwrap();
    assert!(session.with_vault(|v| v.note_has_tag(a, "dream")).unwrap());
}

#[test]
fn bulk_reject_writes_only_feedback() {
    let (_dir, _path, session, g) = session();
    let a = note(&session, "A", 0.9, "");
    let b = note(&session, "B", 0.8, "");
    let trashed = note(&session, "trashed", 0.7, "");
    index(&session, g);
    head(&session, "dream");
    session.with_vault(|v| v.trash_note(trashed)).unwrap();
    let before: Vec<_> = [a, b].iter().map(|id| session.with_vault(|v| v.note(*id)).unwrap()).collect();

    // All or nothing: a trashed id rejects nothing.
    assert!(session.with_vault(|v| v.reject_tag_suggestions(&[a, trashed], "dream")).is_err());
    assert!(session.with_vault(|v| v.tag_feedback(a)).unwrap().is_empty());

    session.with_vault(|v| v.reject_tag_suggestions(&[a, b], "dream")).unwrap();
    for (id, note) in [a, b].iter().zip(&before) {
        let after = session.with_vault(|v| v.note(*id)).unwrap();
        assert_eq!((&after.body, after.updated_at), (&note.body, note.updated_at));
        let fb = session.with_vault(|v| v.tag_feedback(*id)).unwrap();
        assert_eq!(fb.iter().map(|f| (f.tag.as_str(), f.accepted)).collect::<Vec<_>>(), [("dream", false)]);
    }
    let r = review::review(&session, "dream", ReviewOrder::MostLikely, 10).unwrap();
    assert_eq!(r.total, 0);
}

#[test]
fn decisions_retrain_on_demand_and_the_list_follows() {
    let (_dir, _path, session, g) = session();
    let likely: Vec<i64> = (0..12).map(|i| note(&session, &format!("dream {i}"), 0.6 + 0.03 * i as f32, "")).collect();
    for i in 0..12 {
        note(&session, &format!("other {i}"), 0.05 * i as f32 / 12.0, "");
    }
    session.with_vault(|v| v.set_tag_description("dream", "v 1 0 0")).unwrap();
    index(&session, g);

    let r = review::review(&session, "dream", ReviewOrder::NeedsReview, 100).unwrap();
    assert_eq!((r.method, r.total), (Some(ScoreMethod::Description), 12));

    // Ten decisions, then a pass on demand: the tag gets a head.
    session.with_vault(|v| v.accept_tag_suggestions(&likely[..10], "dream")).unwrap();
    let pass = training::pass(&session, g).unwrap();
    assert_eq!(pass.trained, ["dream"]);
    let r = review::review(&session, "dream", ReviewOrder::MostLikely, 100).unwrap();
    assert_eq!(r.method, Some(ScoreMethod::Classifier));
    let mut ids: Vec<i64> = r.notes.iter().take(2).map(|n| n.id).collect();
    ids.sort();
    assert_eq!(ids, &likely[10..], "the two undecided dream notes come first: {:?}", titles(&r));
    assert!(r.notes[..2].iter().all(|n| n.score >= 0.5));

    // A rejection changes the head's inputs, so the next pass retrains it.
    session.with_vault(|v| v.reject_tag_suggestions(&likely[10..11], "dream")).unwrap();
    assert_eq!(training::pass(&session, g).unwrap().trained, ["dream"]);
}
