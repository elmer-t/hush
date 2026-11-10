//! Search by meaning (#146) with the real bundled model. Needs the files from
//! `npm run fetch-model`; skipped without them. Prints hit@10 for hand-written queries
//! on the fixture vault, and how much weaker a paragraph matches once it is one part of
//! a long note (a note's vector is the mean of its chunks).
//!
//! `cargo test --release --test meaning_real_model -- --nocapture`

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hush_lib::db::{SearchMode, SearchQuery};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::onnx::OnnxEmbedder;
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::import;
use hush_lib::meaning::{self, QueryModels};
use hush_lib::session::Session;

struct Real(Arc<OnnxEmbedder>);

impl EmbedderSource for Real {
    fn embedder(&self, _: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(self.0.clone())
    }
}

impl QueryModels for Real {
    fn query_embedder(&self, _: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(self.0.clone())
    }

    fn query_loaded(&self, _: &str) -> bool {
        true
    }
}

fn index(session: &Session, generation: u64, source: &dyn EmbedderSource) {
    for _ in 0..20 {
        if job::pass(session, generation, source, &mut |_| {}).unwrap().stale == 0 {
            return;
        }
    }
    panic!("indexing never finished");
}

/// Titles of a meaning search's hits, best first, with their cosine (if found by meaning).
fn search(session: &Session, source: &Real, text: &str) -> Vec<(String, Option<f32>, bool)> {
    let q = SearchQuery { text: text.into(), mode: SearchMode::Meaning, ..Default::default() };
    let page = meaning::search(session, source, &q).unwrap().unwrap();
    assert!(page.by_meaning, "{text}: {:?}", page.words_only);
    page.hits.into_iter().map(|h| (h.note.title, h.why.and_then(|w| w.cosine), h.why.is_some_and(|w| w.words))).collect()
}

/// Hand-written: a query that shares (almost) no word with the note it should find.
const QUERIES: &[(&str, &[&str])] = &[
    ("boat crossing schedule", &["thread-0001", "thread-0002"]),
    ("nightmare about soaring above town", &["2004-03-14 Flying"]),
    ("correspondence with grandmother", &["letter to Jan", "2004-04-01 Rain"]),
    ("plans and promises for january", &["2005-01-02 New year"]),
    ("money discussion with staff", &["meeting 20210202"]),
    ("stroll by the water at dawn", &["2004-03-12 Morning walk"]),
    ("foggy coast beacon", &["Dream of the sea"]),
    ("stayed indoors during bad weather", &["2004-04-01 Rain"]),
];

/// Queries nothing in the fixture vault is about: their best cosine shows where
/// "no real match" sits for the threshold.
const UNRELATED: &[&str] =
    &["quantum chromodynamics lecture", "replacing a car gearbox", "kubernetes pod eviction", "chocolate cake recipe"];

/// Sentences for long notes, none about the paragraphs searched for below.
const FILLER: &[&str] = &[
    "The garden needs weeding before the tomatoes can be staked.",
    "We repainted the kitchen cupboards a pale shade of green.",
    "The quarterly invoice was paid two days late this time.",
    "My bicycle chain keeps slipping on the steepest hill.",
    "The library extended its opening hours on Thursdays.",
    "We discussed whether the server migration could wait.",
    "The cat sleeps on the radiator every afternoon.",
    "A new bakery opened next to the pharmacy downtown.",
    "The printer jammed again during the presentation rehearsal.",
    "I sorted the photographs from the summer festival by date.",
    "The neighbours are building an extension at the back.",
    "Our football team lost the match after extra time.",
    "The recipe calls for more garlic than I expected.",
    "Traffic on the ring road was slow because of roadworks.",
    "The spreadsheet formulas broke after the column was moved.",
    "I finally replaced the battery in the smoke detector.",
    "The choir rehearses the new piece on Wednesday evenings.",
    "We compared three quotes for the roof repair.",
    "The children built a den out of cardboard boxes.",
    "The software update changed where the settings are.",
];

