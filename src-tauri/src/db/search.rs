//! Full-text search (FTS5, bm25) combined with metadata filters.

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::notes::{NoteSummary, SUMMARY_COLUMNS};
use super::{Result, Vault};
use crate::markdown;

/// Marks the start and end of a match inside [`SearchHit::snippet`]. The frontend turns
/// them into highlight elements after escaping the text.
pub const HIGHLIGHT_START: char = '\u{2}';
pub const HIGHLIGHT_END: char = '\u{3}';

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateField {
    #[default]
    Created,
    Updated,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchQuery {
    pub text: String,
    /// All must match; a tag also matches its nested tags (`journal` matches `journal/2019`).
    pub tags: Vec<String>,
    pub topic: Option<String>,
    pub author: Option<String>,
    /// Matches the folder and its subfolders.
    pub folder: Option<String>,
    pub date_field: DateField,
    /// Unix millis, inclusive.
    pub from: Option<i64>,
    /// Unix millis, exclusive.
    pub to: Option<i64>,
    pub attributes: Vec<(String, String)>,
    /// Notes rated this many stars or more; unrated notes never match.
    pub min_rating: Option<u8>,
    pub limit: Option<u32>,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub note: NoteSummary,
    /// Part of the body around the first match; `None` when the body has no match.
    pub snippet: Option<String>,
}

/// Above this many matches a search lists the newest matches (by creation) instead of
/// ranking them: ranking reads every match, which at 1M notes takes seconds for a
/// common word or a short prefix. Typing more narrows the matches and brings ranking back.
pub const RANK_LIMIT: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// More than [`RANK_LIMIT`] notes matched, so the hits are the newest matches,
    /// not the most relevant.
    pub newest_first: bool,
}

/// The values the search filters can pick from: only those used by at least one note.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub tags: Vec<String>,
    pub topics: Vec<String>,
    pub authors: Vec<String>,
    pub attribute_keys: Vec<String>,
}

