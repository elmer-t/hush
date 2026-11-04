//! The graph view's data: a subgraph of notes and their connections for one scope,
//! never the whole vault. A **local** graph is the open note and its link neighbours to
//! a depth of 1–3; a **global** graph is a folder, a tag or a search, cut to the most
//! linked notes when it has more than the node cap. Edges are resolved links, and
//! optionally meaning (the related-notes vectors), tags and unresolved targets.
//!
//! Every query goes through the indexes on `links` and `note_tags` for the notes in
//! the graph, and the vault is taken per page, so a large scope doesn't hold it for
//! the whole build. Nothing is stored: the graph lives in the frontend's memory.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::db::{GraphScopeNotes, LinkTarget, NoteSummary, Resolver};
use crate::embed::{self, related};
use crate::session::{Session, SessionError};

pub const DEFAULT_CAP: usize = 5_000;
pub const MAX_CAP: usize = 50_000;
pub const MAX_DEPTH: u8 = 3;
/// Notes whose links, link counts, summaries or tags are read per vault access.
const PAGE: usize = 2_000;
/// Fewest notes per thread when comparing vectors pairwise.
const MIN_ROWS_PER_THREAD: usize = 250;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GraphScope {
    /// Note `id` and the notes linked to or from it, up to `depth` links away.
    Local { id: i64, depth: u8 },
    /// The notes of a folder, a tag, a search or the whole vault.
    Global { scope: GraphScopeNotes },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GraphOptions {
    /// Meaning edges: each note's closest notes in the graph by its vector.
    pub meaning: bool,
    /// Tag nodes, each joined to its notes.
    pub tags: bool,
    /// With `tags`, also the nodes of tags that only one note in the vault has; without
    /// it they are left out, as most of them are typos or one-offs.
    pub rare_tags: bool,
    /// Unresolved link targets as nodes.
    pub ghosts: bool,
    /// Lowest cosine similarity a meaning edge needs.
    pub threshold: f32,
    /// Meaning edges per note, at most.
    pub neighbours: usize,
    /// Most notes in the graph (tag and ghost nodes not counted).
    pub cap: usize,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self { meaning: false, tags: false, rare_tags: false, ghosts: false, threshold: 0.7, neighbours: 5, cap: DEFAULT_CAP }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Note,
    Tag,
    Ghost,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    /// `n<id>` for a note, `t:<tag>` for a tag, `g:<target>` (lowercased) for a ghost.
    pub key: String,
    pub kind: NodeKind,
    /// The note's id; `None` for tag and ghost nodes.
    pub id: Option<i64>,
    pub label: String,
    pub folder: String,
    pub topic: Option<String>,
    pub tags: Vec<String>,
    /// Links away from the local graph's centre; `None` in a global graph.
    pub depth: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    /// `source` links to `target`.
    Link,
    /// Close in meaning; undirected.
    Meaning,
    /// `source` (a note) has tag `target`.
    Tag,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    /// The cosine similarity of a meaning edge.
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Notes in the scope: for a local graph, those found before the cap stopped it.
    pub total: usize,
    /// Whether the cap left notes out.
    pub capped: bool,
    /// Meaning edges were asked for but no model is ready.
    pub meaning_unavailable: bool,
}

pub fn note_key(id: i64) -> String {
    format!("n{id}")
}

/// Builds the graph for `scope`. Only the scope's own queries take the vault, a page
/// at a time; it counts as activity, being the user's request.
pub fn build(session: &Session, scope: &GraphScope, options: &GraphOptions) -> Result<Graph, SessionError> {
    let cap = options.cap.clamp(1, MAX_CAP);
    let mut resolver = Resolver::default();
    let (mut ids, mut depths, total, capped) = match scope {
        GraphScope::Local { id, depth } => local(session, *id, (*depth).clamp(1, MAX_DEPTH), cap, &mut resolver)?,
        GraphScope::Global { scope } => {
            let (ids, total, capped) = global(session, scope, cap)?;
            (ids, HashMap::new(), total, capped)
        }
    };

    // The local graph's centre gets its closest notes in meaning too, if there is room.
    let mut meaning_unavailable = false;
    let mut centre_meaning = Vec::new();
    if let (GraphScope::Local { id, .. }, true) = (scope, options.meaning) {
        match related::related_notes(session, *id, options.neighbours)? {
            Some(notes) => {
                for n in notes.into_iter().filter(|n| n.score >= options.threshold) {
                    if !depths.contains_key(&n.note.id) && ids.len() < cap {
                        depths.insert(n.note.id, 1);
                        ids.push(n.note.id);
                    }
                    centre_meaning.push((n.note.id, n.score));
                }
            }
            None => meaning_unavailable = true,
        }
    }

    let mut summaries: Vec<NoteSummary> = Vec::with_capacity(ids.len());
    let mut links: Vec<(i64, LinkTarget)> = Vec::new();
    let mut tags: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for page in ids.chunks(PAGE) {
        session.with_vault(|v| {
            summaries.extend(v.note_summaries(page)?);
            links.extend(v.graph_links_from(page, &mut resolver)?);
            tags.extend(v.graph_tags(page)?);
            Ok(())
        })?;
    }
    // Notes trashed since the scope was read are left out.
    ids = summaries.iter().map(|n| n.id).collect();
    let present: HashSet<i64> = ids.iter().copied().collect();

    let mut nodes: Vec<GraphNode> = summaries
        .into_iter()
        .map(|n| GraphNode {
            key: note_key(n.id),
            kind: NodeKind::Note,
            id: Some(n.id),
            depth: depths.get(&n.id).copied(),
            tags: tags.get(&n.id).cloned().unwrap_or_default(),
            label: n.title,
            folder: n.folder,
            topic: n.topic,
        })
        .collect();
    let mut edges = Vec::new();
    let mut ghosts: BTreeMap<String, String> = BTreeMap::new();
    for (from, to) in links {
        match to {
            LinkTarget::Note(to) if present.contains(&to) && to != from => {
                edges.push(GraphEdge { source: note_key(from), target: note_key(to), kind: EdgeKind::Link, score: None });
            }
            LinkTarget::Missing(target) if options.ghosts && (ghosts.len() < cap || ghosts.contains_key(&target.to_lowercase())) => {
                let key = format!("g:{}", target.to_lowercase());
                ghosts.entry(target.to_lowercase()).or_insert(target);
                edges.push(GraphEdge { source: note_key(from), target: key, kind: EdgeKind::Link, score: None });
            }
            _ => {}
        }
    }
    for (lower, target) in ghosts {
        let (folder, title) = crate::markdown::split_target(&target);
        nodes.push(GraphNode {
            key: format!("g:{lower}"),
            kind: NodeKind::Ghost,
            id: None,
            label: title,
            folder,
            topic: None,
            tags: Vec::new(),
            depth: None,
        });
    }
    if options.tags {
        // Notes in view per tag.
        let mut in_view: BTreeMap<&str, usize> = BTreeMap::new();
        for tag in tags.iter().filter(|(id, _)| present.contains(id)).flat_map(|(_, l)| l.iter()) {
            *in_view.entry(tag).or_default() += 1;
        }
        let mut shown: BTreeSet<&str> = in_view.keys().copied().collect();
        if !options.rare_tags {
            // Only a tag on one note in view can be on one note in the vault.
            let once: Vec<&str> = in_view.iter().filter(|(_, n)| **n == 1).map(|(t, _)| *t).collect();
            let mut rare = Vec::new();
            for page in once.chunks(PAGE) {
                session.with_vault(|v| {
                    rare.extend(v.rare_tags(page)?);
                    Ok(())
                })?;
            }
            for tag in &rare {
                shown.remove(tag.as_str());
            }
        }
        for (id, list) in &tags {
            if !present.contains(id) {
                continue;
            }
            for tag in list.iter().filter(|t| shown.contains(t.as_str())) {
                edges.push(GraphEdge { source: note_key(*id), target: format!("t:{tag}"), kind: EdgeKind::Tag, score: None });
            }
        }
        for tag in shown {
            nodes.push(GraphNode {
                key: format!("t:{tag}"),
                kind: NodeKind::Tag,
                id: None,
                label: format!("#{tag}"),
                folder: String::new(),
                topic: None,
                tags: Vec::new(),
                depth: None,
            });
        }
    }
    if options.meaning && !meaning_unavailable {
        match meaning_edges(session, &ids, options)? {
            Some(mut pairs) => {
                // The centre's own closest notes are already exact; keep them even if
                // they are not in each other's top few.
                if let GraphScope::Local { id, .. } = scope {
                    for &(other, score) in &centre_meaning {
                        if present.contains(&other) {
                            pairs.insert((*id.min(&other), *id.max(&other)), score);
                        }
                    }
                }
                edges.extend(pairs.into_iter().map(|((a, b), score)| GraphEdge {
                    source: note_key(a),
                    target: note_key(b),
                    kind: EdgeKind::Meaning,
                    score: Some(score),
                }));
            }
            None => meaning_unavailable = true,
        }
    }
    Ok(Graph { nodes, edges, total, capped, meaning_unavailable })
}

type Scoped = (Vec<i64>, HashMap<i64, u8>, usize, bool);

/// Breadth first over links in both directions, closest notes first. It stops as soon
/// as the cap is reached, so `total` is then a lower bound (the notes found so far).
fn local(session: &Session, id: i64, depth: u8, cap: usize, resolver: &mut Resolver) -> Result<Scoped, SessionError> {
    let mut depths = HashMap::from([(id, 0u8)]);
    let mut ids = vec![id];
    let mut frontier = vec![id];
    let mut beyond_cap: HashSet<i64> = HashSet::new();
    'levels: for d in 1..=depth {
        let mut next = Vec::new();
        for page in frontier.chunks(PAGE / 10) {
            let neighbours = session.with_vault(|v| {
                let mut found = Vec::new();
                for (_, to) in v.graph_links_from(page, resolver)? {
                    if let LinkTarget::Note(to) = to {
                        found.push(to);
                    }
                }
                for &x in page {
                    found.extend(v.graph_links_to(x, resolver)?);
                }
                Ok(found)
            })?;
            for n in neighbours {
                if depths.contains_key(&n) {
                    continue;
                }
                if ids.len() >= cap {
                    beyond_cap.insert(n);
                    continue;
                }
                depths.insert(n, d);
                ids.push(n);
                next.push(n);
            }
            if !beyond_cap.is_empty() {
                break 'levels;
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let total = ids.len() + beyond_cap.len();
    Ok((ids, depths, total, !beyond_cap.is_empty()))
}

/// Every note of `scope`, or the `cap` most linked when it has more (ties by id).
fn global(session: &Session, scope: &GraphScopeNotes, cap: usize) -> Result<(Vec<i64>, usize, bool), SessionError> {
    let mut ids = Vec::new();
    let mut after = 0;
    loop {
        let (page, next) = session.with_vault(|v| v.graph_scope_page(scope, after))?;
        ids.extend(page);
        match next {
            Some(n) => after = n,
            None => break,
        }
    }
    let total = ids.len();
    if total <= cap {
        return Ok((ids, total, false));
    }
    let mut counted: Vec<(u32, i64)> = Vec::with_capacity(total);
    for page in ids.chunks(PAGE * 5) {
        let counts = session.with_vault(|v| v.graph_link_counts(page))?;
        counted.extend(counts.into_iter().zip(page.iter().copied()));
    }
    counted.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    counted.truncate(cap);
    Ok((counted.into_iter().map(|(_, id)| id).collect(), total, true))
}

/// For each of `ids` with a vector, its `neighbours` closest others among `ids` at or
/// above the threshold, by the exact cosine of the stored vectors, as undirected pairs
/// `(low id, high id)`. `None` when no model is ready.
fn meaning_edges(
    session: &Session,
    ids: &[i64],
    options: &GraphOptions,
) -> Result<Option<BTreeMap<(i64, i64), f32>>, SessionError> {
    let Some(model) = session.with_vault(|v| v.embedding_model_ready())? else { return Ok(None) };
    let mut vectors = Vec::with_capacity(ids.len());
    for page in ids.chunks(PAGE) {
        vectors.extend(session.with_vault(|v| v.embedding_vectors(&model, page))?);
    }
    let Some(dims) = vectors.first().map(|v| v.vector.len()) else { return Ok(Some(BTreeMap::new())) };
    vectors.retain(|v| v.vector.len() == dims);
    let n = vectors.len();
    let mut units = Zeroizing::new(Vec::with_capacity(n * dims));
    for v in &vectors {
        let start = units.len();
        units.extend_from_slice(&v.vector);
        embed::normalize(&mut units[start..]);
    }
    let units = &units[..];
    let k = options.neighbours.max(1);
    let threshold = options.threshold;
    // Best `k` of row `i` against every other row.
    let best = |i: usize| -> Vec<(usize, f32)> {
        let row = &units[i * dims..(i + 1) * dims];
        let mut top: Vec<(usize, f32)> = Vec::with_capacity(k + 1);
        for (j, other) in units.chunks_exact(dims).enumerate() {
            if j == i {
                continue;
            }
            let score = related::dot(row, other);
            if score < threshold || (top.len() == k && score <= top[k - 1].1) {
                continue;
            }
            let at = top.partition_point(|&(_, s)| s >= score);
            top.insert(at, (j, score));
            top.truncate(k);
        }
        top
    };
    let threads =
        std::thread::available_parallelism().map_or(1, |c| (c.get() / 2).max(1)).min(n.div_ceil(MIN_ROWS_PER_THREAD)).max(1);
    let per = n.div_ceil(threads).max(1);
    let rows: Vec<Vec<(usize, f32)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..n)
            .step_by(per)
            .map(|start| s.spawn(move || (start..(start + per).min(n)).map(best).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("meaning edges panicked")).collect()
    });
    let mut pairs = BTreeMap::new();
    for (i, top) in rows.into_iter().enumerate() {
        for (j, score) in top {
            let (a, b) = (vectors[i].id, vectors[j].id);
            pairs.insert((a.min(b), a.max(b)), score);
        }
    }
    Ok(Some(pairs))
}
