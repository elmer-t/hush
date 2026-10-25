//! Tag discovery: groups notes by meaning (k-means on their vectors) and describes
//! each group by its notes nearest the centre and its distinctive words, so the user
//! can see which tags the archive needs and create one from a group.
//!
//! A scope of more than [`SAMPLE_SIZE`] notes is grouped from a random sample of that
//! many (fixed seed); every other note in scope is then put in the group of its nearest
//! centre in one pass over the vectors, keeping only ids, so a run's memory doesn't grow
//! with the vault.
//!
//! Nothing is stored: a run reads vectors and texts, computes in memory and returns
//! the groups. Like the embedding job it reads the vault only through
//! [`Session::with_vault_quiet`], in short pages, never while computing, and stops
//! when the vault locks.

pub mod kmeans;
pub mod words;

use serde::{Deserialize, Serialize};

pub use crate::db::DiscoverScope as Scope;
use crate::db::{self, ScopeCursor};
use crate::session::{Session, SessionError};

/// The fixed seed, so the same notes always give the same groups.
pub const SEED: u64 = 0x4855_5348; // "HUSH"
/// Fewest and most groups the default picks.
pub const DEFAULT_MIN_GROUPS: usize = 5;
pub const DEFAULT_MAX_GROUPS: usize = 50;
/// Most groups the user may ask for.
pub const MAX_GROUPS: usize = 200;
/// Most notes k-means runs on; the rest of the scope joins the nearest group.
pub const SAMPLE_SIZE: usize = 20_000;
/// Notes listed per group, nearest the centre first.
pub const GROUP_NOTES: usize = 50;
pub const WORDS_PER_GROUP: usize = 8;
/// Notes read per vault access.
const PAGE: u32 = 500;
/// Ids, or vectors in the pass over all of them, read per vault access.
const SCOPE_PAGE: u32 = 10_000;
const VECTOR_PAGE: u32 = 2_000;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Number of groups; `None` picks [`default_groups`].
    pub groups: Option<usize>,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    /// The model whose vectors were grouped; `None` while no model is ready yet.
    pub model: Option<String>,
    /// Notes grouped: those in scope that have a vector.
    pub notes: usize,
    /// Notes k-means ran on: all of them, or a sample of [`SAMPLE_SIZE`].
    pub sampled: usize,
    /// Groups asked for (the default, or the user's number capped at `sampled`).
    pub k: usize,
    /// Largest first.
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Distinctive words, best first, from the sampled notes.
    pub words: Vec<String>,
    /// Notes in the group.
    pub size: usize,
    /// The [`GROUP_NOTES`] notes nearest the centre, nearest first.
    pub notes: Vec<GroupNote>,
    /// Every note in the group, by id; kept by the backend for *Select all*, not sent.
    #[serde(skip)]
    pub members: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupNote {
    pub id: i64,
    pub folder: String,
    pub title: String,
    /// Cosine similarity to the group's centre.
    pub score: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("vault is locked")]
    Locked,
    /// `cancelled` said so: a newer run started, or the user closed the view.
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Db(#[from] db::Error),
}

impl From<SessionError> for DiscoverError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Db(e) => Self::Db(e),
            _ => Self::Locked,
        }
    }
}

/// `clamp(sqrt(n / 2), 5, 50)`, and never more than `n`.
pub fn default_groups(notes: usize) -> usize {
    ((notes as f64 / 2.0).sqrt().round() as usize).clamp(DEFAULT_MIN_GROUPS, DEFAULT_MAX_GROUPS).min(notes)
}

/// `count` of `ids` picked at random (all of them when there are no more), from `seed`,
/// so the same ids give the same sample; sorted.
pub fn sample(ids: &[i64], count: usize, seed: u64) -> Vec<i64> {
    let mut ids = ids.to_vec();
    if ids.len() > count {
        // The first `count` steps of a Fisher-Yates shuffle, on ids in a fixed order.
        ids.sort_unstable();
        let mut rng = kmeans::SplitMix64(seed);
        for i in 0..count {
            let j = i + rng.below(ids.len() - i);
            ids.swap(i, j);
        }
        ids.truncate(count);
    }
    ids.sort_unstable();
    ids
}