impl Vault {
    /// [`Vault::search_page`]'s hits.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        Ok(self.search_page(q)?.hits)
    }

    /// With text: ranked by relevance (title matches weigh more), or newest first when
    /// more than [`RANK_LIMIT`] notes match. Without text: most recently updated first.
    pub fn search_page(&self, q: &SearchQuery) -> Result<SearchPage> {
        Ok(self
            .search_page_until(q, &|| false)?
            .expect("never stopped"))
    }

    /// [`Vault::search_page`], giving up (`None`) as soon as `stop` says so; it is asked
    /// between the steps of the search, so a superseded search ends early.
    pub fn search_page_until(
        &self,
        q: &SearchQuery,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<SearchPage>> {
        let (filters, mut args) = filters(q);

        let min_rating = q.min_rating.filter(|m| *m > 0);

        let limit = i64::from(q.limit.unwrap_or(200));
        let offset = i64::from(q.offset);
        let Some(fts) = fts_query(&q.text) else {
            // Left to itself SQLite walks `notes_updated` newest first and checks each
            // note's rating, which reads the whole vault when few notes are rated. Most
            // notes are unrated, so reading the rated ones and sorting them is the
            // cheaper bet.
            let index = if min_rating.is_some() {
                " INDEXED BY notes_rating"
            } else {
                ""
            };
            let mut sql = format!("SELECT {SUMMARY_COLUMNS} FROM notes n{index} WHERE 1");
            for f in &filters {
                sql.push_str(" AND ");
                sql.push_str(f);
            }
            sql.push_str(" ORDER BY n.updated_at DESC, n.id DESC LIMIT ? OFFSET ?");
            args.push(limit.into());
            args.push(offset.into());
            let mut stmt = self.conn.prepare(&sql)?;
            let hits = stmt
                .query_map(params_from_iter(args), |r| {
                    Ok(SearchHit {
                        note: NoteSummary::from_row(r)?,
                        snippet: None,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            return Ok(Some(SearchPage {
                hits,
                newest_first: false,
            }));
        };

        // Matching ids first, from the full-text index alone where possible: joining
        // every match to `notes` costs more than finding it. Only the trash filter
        // (always there) is answered without the join, from the trashed ids.
        let from = if filters.len() == 1 {
            "FROM notes_fts WHERE notes_fts MATCH ?
             AND notes_fts.rowid NOT IN (SELECT id FROM notes WHERE deleted_at IS NOT NULL)"
                .to_string()
        } else {
            let mut from =
                "FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid WHERE notes_fts MATCH ?"
                    .to_string();
            for f in &filters {
                from.push_str(" AND ");
                from.push_str(f);
            }
            from
        };
        let mut args_with_text = vec![Value::from(fts.clone())];
        args_with_text.extend(args.iter().cloned());

        // Ranking reads every match (and bm25 every list of the query's words), so a
        // query matching more than `RANK_LIMIT` notes lists the newest matches instead.
        let mut probe_args = args_with_text.clone();
        probe_args.push((RANK_LIMIT as i64 + 1).into());
        let matches: i64 = self.conn.query_row(
            &format!("SELECT count(*) FROM (SELECT notes_fts.rowid {from} LIMIT ?)"),
            params_from_iter(probe_args),
            |r| r.get(0),
        )?;
        if stop() {
            return Ok(None);
        }
        let newest_first = matches as usize > RANK_LIMIT;
        let order = if newest_first {
            "notes_fts.rowid DESC"
        } else {
            "bm25(notes_fts, 10.0, 1.0)"
        };
        let mut page_args = args_with_text;
        page_args.push(limit.into());
        page_args.push(offset.into());
        let ids: Vec<i64> = self
            .conn
            .prepare(&format!(
                "SELECT notes_fts.rowid {from} ORDER BY {order} LIMIT ? OFFSET ?"
            ))?
            .query_map(params_from_iter(page_args), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        // Summaries and snippets for this page only.
        let mut snippet = self.conn.prepare_cached(
            "SELECT snippet(notes_fts, 1, char(2), char(3), '…', 16) FROM notes_fts
             WHERE notes_fts MATCH ?1 AND rowid = ?2",
        )?;
        let mut hits = Vec::with_capacity(ids.len());
        if stop() {
            return Ok(None);
        }
        for note in self.note_summaries(&ids)? {
            if stop() {
                return Ok(None);
            }
            let text: Option<String> = snippet
                .query_row(params![fts, note.id], |r| r.get(0))
                .optional()?;
            hits.push(SearchHit {
                snippet: text.and_then(|s| clean_snippet(&s)),
                note,
            });
        }
        Ok(Some(SearchPage { hits, newest_first }))
    }

    /// Up to `limit` ids of notes matching `q` (text and filters; paging and order are
    /// ignored) above `after`, in id order: every match, unranked, for tag discovery's
    /// *Current search results* scope.
    pub fn search_ids(&self, q: &SearchQuery, after: i64, limit: u32) -> Result<Vec<i64>> {
        let (filters, filter_args) = filters(q);
        let mut args: Vec<Value> = Vec::new();
        // The full-text index hands out matches in rowid order and seeks to `after`.
        let mut sql = match fts_query(&q.text) {
            Some(fts) => {
                args.push(fts.into());
                "SELECT n.id FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid
                 WHERE notes_fts MATCH ? AND notes_fts.rowid > ?"
                    .to_string()
            }
            None => "SELECT n.id FROM notes n WHERE n.id > ?".to_string(),
        };
        args.push(after.into());
        args.extend(filter_args);
        for f in &filters {
            sql.push_str(" AND ");
            sql.push_str(f);
        }
        sql.push_str(" ORDER BY n.id LIMIT ?");
        args.push(i64::from(limit).into());
        let mut stmt = self.conn.prepare(&sql)?;
        let ids = stmt
            .query_map(params_from_iter(args), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// The number of notes in `folder` and its subfolders; every note for the root.
    pub fn count_in_folder(&self, folder: &str) -> Result<usize> {
        let folder = markdown::normalize_folder(folder);
        let count: i64 = if folder.is_empty() {
            self.conn.query_row(
                "SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL",
                [],
                |r| r.get(0),
            )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL AND (folder = ?1 OR folder LIKE ?2 ESCAPE '\\')",
                rusqlite::params![folder, format!("{}/%", escape_like(&folder))],
                |r| r.get(0),
            )?
        };
        Ok(count as usize)
    }

    pub fn facets(&self) -> Result<Facets> {
        Ok(Facets {
            tags: self.strings(
                "SELECT t.name FROM tags t
                 WHERE EXISTS (SELECT 1 FROM note_tags nt WHERE nt.tag_id = t.id) ORDER BY t.name COLLATE NOCASE, t.name",
                [],
            )?,
            topics: self.strings("SELECT DISTINCT topic FROM notes WHERE topic IS NOT NULL AND deleted_at IS NULL ORDER BY topic", [])?,
            authors: self.strings("SELECT DISTINCT author FROM notes WHERE author IS NOT NULL AND deleted_at IS NULL ORDER BY author", [])?,
            attribute_keys: self.strings("SELECT DISTINCT key FROM note_attributes ORDER BY key", [])?,
        })
    }

    pub fn attribute_values(&self, key: &str) -> Result<Vec<String>> {
        self.strings(
            "SELECT DISTINCT value FROM note_attributes WHERE key = ?1 ORDER BY value",
            [key],
        )
    }

    fn strings(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt
            .query_map(params, |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// The WHERE conditions of `q`'s filters (on `notes n`, live notes only) and their
/// arguments, in order.
fn filters(q: &SearchQuery) -> (Vec<String>, Vec<Value>) {
    let mut args: Vec<Value> = Vec::new();
    let mut filters: Vec<String> = vec!["n.deleted_at IS NULL".into()];

    for tag in &q.tags {
        filters.push(
            "EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
             WHERE nt.note_id = n.id AND (t.name = ? OR (t.name >= ? AND t.name < ?)))"
                .into(),
        );
        // Tags are case-sensitive, so nested tags are a binary range ('0' follows '/'), not LIKE.
        args.push(tag.trim().to_string().into());
        args.push(format!("{}/", tag.trim()).into());
        args.push(format!("{}0", tag.trim()).into());
    }
    if let Some(topic) = &q.topic {
        filters.push("n.topic = ?".into());
        args.push(topic.clone().into());
    }
    if let Some(author) = &q.author {
        filters.push("n.author = ?".into());
        args.push(author.clone().into());
    }
    if let Some(folder) = &q.folder {
        let folder = markdown::normalize_folder(folder);
        if !folder.is_empty() {
            filters.push("(n.folder = ? OR n.folder LIKE ? ESCAPE '\\')".into());
            let pattern = format!("{}/%", escape_like(&folder));
            args.push(folder.into());
            args.push(pattern.into());
        }
    }
    let date_col = match q.date_field {
        DateField::Created => "n.created_at",
        DateField::Updated => "n.updated_at",
    };
    if let Some(from) = q.from {
        filters.push(format!("{date_col} >= ?"));
        args.push(from.into());
    }
    if let Some(to) = q.to {
        filters.push(format!("{date_col} < ?"));
        args.push(to.into());
    }
    for (key, value) in &q.attributes {
        filters.push(
            "EXISTS (SELECT 1 FROM note_attributes a WHERE a.note_id = n.id AND a.key = ? AND a.value = ?)".into(),
        );
        args.push(key.clone().into());
        args.push(value.clone().into());
    }
    if let Some(min) = q.min_rating.filter(|m| *m > 0) {
        // Implies `rating IS NOT NULL`, so the partial index on rating applies.
        filters.push("n.rating >= ?".into());
        args.push(i64::from(min).into());
    }
    (filters, args)
}

/// Turns user input into a safe FTS5 query: every word is a quoted phrase (so FTS
/// syntax characters are literal), all words must match, and the last word is a
/// prefix while the user is still typing it, from two characters on (a one-letter
/// prefix matches nearly every note, and the prefix index starts at two).
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .collect();
    let last = words.len().checked_sub(1)?;
    let typing = !text.ends_with(char::is_whitespace);
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == last && typing && w.chars().count() >= 2 {
                    format!("\"{w}\"*")
                } else {
                    format!("\"{w}\"")
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// `None` unless the snippet contains a match. Text before the match up to a `---`
/// (the end of the frontmatter, usually) is dropped.
fn clean_snippet(snippet: &str) -> Option<String> {
    let first = snippet.find(HIGHLIGHT_START)?;
    let start = snippet[..first].rfind("---").map_or(0, |i| i + 3);
    Some(snippet[start..].trim_start().to_string())
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::{clean_snippet, fts_query};

    #[test]
    fn snippets_need_a_match_and_skip_frontmatter() {
        assert_eq!(clean_snippet("no match"), None);
        assert_eq!(
            clean_snippet("--- created: 2021 --- Budget \u{2}review\u{3} --- end").as_deref(),
            Some("Budget \u{2}review\u{3} --- end")
        );
        assert_eq!(
            clean_snippet("\u{2}a\u{3} b").as_deref(),
            Some("\u{2}a\u{3} b")
        );
    }

    #[test]
    fn fts_query_quotes_words_and_prefixes_last() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("foo ba").as_deref(), Some(r#""foo" "ba"*"#));
        assert_eq!(
            fts_query("foo b").as_deref(),
            Some(r#""foo" "b""#),
            "one letter is a whole word"
        );
        assert_eq!(fts_query("é").as_deref(), Some(r#""é""#));
        assert_eq!(fts_query("foo bar ").as_deref(), Some(r#""foo" "bar""#));
        assert_eq!(
            fts_query(r#"a"b OR c*"#).as_deref(),
            Some(r#""ab" "OR" "c*"*"#)
        );
    }
}
