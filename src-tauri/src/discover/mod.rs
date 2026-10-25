//! Tag discovery: groups notes by meaning (k-means on their vectors) and describes
//! each group by its notes nearest the centre and its distinctive words, so the user
//! can see which tags the archive needs and create one from a group.
//!
//! Nothing is stored: a run reads vectors and texts, computes in memory and returns
//! the groups. Like the embedding job it reads the vault only through
//! [`Session::with_vault_quiet`], in short pages, never while computing, and stops
//! when the vault locks.

pub mod kmeans;
pub mod words;

use serde::{Deserialize, Serialize};

use crate::db;
use crate::session::{Session, SessionError};

/// The fixed seed, so the same notes always give the same groups.
pub const SEED: u64 = 0x4855_5348; // "HUSH"
/// Fewest and most groups the default picks.
pub const DEFAULT_MIN_GROUPS: usize = 5;
pub const DEFAULT_MAX_GROUPS: usize = 50;
/// Most groups the user may ask for.
pub const MAX_GROUPS: usize = 200;
/// Titles shown as a group's sample, and distinctive words per group.
pub const SAMPLE_TITLES: usize = 8;
pub const WORDS_PER_GROUP: usize = 8;
/// Notes read per vault access.
const PAGE: u32 = 500;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Number of groups; `None` picks [`default_groups`].
    pub groups: Option<usize>,
    /// Only notes without any tag (frontmatter or inline), to find what no tag covers.
    pub untagged_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    /// The model whose vectors were grouped; `None` while no model is ready yet.
    pub model: Option<String>,
    /// Notes grouped.
    pub notes: usize,
    /// Groups asked for (the default, or the user's number capped at `notes`).
    pub k: usize,
    /// Largest first.
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Distinctive words, best first.
    pub words: Vec<String>,
    /// Every note in the group, nearest the centre first; the first
    /// [`SAMPLE_TITLES`] are its sample.
    pub notes: Vec<GroupNote>,
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

/// Groups the live notes of the vault of unlock `generation` that have a vector from
/// the ready model.
pub fn run(
    session: &Session,
    generation: u64,
    options: &Options,
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
        return Ok(Discovery { model: None, notes: 0, k: 0, groups: Vec::new() });
    };

    // Vectors, in pages. Every vector is normalised again, so cosine is a dot product,
    // and one of another length (a stray row) is left out.
    let mut notes: Vec<(i64, String, String)> = Vec::new();
    let mut points: Vec<f32> = Vec::new();
    let mut dims = 0;
    let mut after = 0;
    loop {
        check()?;
        let (page, next) =
            session.with_vault_quiet(generation, |v| v.discover_vectors(&model, options.untagged_only, after, PAGE))?;
        for (id, folder, title, mut vector) in page {
            if dims == 0 {
                dims = vector.len();
            }
            if vector.len() != dims || dims == 0 {
                continue;
            }
            crate::embed::normalize(&mut vector);
            points.extend_from_slice(&vector);
            notes.push((id, folder, title));
        }
        match next {
            Some(n) => after = n,
            None => break,
        }
    }

    let n = notes.len();
    let k = options.groups.map_or_else(|| default_groups(n), |g| g.clamp(1, MAX_GROUPS).min(n));
    let clusters = kmeans::kmeans(&points, dims, k, SEED, &stop).ok_or_else(|| check().err().unwrap_or(DiscoverError::Cancelled))?;

    // Members of each group, nearest the centre first.
    let mut members: Vec<Vec<(usize, f32)>> = vec![Vec::new(); k];
    for (i, &g) in clusters.assignments.iter().enumerate() {
        let centre = &clusters.centroids[g * dims..(g + 1) * dims];
        members[g].push((i, kmeans::dot(&points[i * dims..(i + 1) * dims], centre)));
    }
    drop(points);
    for m in &mut members {
        m.sort_by(|a, b| b.1.total_cmp(&a.1).then(notes[a.0].0.cmp(&notes[b.0].0)));
    }

    // Word counts per group, from title and body without frontmatter (what was
    // embedded), read a page at a time; the texts are dropped as soon as they're counted.
    let mut group_of = std::collections::HashMap::with_capacity(n);
    for (i, &g) in clusters.assignments.iter().enumerate() {
        group_of.insert(notes[i].0, g);
    }
    let ids: Vec<i64> = notes.iter().map(|n| n.0).collect();
    let mut counts = vec![words::GroupWords::default(); k];
    for page in ids.chunks(PAGE as usize) {
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

    let mut groups: Vec<Group> = members
        .into_iter()
        .zip(top)
        .filter(|(m, _)| !m.is_empty())
        .map(|(m, words)| Group {
            words,
            notes: m
                .into_iter()
                .map(|(i, score)| {
                    let (id, folder, title) = &notes[i];
                    GroupNote { id: *id, folder: folder.clone(), title: title.clone(), score }
                })
                .collect(),
        })
        .collect();
    groups.sort_by(|a, b| b.notes.len().cmp(&a.notes.len()).then(a.notes[0].id.cmp(&b.notes[0].id)));
    Ok(Discovery { model: Some(model), notes: n, k, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_group_count() {
        assert_eq!(default_groups(0), 0);
        assert_eq!(default_groups(3), 3);
        assert_eq!(default_groups(24), 5);
        assert_eq!(default_groups(200), 10);
        assert_eq!(default_groups(10_000), 50);
    }
}
