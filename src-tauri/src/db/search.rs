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
    pub snippet: Option<String>,
}

impl Vault {
    /// With text: ranked by relevance (title matches weigh more). Without: newest first.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let mut args: Vec<Value> = Vec::new();
        let mut filters: Vec<String> = Vec::new();

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
                Ok(SearchHit { note: NoteSummary::from_row(r)?, snippet: r.get(7)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(hits)
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

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::fts_query;

    #[test]
    fn fts_query_quotes_words_and_prefixes_last() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("foo ba").as_deref(), Some(r#""foo" "ba"*"#));
        assert_eq!(fts_query("foo bar ").as_deref(), Some(r#""foo" "bar""#));
        assert_eq!(fts_query(r#"a"b OR c*"#).as_deref(), Some(r#""ab" "OR" "c*"*"#));
    }
}
