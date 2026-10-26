mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use hush_lib::db::{GraphScopeNotes, NewNote, SearchQuery, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder};
use hush_lib::graph::{self, EdgeKind, Graph, GraphOptions, GraphScope, NodeKind};
use hush_lib::session::Session;
use tempfile::TempDir;

/// A note whose body has a line `v 1 0 0` gets that vector (normalised).
struct Controlled;

impl Embedder for Controlled {
    fn model_id(&self) -> &str {
        "ctl-3"
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
    fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(self.0.clone())
    }
}

fn session() -> (TempDir, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model("ctl-3").unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path).unwrap();
    (dir, session, generation)
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

fn note(session: &Session, folder: &str, title: &str, body: &str) -> i64 {
    session.with_vault(|v| v.create_note(folder, title, body)).unwrap()
}

fn labels(g: &Graph, kind: NodeKind) -> BTreeSet<String> {
    g.nodes.iter().filter(|n| n.kind == kind).map(|n| n.label.clone()).collect()
}

/// Edges of `kind` as `source label -> target label`.
fn edges(g: &Graph, kind: EdgeKind) -> BTreeSet<String> {
    let label = |key: &str| g.nodes.iter().find(|n| n.key == key).unwrap().label.clone();
    g.edges.iter().filter(|e| e.kind == kind).map(|e| format!("{} -> {}", label(&e.source), label(&e.target))).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn local(id: i64, depth: u8) -> GraphScope {
    GraphScope::Local { id, depth }
}

/// A chain A -> B -> C -> D, E -> A, and A -> [[Nowhere]].
fn chain(session: &Session) -> [i64; 5] {
    let a = note(session, "", "A", "[[B]] [[Nowhere]] #x");
    let b = note(session, "", "B", "[[C]] #x");
    let c = note(session, "", "C", "[[D]]");
    let d = note(session, "", "D", "");
    let e = note(session, "", "E", "[[A]]");
    [a, b, c, d, e]
}

#[test]
fn local_graph_follows_links_both_ways_to_the_depth() {
    let (_dir, session, _) = session();
    let [a, ..] = chain(&session);
    let opts = GraphOptions::default();

    let g = graph::build(&session, &local(a, 1), &opts).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["A", "B", "E"]));
    assert_eq!(edges(&g, EdgeKind::Link), set(&["A -> B", "E -> A"]));
    assert!(!g.capped);
    let depth = |t: &str| g.nodes.iter().find(|n| n.label == t).unwrap().depth;
    assert_eq!((depth("A"), depth("B")), (Some(0), Some(1)));

    let g = graph::build(&session, &local(a, 2), &opts).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["A", "B", "C", "E"]));
    assert_eq!(edges(&g, EdgeKind::Link), set(&["A -> B", "B -> C", "E -> A"]));

    let g = graph::build(&session, &local(a, 3), &opts).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["A", "B", "C", "D", "E"]));
    assert_eq!(g.total, 5);
}

#[test]
fn the_cap_keeps_the_closest_notes_and_says_so() {
    let (_dir, session, _) = session();
    let [a, ..] = chain(&session);
    let opts = GraphOptions { cap: 2, ..Default::default() };
    let g = graph::build(&session, &local(a, 3), &opts).unwrap();
    assert_eq!(g.nodes.len(), 2);
    assert!(g.capped);
    assert_eq!(g.total, 3, "A plus both neighbours found at depth 1");
    assert!(labels(&g, NodeKind::Note).contains("A"));
}

#[test]
fn tag_and_ghost_nodes_are_toggles() {
    let (_dir, session, _) = session();
    let [a, ..] = chain(&session);
    let g = graph::build(&session, &local(a, 1), &GraphOptions::default()).unwrap();
    assert!(labels(&g, NodeKind::Tag).is_empty() && labels(&g, NodeKind::Ghost).is_empty());
    assert_eq!(g.nodes.iter().find(|n| n.label == "A").unwrap().tags, ["x"], "tags are there for colouring");

    let opts = GraphOptions { tags: true, ghosts: true, ..Default::default() };
    let g = graph::build(&session, &local(a, 1), &opts).unwrap();
    assert_eq!(labels(&g, NodeKind::Tag), set(&["#x"]));
    assert_eq!(edges(&g, EdgeKind::Tag), set(&["A -> #x", "B -> #x"]));
    assert_eq!(labels(&g, NodeKind::Ghost), set(&["Nowhere"]));
    assert!(edges(&g, EdgeKind::Link).contains("A -> Nowhere"));
}