/// About `words` words of filler, varied by `seed`.
fn filler(seed: usize, words: usize) -> String {
    let mut out = String::new();
    let mut i = seed;
    while out.split_whitespace().count() < words {
        out.push_str(FILLER[i % FILLER.len()]);
        out.push(if i % 5 == 4 { '\n' } else { ' ' });
        i = i * 7 + 3;
    }
    out
}

/// One paragraph searched for inside long notes, and a query for it sharing no words.
const NEEDLES: &[(&str, &str)] = &[
    (
        "At the airport I realised my passport was still in the hotel safe. The airline let me rebook on the evening flight once the taxi brought it back.",
        "lost travel documents before departure",
    ),
    (
        "Grandfather taught me to sharpen a chisel on the whetstone, keeping the bevel flat and counting the strokes on each side.",
        "learning woodworking tool care from an old relative",
    ),
    (
        "The vet said the dog's limp comes from a torn ligament and that surgery is the only lasting fix.",
        "injured pet needs an operation",
    ),
    (
        "I panicked in the middle of the exam when I could not remember a single formula, then slowly wrote out the derivation from first principles.",
        "blanking out during a test",
    ),
];

#[test]
fn measure_meaning_search_on_the_fixture_vault() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    if !dir.join(d.model.name).is_file() || !dir.join(d.tokenizer.name).is_file() {
        eprintln!("SKIPPED: model files missing in {}; run `npm run fetch-model`", dir.display());
        return;
    }
    let model = Arc::new(OnnxEmbedder::load(&dir, d).unwrap());
    let source = Real(model.clone());

    let (vault_dir, vault) = common::vault();
    vault.set_embedding_model(d.id).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, vault_dir.path().join("vault.hush")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
    import::run(&session, &fixture, "", false, &AtomicBool::new(false), |_| {}).unwrap();
    index(&session, generation, &source);

    let titles: Vec<String> = session
        .with_vault(|v| {
            let mut out = Vec::new();
            for folder in std::iter::once(String::new()).chain(v.folders()?) {
                out.extend(v.folder_notes(&folder, 0, 1000)?.into_iter().map(|n| n.title));
            }
            Ok(out)
        })
        .unwrap();
    for (_, expected) in QUERIES {
        for e in *expected {
            assert!(titles.iter().any(|t| t == e), "no fixture note titled {e:?}; titles: {titles:?}");
        }
    }
    println!("\n== fixture vault, threshold {}", d.meaning_threshold);
    let mut hits = 0;
    let mut relevant_cosines = Vec::new();
    let mut other_best: f32 = -1.0;
    for (query, expected) in QUERIES {
        let found = search(&session, &source, query);
        let top: Vec<&str> = found.iter().take(10).map(|(t, _, _)| t.as_str()).collect();
        let hit = expected.iter().any(|e| top.contains(e));
        hits += usize::from(hit);
        // Every note's exact cosine, to place the threshold.
        let q = model.embed_query(query).unwrap();
        let mut all = session
            .with_vault(|v| {
                let mut out = Vec::new();
                for folder in std::iter::once(String::new()).chain(v.folders()?) {
                    for n in v.folder_notes(&folder, 0, 1000)? {
                        if let Some(x) = v.note_embedding(n.id)? {
                            out.push((embed::cosine(&q, &x), n.title.clone()));
                        }
                    }
                }
                Ok(out)
            })
            .unwrap();
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (c, t) in &all {
            if expected.contains(&t.as_str()) {
                relevant_cosines.push(*c);
            } else {
                other_best = other_best.max(*c);
            }
        }
        let shown: Vec<String> = all.iter().take(4).map(|(c, t)| format!("{c:.3}{} {t}", if expected.contains(&t.as_str()) { "*" } else { "" })).collect();
        println!("{} {query:40} | {}", if hit { "hit " } else { "MISS" }, shown.join(" | "));
    }
    println!("hit@10: {hits} of {}", QUERIES.len());
    relevant_cosines.sort_by(|a, b| a.total_cmp(b));
    println!(
        "relevant notes' cosine: min {:.3}, median {:.3}; best irrelevant {:.3}",
        relevant_cosines[0],
        relevant_cosines[relevant_cosines.len() / 2],
        other_best
    );
    for query in UNRELATED {
        let q = model.embed_query(query).unwrap();
        let best = session
            .with_vault(|v| {
                let mut best: f32 = -1.0;
                for folder in std::iter::once(String::new()).chain(v.folders()?) {
                    for n in v.folder_notes(&folder, 0, 1000)? {
                        if let Some(x) = v.note_embedding(n.id)? {
                            best = best.max(embed::cosine(&q, &x));
                        }
                    }
                }
                Ok(best)
            })
            .unwrap();
        let found = search(&session, &source, query).iter().filter(|h| h.1.is_some()).count();
        println!("unrelated {query:40} best cosine {best:.3}, notes over the threshold {found}");
    }

    // A paragraph inside a long note: the note's vector is the mean of its chunks, so
    // the paragraph weighs less the longer the note is.
    println!("\n== one paragraph inside a long note (cosine of the query to the note; rank among {} filler notes)", 30);
    session
        .with_vault(|v| {
            for i in 0..30 {
                v.create_note("filler", &format!("filler {i}"), &filler(i, 3000))?;
            }
            Ok(())
        })
        .unwrap();
    let lengths = [0usize, 400, 1000, 3000];
    let chunks: Vec<String> = lengths
        .iter()
        .map(|&w| format!("{w}w = {} chunks", model.chunk_lengths(&filler(1, w.max(1))).unwrap().len()))
        .collect();
    println!("({})", chunks.join(", "));
    for (n, (paragraph, _)) in NEEDLES.iter().enumerate() {
        for &words in &lengths {
            // The paragraph in the middle of `words` words of filler.
            let half = filler(100 + n, words / 2);
            let body = if words == 0 { paragraph.to_string() } else { format!("{half}\n\n{paragraph}\n\n{}", filler(200 + n, words / 2)) };
            session.with_vault(|v| v.create_note("needles", &format!("needle {n} in {words}"), &body)).unwrap();
        }
    }
    index(&session, generation, &source);
    let mut long_hits = 0;
    for (n, (_, query)) in NEEDLES.iter().enumerate() {
        let found = search(&session, &source, query);
        let mut row = Vec::new();
        for &words in &lengths {
            let title = format!("needle {n} in {words}");
            let rank = found.iter().position(|(t, _, _)| *t == title);
            let cosine = found.iter().find(|(t, _, _)| *t == title).and_then(|h| h.1);
            let q = model.embed_query(query).unwrap();
            let exact = session
                .with_vault(|v| {
                    let id = v.folder_notes("needles", 0, 100)?.into_iter().find(|s| s.title == title).unwrap().id;
                    Ok(v.note_embedding(id)?.map(|x| embed::cosine(&q, &x)))
                })
                .unwrap()
                .unwrap();
            if words == 3000 && rank.is_some_and(|r| r < 10) {
                long_hits += 1;
            }
            row.push(format!(
                "{words:>4}w: {exact:.3} rank {}{}",
                rank.map_or("-".to_string(), |r| (r + 1).to_string()),
                if cosine.is_none() { " (below threshold)" } else { "" }
            ));
        }
        println!("{query:52} | {}", row.join(" | "));
    }
    println!("3,000-word notes found in the top 10: {long_hits} of {}", NEEDLES.len());

    // Most fixture queries must be found; this guards the threshold and the fusion.
    assert!(hits * 4 >= QUERIES.len() * 3, "hit@10 {hits} of {}", QUERIES.len());
}
