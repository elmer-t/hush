//! Full-text search (FTS5, bm25) combined with metadata filters.

use rusqlite::params_from_iter;
use rusqlite::types::Value;
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
    /// With text: ranked by relevance (title matches weigh more). Without: newest first.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let mut args: Vec<Value> = Vec::new();
        let mut filters: Vec<String> = vec!["n.deleted_at IS NULL".into()];

        for tag in &q.tags {
            filters.push(
                "EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
                 WHERE nt.note_id = n.id AND (t.name = ? OR t.name LIKE ? ESCAPE '\\'))"
                    .into(),
            );
            args.push(tag.trim().to_string().into());
            args.push(format!("{}/%", escape_like(tag.trim())).into());
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

        let fts = fts_query(&q.text);
        let ranked = fts.is_some();
        let mut sql = match &fts {
            Some(_) => format!(
                "SELECT {SUMMARY_COLUMNS}, snippet(notes_fts, 1, char(2), char(3), '…', 16)
                 FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid
                 WHERE notes_fts MATCH ?"
            ),
            None => format!("SELECT {SUMMARY_COLUMNS}, NULL FROM notes n WHERE 1"),
        };
        if let Some(fts) = fts {
            args.insert(0, fts.into());
        }
        for f in &filters {
            sql.push_str(" AND ");
            sql.push_str(f);
        }
        sql.push_str(if ranked { " ORDER BY bm25(notes_fts, 10.0, 1.0)" } else { " ORDER BY n.updated_at DESC, n.id DESC" });
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push(i64::from(q.limit.unwrap_or(200)).into());
        args.push(i64::from(q.offset).into());

        let mut stmt = self.conn.prepare(&sql)?;
        let hits = stmt
            .query_map(params_from_iter(args), |r| {
                let snippet: Option<String> = r.get(7)?;
                Ok(SearchHit {
                    note: NoteSummary::from_row(r)?,
                    snippet: snippet.and_then(|s| clean_snippet(&s)),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(hits)
    }

    pub fn facets(&self) -> Result<Facets> {
        Ok(Facets {
            tags: self.strings(
                "SELECT t.name FROM tags t
                 WHERE EXISTS (SELECT 1 FROM note_tags nt WHERE nt.tag_id = t.id) ORDER BY t.name",
                [],
            )?,
            topics: self.strings("SELECT DISTINCT topic FROM notes WHERE topic IS NOT NULL AND deleted_at IS NULL ORDER BY topic", [])?,
            authors: self.strings("SELECT DISTINCT author FROM notes WHERE author IS NOT NULL AND deleted_at IS NULL ORDER BY author", [])?,
            attribute_keys: self.strings("SELECT DISTINCT key FROM note_attributes ORDER BY key", [])?,
        })
    }

    pub fn attribute_values(&self, key: &str) -> Result<Vec<String>> {
        self.strings("SELECT DISTINCT value FROM note_attributes WHERE key = ?1 ORDER BY value", [key])
    }

    fn strings(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params, |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// Turns user input into a safe FTS5 query: every word is a quoted phrase (so FTS
/// syntax characters are literal), all words must match, and the last word is a
/// prefix while the user is still typing it.
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
            .map(|(i, w)| if i == last && typing { format!("\"{w}\"*") } else { format!("\"{w}\"") })
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
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
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
        assert_eq!(clean_snippet("\u{2}a\u{3} b").as_deref(), Some("\u{2}a\u{3} b"));
    }

    #[test]
    fn fts_query_quotes_words_and_prefixes_last() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("foo ba").as_deref(), Some(r#""foo" "ba"*"#));
        assert_eq!(fts_query("foo bar ").as_deref(), Some(r#""foo" "bar""#));
        assert_eq!(fts_query(r#"a"b OR c*"#).as_deref(), Some(r#""ab" "OR" "c*"*"#));
    }
}
