//! Why notes are related: the reasons the Related panel shows next to the meaning score.
//! They are worked out only for the few notes shown, with one query for the links of
//! all of them, one for the links to all of them and one for their tags, never one per
//! pair.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::params_from_iter;
use serde::Serialize;

use super::links::{Resolution, resolve};
use super::metadata::TitledNote;
use super::{Result, Vault};

/// Most notes listed per kind of two-step link.
pub const MAX_VIA: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reasons {
    /// The open note links to this one.
    pub links_to: bool,
    /// This note links to the open note.
    pub linked_from: bool,
    /// Notes both link to, by title (at most [`MAX_VIA`]).
    pub both_link_to: Vec<TitledNote>,
    /// Notes that link to both.
    pub both_linked_from: Vec<TitledNote>,
    /// Notes on a two-step path between them: the open note links to one that links to
    /// this one, or the other way round.
    pub via: Vec<TitledNote>,
    pub shared_tags: Vec<String>,
    pub same_folder: bool,
    pub same_author: Option<String>,
    pub same_topic: Option<String>,
}

impl Vault {
    /// The reasons each of `others` relates to note `id`, in the order of `others`.
    /// Only links that resolve to exactly one note count.
    pub fn relation_reasons(&self, id: i64, others: &[i64]) -> Result<Vec<Reasons>> {
        let mut all = Vec::with_capacity(others.len() + 1);
        all.push(id);
        all.extend(others.iter().copied().filter(|&o| o != id));
        let notes: HashMap<i64, super::NoteSummary> = self
            .note_summaries(&all)?
            .into_iter()
            .map(|n| (n.id, n))
            .collect();
        let Some(open) = notes.get(&id) else {
            return Ok(vec![Reasons::default(); others.len()]);
        };
        let marks = placeholders(all.len());
        let mut resolved: HashMap<(String, String), Option<i64>> = HashMap::new();
        let mut unique = |path: String, title: String| -> Result<Option<i64>> {
            if let Some(&known) = resolved.get(&(path.clone(), title.clone())) {
                return Ok(known);
            }
            let target = match resolve(&self.conn, &path, &title)? {
                Resolution::Unique(t) => Some(t),
                _ => None,
            };
            resolved.insert((path, title), target);
            Ok(target)
        };

        // Where each note links to.
        let mut out: HashMap<i64, HashSet<i64>> = HashMap::new();
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT DISTINCT from_note_id, target_path, target_title FROM links WHERE from_note_id IN ({marks})"))?;
        let rows: Vec<(i64, String, String)> = stmt
            .query_map(params_from_iter(&all), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (from, path, title) in rows {
            if let Some(to) = unique(path, title)? {
                if to != from {
                    out.entry(from).or_default().insert(to);
                }
            }
        }

        // What links to each note. Links can only mean a note with the same title.
        let titles: Vec<&str> = all
            .iter()
            .filter_map(|n| notes.get(n))
            .map(|n| n.title.as_str())
            .collect();
        let mut back: HashMap<i64, HashSet<i64>> = HashMap::new();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT from_note_id, target_path, target_title FROM links WHERE target_title IN ({})",
            placeholders(titles.len())
        ))?;
        let rows: Vec<(i64, String, String)> = stmt
            .query_map(params_from_iter(&titles), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (from, path, title) in rows {
            if let Some(to) = unique(path, title)? {
                if to != from && notes.contains_key(&to) {
                    back.entry(to).or_default().insert(from);
                }
            }
        }

        let mut stmt = self.conn.prepare(&format!(
            "SELECT nt.note_id, t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id IN ({marks})"
        ))?;
        let mut tags: HashMap<i64, BTreeSet<String>> = HashMap::new();
        for row in stmt.query_map(params_from_iter(&all), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (note, tag) = row?;
            tags.entry(note).or_default().insert(tag);
        }

        let empty = HashSet::new();
        let none = BTreeSet::new();
        let (open_out, open_back) = (
            out.get(&id).unwrap_or(&empty),
            back.get(&id).unwrap_or(&empty),
        );
        let open_tags = tags.get(&id).unwrap_or(&none);
        let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(a), Some(b)) if !a.trim().is_empty() && a.eq_ignore_ascii_case(b) => {
                Some(b.clone())
            }
            _ => None,
        };
        others
            .iter()
            .map(|&other| {
                let Some(note) = notes.get(&other).filter(|_| other != id) else {
                    return Ok(Reasons::default());
                };
                let (other_out, other_back) = (
                    out.get(&other).unwrap_or(&empty),
                    back.get(&other).unwrap_or(&empty),
                );
                let pair = |x: &i64| *x != id && *x != other;
                let both_link_to: Vec<i64> = open_out
                    .intersection(other_out)
                    .copied()
                    .filter(pair)
                    .collect();
                let both_linked_from: Vec<i64> = open_back
                    .intersection(other_back)
                    .copied()
                    .filter(pair)
                    .collect();
                let via: Vec<i64> = open_out
                    .intersection(other_back)
                    .chain(other_out.intersection(open_back))
                    .copied()
                    .filter(pair)
                    .collect();
                Ok(Reasons {
                    links_to: open_out.contains(&other),
                    linked_from: other_out.contains(&id),
                    both_link_to: self.titled(&both_link_to)?,
                    both_linked_from: self.titled(&both_linked_from)?,
                    via: self.titled(&via)?,
                    shared_tags: open_tags
                        .intersection(tags.get(&other).unwrap_or(&none))
                        .cloned()
                        .collect(),
                    same_folder: open.folder.eq_ignore_ascii_case(&note.folder),
                    same_author: same(&open.author, &note.author),
                    same_topic: same(&open.topic, &note.topic),
                })
            })
            .collect()
    }

    /// Up to [`MAX_VIA`] of the live notes among `ids`, sorted by title, without repeats.
    fn titled(&self, ids: &[i64]) -> Result<Vec<TitledNote>> {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let mut notes: Vec<TitledNote> = self
            .note_summaries(&ids)?
            .into_iter()
            .map(|n| TitledNote {
                id: n.id,
                folder: n.folder,
                title: n.title,
            })
            .collect();
        notes.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        notes.truncate(MAX_VIA);
        Ok(notes)
    }
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}