#[test]
fn global_graph_is_scoped_and_capped_by_link_count() {
    let (_dir, session, _) = session();
    note(&session, "p", "Hub", "");
    note(&session, "p", "One", "[[Hub]] [[Two]]");
    note(&session, "p", "Two", "[[Hub]]");
    note(&session, "p", "Lonely", "");
    note(&session, "q", "Outside", "[[Hub]] #t");
    let nested = note(&session, "q", "Nested", "#t/sub");
    note(&session, "q", "Other", "#tx #T");

    let folder = GraphScope::Global { scope: GraphScopeNotes::Folder { folder: "p".into() } };
    let g = graph::build(&session, &folder, &GraphOptions::default()).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["Hub", "One", "Two", "Lonely"]));
    assert_eq!(edges(&g, EdgeKind::Link), set(&["One -> Hub", "One -> Two", "Two -> Hub"]));
    assert_eq!((g.total, g.capped), (4, false));

    // Hub: 3 links in; One: 2 out; Two: 1 out + 1 in; Lonely: none.
    let g = graph::build(&session, &folder, &GraphOptions { cap: 3, ..Default::default() }).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["Hub", "One", "Two"]));
    assert_eq!((g.total, g.capped), (4, true));

    let tag = GraphScope::Global { scope: GraphScopeNotes::Tag { tag: "t".into() } };
    let g = graph::build(&session, &tag, &GraphOptions::default()).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["Outside", "Nested"]), "nested tags, case-sensitive");
    assert!(g.edges.is_empty(), "Hub is outside the scope");
    session.with_vault(|v| v.trash_note(nested)).unwrap();
    let g = graph::build(&session, &tag, &GraphOptions::default()).unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["Outside"]), "trashed notes are left out");

    let search = SearchQuery { text: "Hub".into(), folder: Some("p".into()), ..Default::default() };
    let g = graph::build(&session, &GraphScope::Global { scope: GraphScopeNotes::Search { query: search } }, &GraphOptions::default())
        .unwrap();
    assert_eq!(labels(&g, NodeKind::Note), set(&["Hub", "One", "Two"]));
}

#[test]
fn meaning_edges_join_close_notes_above_the_threshold() {
    let (_dir, session, generation) = session();
    let a = note(&session, "", "A", "v 1 0 0");
    note(&session, "", "Near", "v 0.9 0.1 0");
    note(&session, "", "Far", "v 0 1 0");
    note(&session, "", "Linked", "v 0 0 1\n[[A]]");

    let opts = GraphOptions { meaning: true, threshold: 0.5, ..Default::default() };
    let g = graph::build(&session, &local(a, 1), &opts).unwrap();
    assert!(g.meaning_unavailable, "nothing indexed yet");

    index(&session, generation);
    let g = graph::build(&session, &local(a, 1), &opts).unwrap();
    assert!(!g.meaning_unavailable);
    assert_eq!(labels(&g, NodeKind::Note), set(&["A", "Linked", "Near"]), "Near joins through meaning");
    assert_eq!(edges(&g, EdgeKind::Meaning).len(), 1);
    let edge = g.edges.iter().find(|e| e.kind == EdgeKind::Meaning).unwrap();
    let expected = 0.9 / (0.9f32 * 0.9 + 0.1 * 0.1).sqrt();
    assert!((edge.score.unwrap() - expected).abs() < 1e-5);

    let all = GraphScope::Global { scope: GraphScopeNotes::All };
    let g = graph::build(&session, &all, &GraphOptions { meaning: true, threshold: 0.0, neighbours: 1, ..Default::default() })
        .unwrap();
    // Each note's single closest other: A-Near, Near-A, Far-Near, Linked-(A, Near or Far at 0).
    assert!(edges(&g, EdgeKind::Meaning).len() >= 2);
    assert!(g.edges.iter().filter(|e| e.kind == EdgeKind::Meaning).all(|e| e.score.unwrap() >= 0.0));
}

