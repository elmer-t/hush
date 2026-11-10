//! Search by meaning (#146) with a fake embedder whose vectors are chosen by the test.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use hush_lib::db::{SearchMode, SearchPage, SearchQuery, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::meaning::{self, QueryModels, VECTOR_LIMIT};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A note whose body has a line `v 1 0 0 0` gets that vector (normalised); any other
/// text gets the last axis. A query gets the vector of the first concept word it
/// contains, so it can share no word with the notes it should find.
struct Concepts {
    id: String,
}

const CONCEPTS: &[(&str, [f32; 4])] = &[("stuck", [1.0, 0.0, 0.0, 0.0]), ("ocean", [0.0, 1.0, 0.0, 0.0])];

impl Embedder for Concepts {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0, 0.0, 0.0, 1.0];
        if let Some(spec) = text.lines().find_map(|l| l.strip_prefix("v ")) {
            for (slot, x) in v.iter_mut().zip(spec.split_whitespace()) {
                *slot = x.parse().unwrap();
            }
        }
        embed::normalize(&mut v);
        Ok(v)
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let found = CONCEPTS.iter().find(|(word, _)| text.split_whitespace().any(|w| w == *word));
        Ok(found.map_or(vec![0.0, 0.0, 0.0, 1.0], |(_, v)| v.to_vec()))
    }
}

struct Models(Vec<Arc<Concepts>>);

impl Models {
    fn new(ids: &[&str]) -> Self {
        Self(ids.iter().map(|id| Arc::new(Concepts { id: id.to_string() })).collect())
    }

    fn get(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let e = self.0.iter().find(|e| e.id == model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        Ok(e.clone())
    }
}

impl EmbedderSource for Models {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        self.get(model)
    }
}

impl QueryModels for Models {
    fn query_embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        self.get(model)
    }

    fn query_loaded(&self, _: &str) -> bool {
        true
    }
}

const MODEL: &str = "concepts-4";

fn session() -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(MODEL).unwrap();
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

fn note(session: &Session, folder: &str, title: &str, body: &str) -> i64 {
    session.with_vault(|v| v.create_note(folder, title, body)).unwrap()
}

fn query(text: &str, mode: SearchMode) -> SearchQuery {
    SearchQuery { text: text.into(), mode, ..Default::default() }
}

fn run(session: &Session, models: &Models, q: &SearchQuery) -> SearchPage {
    match q.mode {
        SearchMode::Meaning => meaning::search(session, models, q).unwrap().unwrap(),
        SearchMode::Words => session.with_vault(|v| v.search_page(q)).unwrap(),
    }
}

fn titles(page: &SearchPage) -> Vec<String> {
    page.hits.iter().map(|h| h.note.title.clone()).collect()
}

#[test]
fn a_note_sharing_no_words_is_found_by_meaning_only() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "journal", "Going nowhere", "Another week of going nowhere on the project.\nv 1 0 0 0");
    note(&session, "journal", "Beach day", "Swimming all afternoon.\nv 0 1 0 0");
    note(&session, "work", "Feeling stuck notes", "I am stuck on the parser.\nv 0 0 1 0");
    index(&session, generation, &models);

    let words = run(&session, &models, &query("feeling stuck", SearchMode::Words));
    assert_eq!(titles(&words), vec!["Feeling stuck notes"]);
    assert!(words.hits[0].why.is_none(), "words mode has no markers");

    let meaning = run(&session, &models, &query("feeling stuck", SearchMode::Meaning));
    assert!(meaning.by_meaning);
    let found = titles(&meaning);
    assert!(found.contains(&"Going nowhere".to_string()), "{found:?}");
    assert!(found.contains(&"Feeling stuck notes".to_string()), "the words hit stays: {found:?}");
    assert!(!found.contains(&"Beach day".to_string()), "below the threshold: {found:?}");

    let by_title = |t: &str| meaning.hits.iter().find(|h| h.note.title == t).unwrap();
    let nowhere = by_title("Going nowhere");
    let why = nowhere.why.unwrap();
    assert!(!why.words);
    assert!((why.cosine.unwrap() - 1.0).abs() < 1e-4, "the exact cosine, not the i8 one: {why:?}");
    // No matched words: the start of the body as its snippet.
    assert!(nowhere.snippet.as_deref().unwrap().starts_with("Another week of going nowhere"), "{:?}", nowhere.snippet);
    let stuck = by_title("Feeling stuck notes").why.unwrap();
    assert!(stuck.words);
    assert!(by_title("Feeling stuck notes").snippet.as_deref().unwrap().contains('\u{2}'), "a words hit keeps its snippet");
}

