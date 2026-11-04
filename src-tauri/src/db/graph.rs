//! The link and tag queries the graph view is built from (see `crate::graph`). Each
//! works on a given set of notes through the indexes on `links` and `note_tags`, never
//! on the whole vault.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, params_from_iter};
use serde::Deserialize;

use super::links::{Resolution, resolve};
use super::{Result, SearchQuery, Vault};
use crate::markdown;

/// The notes of a global graph.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GraphScopeNotes {
    All,
    /// A folder and its subfolders; `""` is the whole vault.
    Folder { folder: String },
    /// Notes with the tag or a tag nested in it (`journal` takes `journal/2019`).
    Tag { tag: String },
    /// The notes matching a search (text and filters).
    Search { query: SearchQuery },
}

/// Ids read per page of a whole-vault or search scope.
const SCOPE_PAGE: u32 = 50_000;

/// Where a link points, once resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LinkTarget {
    Note(i64),
    /// No note matches; the target as written (`path/Title`).
    Missing(String),
}

/// Rows per `IN (...)` list, well under SQLite's variable limit.
const CHUNK: usize = 500;

/// Resolutions of link targets, kept for one graph so each distinct target is
/// resolved once.
#[derive(Default)]
pub struct Resolver(HashMap<(String, String), Resolution>);

impl Resolver {
    fn resolve(&mut self, vault: &Vault, path: String, title: String) -> Result<&Resolution> {
        let key = (path.to_lowercase(), title.to_lowercase());
        if !self.0.contains_key(&key) {
            let r = resolve(&vault.conn, &path, &title)?;
            self.0.insert(key.clone(), r);
        }
        Ok(&self.0[&key])
    }
}

