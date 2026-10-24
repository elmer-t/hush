//! Obsidian-style link resolution.
//!
//! A link target is `path/title`, where `path` may be empty. Candidates are the notes
//! with that title (case-insensitive). A note whose folder equals the path wins
//! outright; otherwise every note whose folder ends with the path matches, and more
//! than one match makes the link ambiguous. Trashed notes never match.

use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::notes::{NoteSummary, SUMMARY_COLUMNS, derive};
use super::{Result, Vault, now_ms};
use crate::markdown;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "ids", rename_all = "camelCase")]
pub enum Resolution {
    Unique(i64),
    Ambiguous(Vec<i64>),
    Missing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlink {
    pub note: NoteSummary,
    /// The linking note's target also matches other notes.
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingLink {
    pub target: String,
    pub resolution: Resolution,
}

impl Vault {
    /// Resolves link text as written between `[[` and `]]`, without alias or heading.
    pub fn resolve_link(&self, target: &str) -> Result<Resolution> {
        let (path, title) = markdown::split_target(target);
        resolve(&self.conn, &path, &title)
    }

    pub fn backlinks(&self, id: i64) -> Result<Vec<Backlink>> {
        let title: String = self.conn.query_row("SELECT title FROM notes WHERE id = ?1", [id], |r| r.get(0))?;
        let mut stmt = self
            .conn
            .prepare_cached("SELECT DISTINCT from_note_id, target_path FROM links WHERE target_title = ?1")?;
        let rows: Vec<(i64, String)> =
            stmt.query_map([&title], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;

        // from note -> ambiguous; an unambiguous link wins over an ambiguous one.
        let mut sources: BTreeMap<i64, bool> = BTreeMap::new();
        for (from, path) in rows {
            let ambiguous = match resolve(&self.conn, &path, &title)? {
                Resolution::Unique(target) if target == id => false,
                Resolution::Ambiguous(ids) if ids.contains(&id) => true,
                _ => continue,
            };
            let entry = sources.entry(from).or_insert(ambiguous);
            *entry &= ambiguous;
        }

        let mut stmt = self.conn.prepare_cached(&format!("SELECT {SUMMARY_COLUMNS} FROM notes n WHERE n.id = ?1"))?;
        let mut backlinks = sources
            .into_iter()
            .map(|(from, ambiguous)| Ok(Backlink { note: stmt.query_row([from], NoteSummary::from_row)?, ambiguous }))
            .collect::<Result<Vec<_>>>()?;
        backlinks.sort_by_key(|b| b.note.title.to_lowercase());
        Ok(backlinks)
    }

    pub fn outgoing_links(&self, id: i64) -> Result<Vec<OutgoingLink>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT target_path, target_title FROM links WHERE from_note_id = ?1 ORDER BY target_title",
        )?;
        let targets: Vec<(String, String)> =
            stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        targets
            .into_iter()
            .map(|(path, title)| {
                Ok(OutgoingLink { resolution: resolve(&self.conn, &path, &title)?, target: join(&path, &title) })
            })
            .collect()
    }

    /// Links from the given notes that match more than one note, as
    /// (`folder/title` of the linking note, link target), sorted.
    pub fn ambiguous_links(&self, from: &[i64]) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT n.folder, n.title, l.target_path, l.target_title
             FROM links l JOIN notes n ON n.id = l.from_note_id WHERE l.from_note_id = ?1",
        )?;
        let mut out = Vec::new();
        for id in from {
            let rows: Vec<(String, String, String, String)> = stmt
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (folder, title, path, target) in rows {
                if let Resolution::Ambiguous(_) = resolve(&self.conn, &path, &target)? {
                    out.push((join(&folder, &title), join(&path, &target)));
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// The shortest link text that resolves uniquely to the note.
    pub fn link_text(&self, id: i64) -> Result<String> {
        shortest_link(&self.conn, id)
    }
}

pub(super) fn resolve(conn: &Connection, path: &str, title: &str) -> Result<Resolution> {
    let mut stmt = conn.prepare_cached("SELECT id, folder FROM notes WHERE title = ?1 AND deleted_at IS NULL ORDER BY id")?;
    let candidates: Vec<(i64, String)> =
        stmt.query_map([title], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let path = path.to_lowercase();
    if let Some((id, _)) = candidates.iter().find(|(_, folder)| folder.to_lowercase() == path) {
        return Ok(Resolution::Unique(*id));
    }
    let suffix = format!("/{path}");
    let matches: Vec<i64> = candidates
        .into_iter()
        .filter(|(_, folder)| path.is_empty() || folder.to_lowercase().ends_with(&suffix))
        .map(|(id, _)| id)
        .collect();
    Ok(match matches.as_slice() {
        [] => Resolution::Missing,
        [id] => Resolution::Unique(*id),
        _ => Resolution::Ambiguous(matches),
    })
}

fn shortest_link(conn: &Connection, id: i64) -> Result<String> {
    let (folder, title): (String, String) =
        conn.query_row("SELECT folder, title FROM notes WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let segments: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
    for k in 0..=segments.len() {
        let path = segments[segments.len() - k..].join("/");
        if resolve(conn, &path, &title)? == Resolution::Unique(id) {
            return Ok(join(&path, &title));
        }
    }
    // The full path always resolves uniquely: folder equality wins and (folder, title) is unique.
    Ok(join(&folder, &title))
}

fn join(path: &str, title: &str) -> String {
    if path.is_empty() { title.to_string() } else { format!("{path}/{title}") }
}

/// Runs `change` (which renames, moves or creates a note) and then rewrites every link
/// to a note titled one of `titles` that resolved uniquely before but not after, so it
/// points at the same note again via its new shortest link.
pub(super) fn preserving_links<T>(
    conn: &Connection,
    titles: &[&str],
    change: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let mut before = Vec::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT from_note_id, target_path, target_title FROM links WHERE target_title = ?1",
        )?;
        for title in titles {
            for row in stmt.query_map([title], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
                let (from, path, title) = row?;
                if let Resolution::Unique(target) = resolve(conn, &path, &title)? {
                    before.push((from, path, title, target));
                }
            }
        }
    }
    before.dedup();

    let out = change(conn)?;

    let mut rewrites: BTreeMap<i64, Vec<(String, String, String)>> = BTreeMap::new();
    for (from, path, title, target) in before {
        if resolve(conn, &path, &title)? != Resolution::Unique(target) {
            let new_target = shortest_link(conn, target)?;
            rewrites.entry(from).or_default().push((path, title, new_target));
        }
    }
    for (from, subs) in rewrites {
        rewrite_links(conn, from, &subs)?;
    }
    Ok(out)
}

/// Replaces the target text of matching `[[links]]` in one note, keeping headings and aliases.
fn rewrite_links(conn: &Connection, id: i64, subs: &[(String, String, String)]) -> Result<()> {
    let body: String = conn.query_row("SELECT body FROM notes WHERE id = ?1", [id], |r| r.get(0))?;
    let mut new_body = body.clone();
    for link in markdown::parse(&body).links.iter().rev() {
        let sub = subs.iter().find(|(path, title, _)| {
            path.to_lowercase() == link.path.to_lowercase() && title.to_lowercase() == link.title.to_lowercase()
        });
        if let Some((_, _, new_target)) = sub {
            new_body.replace_range(link.target_range.clone(), new_target);
        }
    }
    conn.execute("UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1", params![id, new_body, now_ms()])?;
    derive(conn, id, &new_body)
}