#[test]
fn filters_and_the_folder_apply_to_vector_hits() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "journal", "In journal", "One.\nv 1 0 0 0");
    note(&session, "work", "In work", "Two.\nv 1 0 0 0");
    note(&session, "work/deep", "In work below", "Three.\nv 0.9 0.1 0 0");
    index(&session, generation, &models);

    let mut q = query("stuck again", SearchMode::Meaning);
    q.folder = Some("work".into());
    let mut found = titles(&run(&session, &models, &q));
    found.sort();
    assert_eq!(found, vec!["In work", "In work below"]);

    session.with_vault(|v| v.edit_notes_metadata(&[1], &hush_lib::db::MetadataEdit::AddTag("diary".into())).map(|_| ())).unwrap();
    let mut q = query("stuck again", SearchMode::Meaning);
    q.tags = vec!["diary".into()];
    assert_eq!(titles(&run(&session, &models, &q)), vec!["In journal"]);
}

#[test]
fn a_folder_outside_the_vault_wide_best_still_returns_its_matches() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    // More exact matches elsewhere than the vector side keeps, all closer than the one here.
    session
        .with_vault(|v| {
            for i in 0..VECTOR_LIMIT + 20 {
                v.create_note("crowd", &format!("crowd {i}"), "Same.\nv 1 0 0 0")?;
            }
            Ok(())
        })
        .unwrap();
    note(&session, "small", "Close enough", "Mine.\nv 0.8 0.6 0 0");
    index(&session, generation, &models);

    let whole = titles(&run(&session, &models, &query("stuck", SearchMode::Meaning)));
    assert!(!whole.contains(&"Close enough".to_string()), "not among the vault's best");
    let mut q = query("stuck", SearchMode::Meaning);
    q.folder = Some("small".into());
    assert_eq!(titles(&run(&session, &models, &q)), vec!["Close enough"]);
}

#[test]
fn trashed_notes_never_appear() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    let gone = note(&session, "", "Gone", "Bye.\nv 1 0 0 0");
    note(&session, "", "Kept", "Hi.\nv 1 0 0 0");
    index(&session, generation, &models);
    session.with_vault(|v| v.trash_note(gone)).unwrap();
    // Before the job has dropped it from the index.
    assert!(session.related_index().as_ref().unwrap().contains(gone));
    assert_eq!(titles(&run(&session, &models, &query("stuck", SearchMode::Meaning))), vec!["Kept"]);
    let mut q = query("stuck", SearchMode::Meaning);
    q.folder = Some("".into());
    q.min_rating = None;
    assert_eq!(titles(&run(&session, &models, &q)), vec!["Kept"]);
    index(&session, generation, &models);
    assert!(!session.related_index().as_ref().unwrap().contains(gone), "the job drops it");
    assert_eq!(titles(&run(&session, &models, &query("stuck", SearchMode::Meaning))), vec!["Kept"]);
}