impl Vault {
    /// Up to [`SCOPE_PAGE`] ids of the live notes of `scope` above `after`, in id order;
    /// a folder or tag scope comes in one page (`after` is ignored). Folder and tag
    /// read only their index range, never every note.
    pub fn graph_scope_page(&self, scope: &GraphScopeNotes, after: i64) -> Result<(Vec<i64>, Option<i64>)> {
        let ids: Vec<i64> = match scope {
            GraphScopeNotes::Search { query } => self.search_ids(query, after, SCOPE_PAGE)?,
            GraphScopeNotes::All => self
                .conn
                .prepare_cached("SELECT id FROM notes WHERE id > ?1 AND deleted_at IS NULL ORDER BY id LIMIT ?2")?
                .query_map(params![after, SCOPE_PAGE], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?,
            GraphScopeNotes::Folder { folder } => {
                let folder = markdown::normalize_folder(folder);
                if folder.is_empty() {
                    return self.graph_scope_page(&GraphScopeNotes::All, after);
                }
                // The folder itself, then its subfolders as a range ('0' follows '/'):
                // two seeks on the index of live notes.
                let mut ids: Vec<i64> = self
                    .conn
                    .prepare_cached("SELECT id FROM notes INDEXED BY notes_folder_title WHERE folder = ?1 AND deleted_at IS NULL")?
                    .query_map([&folder], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let below = self.conn.prepare_cached(
                    "SELECT id FROM notes INDEXED BY notes_folder_title
                     WHERE folder >= ?1 AND folder < ?2 AND deleted_at IS NULL",
                )?
                .query_map(params![format!("{folder}/"), format!("{folder}0")], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
                ids.extend(below);
                ids.sort_unstable();
                return Ok((ids, None));
            }
            GraphScopeNotes::Tag { tag } => {
                let tag = tag.trim();
                let mut ids: Vec<i64> = self
                    .conn
                    // Trashed notes have no tag rows, so no `deleted_at` check is needed.
                    .prepare_cached(
                        "SELECT DISTINCT note_id FROM note_tags
                         WHERE tag_id IN (SELECT id FROM tags WHERE name = ?1 OR (name >= ?2 AND name < ?3))",
                    )?
                    .query_map(params![tag, format!("{tag}/"), format!("{tag}0")], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                ids.sort_unstable();
                return Ok((ids, None));
            }
        };
        let next = (ids.len() == SCOPE_PAGE as usize).then(|| *ids.last().unwrap());
        Ok((ids, next))
    }

    /// The links from each of `ids`, resolved: `(from, target)` without repeats, sorted.
    /// Ambiguous links are left out; a self-link is kept.
    pub fn graph_links_from(&self, ids: &[i64], resolver: &mut Resolver) -> Result<Vec<(i64, LinkTarget)>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(CHUNK) {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT DISTINCT from_note_id, target_path, target_title FROM links WHERE from_note_id IN ({})",
                marks(chunk.len())
            ))?;
            let rows: Vec<(i64, String, String)> = stmt
                .query_map(params_from_iter(chunk), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (from, path, title) in rows {
                let joined = if path.is_empty() { title.clone() } else { format!("{path}/{title}") };
                match resolver.resolve(self, path, title)? {
                    Resolution::Unique(to) => out.push((from, LinkTarget::Note(*to))),
                    Resolution::Missing => out.push((from, LinkTarget::Missing(joined))),
                    Resolution::Ambiguous(_) => {}
                }
            }
        }
        out.sort();
        // Missing targets that differ only in case name the same note-to-be.
        let mut seen = HashSet::new();
        out.retain(|(from, to)| {
            seen.insert((*from, match to {
                LinkTarget::Note(id) => id.to_string(),
                LinkTarget::Missing(target) => format!("?{}", target.to_lowercase()),
            }))
        });
        Ok(out)
    }

    /// The notes with a link that resolves to exactly `id`, sorted.
    pub fn graph_links_to(&self, id: i64, resolver: &mut Resolver) -> Result<Vec<i64>> {
        let title: String = self.conn.query_row("SELECT title FROM notes WHERE id = ?1", [id], |r| r.get(0))?;
        let mut stmt =
            self.conn.prepare_cached("SELECT DISTINCT from_note_id, target_path FROM links WHERE target_title = ?1")?;
        let rows: Vec<(i64, String)> =
            stmt.query_map([&title], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut from = Vec::new();
        for (source, path) in rows {
            if *resolver.resolve(self, path, title.clone())? == Resolution::Unique(id) {
                from.push(source);
            }
        }
        from.sort_unstable();
        from.dedup();
        Ok(from)
    }

    /// How many links each of `ids` has, out (rows in `links`) plus in (rows whose
    /// target has the note's title, resolved or not): a cheap measure of how linked a
    /// note is, from two index counts, for picking the most-linked notes of a scope.
    pub fn graph_link_counts(&self, ids: &[i64]) -> Result<Vec<u32>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT (SELECT COUNT(*) FROM links WHERE from_note_id = n.id)
                  + (SELECT COUNT(*) FROM links WHERE target_title = n.title)
             FROM notes n WHERE n.id = ?1",
        )?;
        ids.iter().map(|id| Ok(stmt.query_row([id], |r| r.get(0))?)).collect()
    }

    /// The tags of each of `ids` (those with any), each list sorted.
    pub fn graph_tags(&self, ids: &[i64]) -> Result<BTreeMap<i64, Vec<String>>> {
        let mut tags: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for chunk in ids.chunks(CHUNK) {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT nt.note_id, t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id IN ({})",
                marks(chunk.len())
            ))?;
            for row in stmt.query_map(params_from_iter(chunk), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (note, tag) = row?;
                tags.entry(note).or_default().push(tag);
            }
        }
        for list in tags.values_mut() {
            list.sort();
        }
        Ok(tags)
    }
}

impl Vault {
    /// Of `tags`, those that only one live note has. Each lookup stops at the second
    /// note on the `note_tags_tag` index, so a tag on 500,000 notes costs what one on
    /// two does.
    pub fn rare_tags(&self, tags: &[&str]) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT count(*) FROM (SELECT 1 FROM note_tags WHERE tag_id = (SELECT id FROM tags WHERE name = ?1) LIMIT 2)",
        )?;
        let mut rare = Vec::new();
        for tag in tags {
            if stmt.query_row([tag], |r| r.get::<_, i64>(0))? < 2 {
                rare.push(tag.to_string());
            }
        }
        Ok(rare)
    }
}

fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}