/// A group being filled: its size, members and the notes nearest its centre so far.
#[derive(Default)]
struct Filling {
    members: Vec<i64>,
    /// (similarity, id); kept to at most 2 × [`GROUP_NOTES`] between trims.
    nearest: Vec<(f32, i64)>,
}

impl Filling {
    fn add(&mut self, id: i64, score: f32) {
        self.members.push(id);
        self.nearest.push((score, id));
        if self.nearest.len() >= 2 * GROUP_NOTES {
            self.trim();
        }
    }

    fn trim(&mut self) {
        self.nearest.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        self.nearest.truncate(GROUP_NOTES);
    }
}

/// Groups the live notes in `options.scope` of the vault of unlock `generation` that
/// have a vector from the ready model.
pub fn run(
    session: &Session,
    generation: u64,
    options: &Options,
    cancelled: &dyn Fn() -> bool,
) -> Result<Discovery, DiscoverError> {
    run_sampled(session, generation, options, SAMPLE_SIZE, cancelled)
}

/// [`run`] with a sample of `sample_size` instead of [`SAMPLE_SIZE`], so tests can
/// sample a small vault.
#[doc(hidden)]
pub fn run_sampled(
    session: &Session,
    generation: u64,
    options: &Options,
    sample_size: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Discovery, DiscoverError> {
    let stop = || cancelled() || !session.is_open(generation);
    let check = || -> Result<(), DiscoverError> {
        if !session.is_open(generation) {
            Err(DiscoverError::Locked)
        } else if cancelled() {
            Err(DiscoverError::Cancelled)
        } else {
            Ok(())
        }
    };

    let Some(model) = session.with_vault_quiet(generation, |v| v.embedding_model_ready())? else {
        return Ok(Discovery { model: None, notes: 0, sampled: 0, k: 0, groups: Vec::new() });
    };

    // The notes in scope, ids only.
    let mut scope: Vec<i64> = Vec::new();
    let mut cursor = ScopeCursor::default();
    loop {
        check()?;
        let (page, next) = session.with_vault_quiet(generation, |v| v.discover_scope(&options.scope, &cursor, SCOPE_PAGE))?;
        scope.extend(page);
        match next {
            Some(c) => cursor = c,
            None => break,
        }
    }
    scope.sort_unstable();
    scope.dedup();

    // The sample's vectors, in pages. Every vector is normalised again, so cosine is a
    // dot product, and one of another length (a stray row) is left out.
    let chosen = sample(&scope, sample_size, SEED);
    let mut sampled: Vec<i64> = Vec::with_capacity(chosen.len());
    let mut points: Vec<f32> = Vec::new();
    let mut dims = 0;
    for page in chosen.chunks(PAGE as usize) {
        check()?;
        for (id, mut vector) in session.with_vault_quiet(generation, |v| v.discover_vectors(&model, page))? {
            if dims == 0 {
                dims = vector.len();
            }
            if vector.len() != dims || dims == 0 {
                continue;
            }
            crate::embed::normalize(&mut vector);
            points.extend_from_slice(&vector);
            sampled.push(id);
        }
    }

    let n = sampled.len();
    if n == 0 {
        return Ok(Discovery { model: Some(model), notes: 0, sampled: 0, k: 0, groups: Vec::new() });
    }
    let k = options.groups.map_or_else(|| default_groups(n), |g| g.clamp(1, MAX_GROUPS).min(n));
    let clusters = kmeans::kmeans(&points, dims, k, SEED, &stop).ok_or_else(|| check().err().unwrap_or(DiscoverError::Cancelled))?;

    let mut filling: Vec<Filling> = (0..k).map(|_| Filling::default()).collect();
    for (i, &g) in clusters.assignments.iter().enumerate() {
        let centre = &clusters.centroids[g * dims..(g + 1) * dims];
        filling[g].add(sampled[i], kmeans::dot(&points[i * dims..(i + 1) * dims], centre));
    }
    drop(points);

    // Every other note in scope joins its nearest centre, in one pass over the stored
    // vectors (front to back, which beats looking each note up); only ids are kept.
    let mut notes = n;
    if scope.len() > chosen.len() {
        let mut after = 0;
        loop {
            check()?;
            let (page, next) = session.with_vault_quiet(generation, |v| v.discover_vector_page(&model, after, VECTOR_PAGE))?;
            let mut ids = Vec::with_capacity(page.len());
            let mut batch = Vec::with_capacity(page.len() * dims);
            for (id, mut vector) in page {
                if vector.len() != dims || scope.binary_search(&id).is_err() || chosen.binary_search(&id).is_ok() {
                    continue;
                }
                crate::embed::normalize(&mut vector);
                batch.extend_from_slice(&vector);
                ids.push(id);
            }
            if !ids.is_empty() {
                let (groups, scores) = kmeans::assign(&batch, dims, &clusters.centroids);
                for ((id, g), score) in ids.into_iter().zip(groups).zip(scores) {
                    filling[g].add(id, score);
                    notes += 1;
                }
            }
            match next {
                Some(a) => after = a,
                None => break,
            }
        }
    }
    drop(scope);

    // Word counts per group from the sampled notes' title and body without frontmatter
    // (what was embedded), read a page at a time; the texts are dropped once counted.
    let group_of: std::collections::HashMap<i64, usize> =
        sampled.iter().zip(&clusters.assignments).map(|(&id, &g)| (id, g)).collect();
    let mut counts = vec![words::GroupWords::default(); k];
    for page in sampled.chunks(PAGE as usize) {
        check()?;
        let texts = session.with_vault_quiet(generation, |v| v.embedding_texts(page))?;
        for note in &texts {
            if let Some(&g) = group_of.get(&note.id) {
                counts[g].add_note(&note.text);
            }
        }
    }
    check()?;
    let top = words::distinctive(&counts, WORDS_PER_GROUP);
    drop(counts);

    let mut groups = Vec::with_capacity(k);
    for (mut f, words) in filling.into_iter().zip(top) {
        if f.members.is_empty() {
            continue;
        }
        check()?;
        f.trim();
        let ids: Vec<i64> = f.nearest.iter().map(|n| n.1).collect();
        let titles = session.with_vault_quiet(generation, |v| v.discover_titles(&ids))?;
        let notes = f
            .nearest
            .iter()
            .filter_map(|&(score, id)| {
                let (_, folder, title) = titles.iter().find(|t| t.0 == id)?;
                Some(GroupNote { id, folder: folder.clone(), title: title.clone(), score })
            })
            .collect();
        f.members.sort_unstable();
        groups.push(Group { words, size: f.members.len(), notes, members: f.members });
    }
    groups.sort_by(|a: &Group, b: &Group| b.size.cmp(&a.size).then(a.members[0].cmp(&b.members[0])));
    Ok(Discovery { model: Some(model), notes, sampled: n, k, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_is_fixed_by_the_seed_and_sorted() {
        let ids: Vec<i64> = (1..=1_000).rev().collect();
        let a = sample(&ids, 100, SEED);
        assert_eq!(a.len(), 100);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        assert!(a.iter().all(|id| ids.contains(id)));
        // The same ids in another order give the same sample; another seed another one.
        let mut shuffled = ids.clone();
        shuffled.sort();
        assert_eq!(sample(&shuffled, 100, SEED), a);
        assert_ne!(sample(&ids, 100, SEED + 1), a);
        // Spread over the whole range, not the first ids.
        assert!(a[0] < 100 && a[99] > 900, "{a:?}");
        // No more ids than asked for: all of them.
        assert_eq!(sample(&[3, 1, 2], 5, SEED), [1, 2, 3]);
    }

    #[test]
    fn default_group_count() {
        assert_eq!(default_groups(0), 0);
        assert_eq!(default_groups(3), 3);
        assert_eq!(default_groups(24), 5);
        assert_eq!(default_groups(200), 10);
        assert_eq!(default_groups(10_000), 50);
    }
}