#[test]
fn an_index_of_another_model_is_not_used() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL, "other-4"]);
    note(&session, "", "Going nowhere", "Weeks.\nv 1 0 0 0");
    index(&session, generation, &models);
    // Another model becomes ready behind the index's back.
    session
        .with_vault(|v| {
            v.write_embeddings("other-4", &[(1, [0; 32], vec![1.0, 0.0, 0.0, 0.0])])?;
            v.finish_embedding_model("other-4")
        })
        .unwrap();
    let page = run(&session, &models, &query("stuck", SearchMode::Meaning));
    assert!(!page.by_meaning);
    assert_eq!(page.words_only.as_deref(), Some("the meaning index is loading"));
    assert!(page.hits.is_empty(), "no words match");
}

#[test]
fn short_and_quoted_queries_search_by_words() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "", "Going nowhere", "Weeks of s.\nv 1 0 0 0");
    note(&session, "", "Stuck exactly", "\"feeling stuck\" here");
    index(&session, generation, &models);

    let one = run(&session, &models, &query("s", SearchMode::Meaning));
    assert!(!one.by_meaning && one.words_only.is_none());
    assert_eq!(titles(&one), vec!["Going nowhere"]);

    let quoted = run(&session, &models, &query("\"feeling stuck\"", SearchMode::Meaning));
    assert!(!quoted.by_meaning);
    assert_eq!(quoted.words_only.as_deref(), Some("quoted phrases search by words"));
    assert_eq!(titles(&quoted), vec!["Stuck exactly"]);

    // No text: the same list as a words search.
    let empty = run(&session, &models, &query("", SearchMode::Meaning));
    assert_eq!(titles(&empty), titles(&run(&session, &models, &query("", SearchMode::Words))));
}

#[test]
fn without_a_ready_model_meaning_searches_words() {
    let (_dir, _path, session, _generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "", "Going nowhere", "stuck weeks");
    let page = run(&session, &models, &query("stuck", SearchMode::Meaning));
    assert!(!page.by_meaning);
    assert_eq!(page.words_only.as_deref(), Some("no model is ready"));
    assert_eq!(titles(&page), vec!["Going nowhere"]);
}

#[test]
fn the_fused_list_is_cut_and_paged() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    session
        .with_vault(|v| {
            for i in 0..meaning::FUSED_LIMIT + 50 {
                v.create_note("", &format!("n {i}"), "stuck here")?;
            }
            Ok(())
        })
        .unwrap();
    index(&session, generation, &models);
    let mut q = query("stuck", SearchMode::Meaning);
    q.limit = Some(400);
    let first = run(&session, &models, &q);
    assert_eq!(first.top, Some(meaning::FUSED_LIMIT));
    assert_eq!(first.hits.len(), 400);
    q.offset = 800;
    assert_eq!(run(&session, &models, &q).hits.len(), 200, "the cut list ends at 1,000");
}

#[test]
fn a_newer_search_cancels_an_older_meaning_search() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "", "Going nowhere", "Weeks.\nv 1 0 0 0");
    index(&session, generation, &models);
    let ticket = session.begin_search();
    // A newer search starts while the older one is between steps.
    let _ = session.search(|_, _| Ok(Some(())));
    assert!(!session.search_is_current(ticket));
    assert_eq!(session.run_search(ticket, |_, _| Ok(Some(1))).unwrap(), None);
}

#[test]
fn a_broad_read_status_filter_applies_to_vector_hits() {
    let (_dir, _path, session, generation) = session();
    let models = Models::new(&[MODEL]);
    note(&session, "", "Unread one", "One.\nv 1 0 0 0");
    let read = note(&session, "", "Read one", "Two.\nv 1 0 0 0");
    index(&session, generation, &models);
    session.with_vault(|v| v.set_read_status(&[read], hush_lib::db::ReadStatus::Read).map(|_| ())).unwrap();
    for f in [hush_lib::db::ReadFilter::Unread, hush_lib::db::ReadFilter::NotFinished] {
        let mut q = query("stuck", SearchMode::Meaning);
        q.read_status = Some(f);
        assert_eq!(titles(&run(&session, &models, &q)), vec!["Unread one"], "{f:?}");
    }
}