/// The subgraph queries on a synthetic vault with links: `HUSH_GRAPH_NOTES` notes
/// (default 1,000,000), each linking to three others, in folders of 20. Ignored (it
/// takes minutes to generate); run with
/// `cargo test --release --test graph -- --ignored --nocapture`.
#[test]
#[ignore]
fn subgraph_queries_at_scale() {
    let n: usize = std::env::var("HUSH_GRAPH_NOTES").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let (_dir, session, _) = session();
    let start = Instant::now();
    let bodies: Vec<(String, String, String)> = (0..n)
        .map(|i| {
            // A few hubs (every 1000th note) get most of the links, as real vaults do.
            let hub = (i * 7919) % (n / 1000).max(1) * 1000;
            (
                format!("f{}", i / 20),
                format!("Note {i}"),
                format!("[[Note {hub}]] [[Note {}]] [[Note {}]] #t{}", (i + 1) % n, (i * 31) % n, i % 100),
            )
        })
        .collect();
    for chunk in bodies.chunks(5_000) {
        let notes: Vec<NewNote> = chunk
            .iter()
            .map(|(f, t, b)| NewNote { folder: f, title: t, body: b, created_at: 0, updated_at: 0 })
            .collect();
        session.with_vault(|v| v.import_notes(&notes)).unwrap();
    }
    println!("generated {n} notes in {:?}", start.elapsed());

    // "Note 0" is a hub; "Note 1" an ordinary note.
    let hub = session.with_vault(|v| v.search_ids(&SearchQuery::default(), 0, 1)).unwrap()[0];
    let probe = hub + 1;
    for (name, scope) in [
        ("local depth 1", local(probe, 1)),
        ("local depth 2", local(probe, 2)),
        ("local depth 3", local(probe, 3)),
        ("local hub depth 1", local(hub, 1)),
        ("folder f1", GraphScope::Global { scope: GraphScopeNotes::Folder { folder: "f1".into() } }),
        ("tag t7 (cap)", GraphScope::Global { scope: GraphScopeNotes::Tag { tag: "t7".into() } }),
        ("whole vault (cap)", GraphScope::Global { scope: GraphScopeNotes::All }),
    ] {
        let start = Instant::now();
        let g = graph::build(&session, &scope, &GraphOptions::default()).unwrap();
        println!(
            "{name}: {:?}, {} nodes, {} edges, total {}, capped {}",
            start.elapsed(),
            g.nodes.len(),
            g.edges.len(),
            g.total,
            g.capped
        );
    }
}

/// Meaning edges for a global graph at the default cap: 5,000 notes with random
/// 384-dim vectors, compared pairwise. Ignored (a timing); run with
/// `cargo test --release --test graph -- --ignored --nocapture`.
#[test]
#[ignore]
fn meaning_edges_at_the_cap() {
    const N: usize = 5_000;
    const DIMS: usize = 384;
    let (_dir, session, _) = session();
    let bodies: Vec<String> = (0..N).map(|i| format!("Note {i}")).collect();
    let notes: Vec<NewNote> =
        bodies.iter().map(|t| NewNote { folder: "f", title: t, body: "", created_at: 0, updated_at: 0 }).collect();
    let ids: Vec<i64> = session.with_vault(|v| v.import_notes(&notes)).unwrap().into_iter().map(|r| r.unwrap()).collect();
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
    let all = GraphScope::Global { scope: GraphScopeNotes::All };
    let opts = GraphOptions { meaning: true, threshold: 0.1, ..Default::default() };
    for _ in 0..3 {
        let start = Instant::now();
        let g = graph::build(&session, &all, &opts).unwrap();
        let meaning = g.edges.iter().filter(|e| e.kind == EdgeKind::Meaning).count();
        println!("{N} notes x {DIMS} with meaning edges: {:?}, {meaning} meaning edges", start.elapsed());
        assert!(!g.meaning_unavailable);
    }
}
