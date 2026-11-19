//! Full-text search (FTS5, bm25) combined with metadata filters.

use rusqlite::{OptionalExtension, params, params_from_iter};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use super::notes::{NoteSummary, ReadStatus, SUMMARY_COLUMNS, subtree_params};
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

/// How the sidebar search finds notes (#146).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMode {
    /// By the words in the notes (full-text search).
    #[default]
    Words,
    /// By meaning and words fused (see `meaning`). Only the app's command runs it:
    /// [`Vault::search_page`] treats it as `Words`.
    Meaning,
}

/// The search's read status filter. `NotFinished` is unread or partly read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadFilter {
    Unread,
    PartlyRead,
    Read,
    NotFinished,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchQuery {
    pub text: String,
    pub mode: SearchMode,
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
    /// `None` for any read status.
    pub read_status: Option<ReadFilter>,
    pub limit: Option<u32>,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub note: NoteSummary,
    /// Part of the body around the first match; `None` when the body has no match.
    /// A hit found by meaning alone has the start of its body instead.
    pub snippet: Option<String>,
    /// Why a meaning search found the note; `None` in words mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<Why>,
}

/// Which side of a meaning search found a note (see `meaning::fuse`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Why {
    /// Its words match the query.
    pub words: bool,
    /// Its vector is close to the query's: the cosine.
    pub cosine: Option<f32>,
}

/// Above this many matches a search lists the newest matches (by creation) instead of
/// ranking them: ranking reads every match, which at 1M notes takes seconds for a
/// common word or a short prefix. Typing more narrows the matches and brings ranking back.
pub const RANK_LIMIT: usize = 10_000;

/// The last typed word matches word starts once it has this many characters; a shorter
/// one matches the whole word only. The full-text index has no prefix indexes (they were
/// more than half of it), so a word start merges the lists of every word that starts
/// with it: affordable from three letters, too many words for one or two.
pub const PREFIX_MIN_CHARS: usize = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// More than [`RANK_LIMIT`] notes matched, so the hits are the newest matches,
    /// not the most relevant.
    pub newest_first: bool,
    /// A meaning search used the note vectors (else it found notes by their words).
    #[serde(default)]
    pub by_meaning: bool,
    /// The fused list of a meaning search was cut at this many notes.
    pub top: Option<usize>,
    /// Why a meaning search found notes by their words only.
    pub words_only: Option<String>,
}

impl SearchPage {
    /// A page of a words search.
    pub fn words(hits: Vec<SearchHit>, newest_first: bool) -> Self {
        Self { hits, newest_first, by_meaning: false, top: None, words_only: None }
    }
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
        Ok(self.search_page_until(q, &|| false)?.expect("never stopped"))
    }

    /// [`Vault::search_page`], giving up (`None`) as soon as `stop` says so; it is asked
    /// between the steps of the search, so a superseded search ends early.
    pub fn search_page_until(&self, q: &SearchQuery, stop: &dyn Fn() -> bool) -> Result<Option<SearchPage>> {
        let (filters, mut args) = filters(q);

        let limit = i64::from(q.limit.unwrap_or(200));
        let offset = i64::from(q.offset);
        let Some(fts) = fts_query(&q.text) else {
            let sql = browse_sql(q, &filters, self.browse_by_folder(q, limit + offset)?);
            args.push(limit.into());
            args.push(offset.into());
            let mut stmt = self.conn.prepare(&sql)?;
            let hits = stmt
                .query_map(params_from_iter(args), |r| Ok(SearchHit { note: NoteSummary::from_row(r)?, snippet: None, why: None }))?
                .collect::<rusqlite::Result<_>>()?;
            return Ok(Some(SearchPage::words(hits, false)));
        };
        let Some(ranked) = self.word_ranking(q, &fts, &filters, &args, limit, offset, stop)? else { return Ok(None) };
        let Some(hits) = self.word_hits(&fts, &ranked, stop)? else { return Ok(None) };
        Ok(Some(SearchPage::words(hits, ranked.newest_first)))
    }

    /// The full-text matches of `q` (whose text must have words) from `offset`, at most
    /// `limit`, best first (or newest first above [`RANK_LIMIT`] matches), and whether
    /// they are newest first: the words side of a meaning search. `None` when `stop`
    /// said so.
    pub fn search_word_ids(&self, q: &SearchQuery, limit: u32, stop: &dyn Fn() -> bool) -> Result<Option<(Vec<i64>, bool)>> {
        let Some(fts) = fts_query(&q.text) else { return Ok(Some((Vec::new(), false))) };
        let (filters, args) = filters(q);
        Ok(self
            .word_ranking(q, &fts, &filters, &args, i64::from(limit), 0, stop)?
            .map(|r| (r.rows.iter().map(|(id, _)| *id).collect(), r.newest_first)))
    }

    /// The notes passing `q`'s filters and folder (not its text), for a meaning search
    /// to score only those; `None` when `q` has no filter and no folder, so every live
    /// note passes. Trashed notes may be in an [`NoteFilter::Except`] list or not; the
    /// meaning search drops them anyway.
    pub fn search_filter_ids(&self, q: &SearchQuery) -> Result<Option<NoteFilter>> {
        // "Unread" and "Not finished" pass nearly every note, and `read_status` comes
        // after the body in each row, so checking it row by row reads the whole vault.
        // The few notes they exclude are read from the partial index instead.
        let excluded = match q.read_status {
            Some(ReadFilter::Unread) => Some("SELECT id FROM notes WHERE read_status > 0"),
            Some(ReadFilter::NotFinished) => Some("SELECT id FROM notes WHERE read_status > 0 AND read_status = 2"),
            _ => None,
        };
        let (filters, args) = if excluded.is_some() {
            filters(&SearchQuery { read_status: None, ..q.clone() })
        } else {
            filters(q)
        };
        if filters.len() == 1 && excluded.is_none() {
            return Ok(None);
        }
        let excluded: Option<Vec<i64>> = match excluded {
            Some(sql) => Some(self.conn.prepare(sql)?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?),
            None => None,
        };
        if filters.len() == 1 {
            // Only the read status: every note but the excluded ones, without listing them.
            return Ok(excluded.map(NoteFilter::Except));
        }
        let mut sql = "SELECT n.id FROM notes n WHERE 1".to_string();
        for f in &filters {
            sql.push_str(" AND ");
            sql.push_str(f);
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let mut ids: Vec<i64> = stmt.query_map(params_from_iter(args), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        if let Some(excluded) = excluded {
            let excluded: std::collections::HashSet<i64> = excluded.into_iter().collect();
            ids.retain(|id| !excluded.contains(id));
        }
        Ok(Some(NoteFilter::Only(ids)))
    }

    /// The search hits of a meaning search's fused notes (one page of them), in order:
    /// a words hit with its snippet as in a words search, a hit by meaning alone with
    /// the start of its body. Notes trashed meanwhile are left out. `None` when `stop`
    /// said so.
    pub fn meaning_hits(&self, q: &SearchQuery, fused: &[crate::meaning::FusedHit], stop: &dyn Fn() -> bool) -> Result<Option<Vec<SearchHit>>> {
        let fts = fts_query(&q.text);
        let phrase = phrase_query(&q.text);
        let ids: Vec<i64> = fused.iter().map(|h| h.id).collect();
        // A phrase match shows the phrase, as in a words search; the notes without one
        // show the words.
        let mut snippets = std::collections::HashMap::new();
        let mut wanted: Vec<i64> = fused.iter().filter(|h| h.words).map(|h| h.id).collect();
        for matching in phrase.iter().chain(fts.iter()) {
            if stop() {
                return Ok(None);
            }
            snippets.extend(self.snippets(matching, &wanted)?);
            wanted.retain(|id| !snippets.contains_key(id));
        }
        let mut body = self.conn.prepare_cached("SELECT substr(body, 1, 4000) FROM notes WHERE id = ?1")?;
        let mut hits = Vec::with_capacity(ids.len());
        for (note, why) in self.note_summaries(&ids)?.into_iter().filter_map(|n| {
            let h = fused.iter().find(|h| h.id == n.id)?;
            Some((n, Why { words: h.words, cosine: h.cosine }))
        }) {
            if stop() {
                return Ok(None);
            }
            let mut text = snippets.remove(&note.id);
            if text.is_none() {
                let start: Option<String> = body.query_row([note.id], |r| r.get(0)).optional()?;
                text = start.map(|b| super::review::excerpt(&b)).filter(|e| !e.is_empty());
            }
            hits.push(SearchHit { note, snippet: text, why: Some(why) });
        }
        Ok(Some(hits))
    }

    /// The full-text ranking of `fts` with `filters` (from [`filters`]): one page of
    /// note ids with whether each matched as a phrase.
    #[allow(clippy::too_many_arguments)]
    fn word_ranking(
        &self,
        q: &SearchQuery,
        fts: &str,
        filters: &[String],
        args: &[Value],
        limit: i64,
        offset: i64,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<WordRanking>> {

        // Matching ids first, from the full-text index alone where possible: joining
        // every match to `notes` costs more than finding it. Only the trash filter
        // (always there) is answered without the join, from the trashed ids.
        let from = if filters.len() == 1 {
            "FROM notes_fts WHERE notes_fts MATCH ?
             AND notes_fts.rowid NOT IN (SELECT id FROM notes WHERE deleted_at IS NOT NULL)"
                .to_string()
        } else {
            let mut from = "FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid WHERE notes_fts MATCH ?".to_string();
            for f in filters {
                from.push_str(" AND ");
                from.push_str(f);
            }
            from
        };
        let mut args_with_text = vec![Value::from(fts.to_string())];
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
        let order = if newest_first { "notes_fts.rowid DESC" } else { "bm25(notes_fts, 10.0, 1.0)" };
        // Several unquoted words: when ranking, notes with the words as a phrase come
        // first. The phrase's matches are read once, into a set; above `RANK_LIMIT` the
        // newest matches are listed as before, without reading them.
        let phrase = phrase_query(&q.text).filter(|_| !newest_first);
        let (is_phrase, order) = match &phrase {
            Some(_) => (
                "notes_fts.rowid IN (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?)",
                format!("2 DESC, {order}"),
            ),
            None => ("0", order.to_string()),
        };
        let mut page_args: Vec<Value> = phrase.iter().map(|p| Value::from(p.clone())).collect();
        page_args.extend(args_with_text);
        page_args.push(limit.into());
        page_args.push(offset.into());
        let rows: Vec<(i64, bool)> = self
            .conn
            .prepare(&format!("SELECT notes_fts.rowid, {is_phrase} {from} ORDER BY {order} LIMIT ? OFFSET ?"))?
            .query_map(params_from_iter(page_args), |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(WordRanking { rows, newest_first, phrase }))
    }

    /// The hits of a words search's page (see [`Vault::word_ranking`]), with snippets.
    fn word_hits(&self, fts: &str, ranked: &WordRanking, stop: &dyn Fn() -> bool) -> Result<Option<Vec<SearchHit>>> {
        let ids: Vec<i64> = ranked.rows.iter().map(|(id, _)| *id).collect();
        let phrase_ids: std::collections::HashSet<i64> =
            ranked.rows.iter().filter(|(_, p)| *p).map(|(id, _)| *id).collect();
        let phrase = &ranked.phrase;

        // Summaries and snippets for this page only. A phrase match shows the phrase,
        // not the words elsewhere in the note.
        if stop() {
            return Ok(None);
        }
        let (by_phrase, by_words): (Vec<i64>, Vec<i64>) = ids.iter().partition(|id| phrase_ids.contains(id));
        let mut snippets = self.snippets(fts, &by_words)?;
        if let Some(p) = phrase {
            if stop() {
                return Ok(None);
            }
            snippets.extend(self.snippets(p, &by_phrase)?);
        }
        if stop() {
            return Ok(None);
        }
        let hits = self
            .note_summaries(&ids)?
            .into_iter()
            .map(|note| SearchHit { snippet: snippets.remove(&note.id), note, why: None })
            .collect();
        Ok(Some(hits))
    }

    /// The cleaned body snippet (see [`clean_snippet`]) of each of `ids` that matches
    /// `matching` and has one.
    ///
    /// A query that ends in a word start is read in one pass over its matches, not a
    /// statement per note: there is no prefix index, so each statement merges the lists
    /// of every word that starts with it again, which for a page of 200 notes took
    /// seconds at 50,000 notes. Whole words keep a lookup per note, which reads one
    /// list and seeks in it.
    fn snippets(&self, matching: &str, ids: &[i64]) -> Result<std::collections::HashMap<i64, String>> {
        const SNIPPET: &str = "snippet(notes_fts, 1, char(2), char(3), '…', 16)";
        let mut found = std::collections::HashMap::new();
        let (Some(first), Some(last)) = (ids.iter().min(), ids.iter().max()) else { return Ok(found) };
        let mut keep = |id: i64, text: Option<String>| {
            if let Some(text) = text.and_then(|t| clean_snippet(&t)) {
                found.insert(id, text);
            }
        };
        if matching.ends_with('*') {
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            // The index walks its matches between the first and the last id (it seeks by
            // rowid); the `+` keeps SQLite from asking it for each id of the list in turn.
            let sql = format!(
                "SELECT rowid, {SNIPPET} FROM notes_fts
                 WHERE notes_fts MATCH ?1 AND rowid >= ?2 AND rowid <= ?3 AND +rowid IN ({list})"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![matching, first, last])?;
            while let Some(row) = rows.next()? {
                keep(row.get(0)?, row.get(1)?);
            }
        } else {
            let mut one =
                self.conn.prepare_cached(&format!("SELECT {SNIPPET} FROM notes_fts WHERE notes_fts MATCH ?1 AND rowid = ?2"))?;
            for &id in ids {
                keep(id, one.query_row(params![matching, id], |r| r.get(0)).optional()?);
            }
        }
        Ok(found)
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
        let ids = stmt.query_map(params_from_iter(args), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Whether a list of `q`'s folder without search text, `wanted` notes long, is
    /// cheaper read from the folder's part of the index and sorted than found by walking
    /// the vault newest first. The walk passes about `wanted × vault / folder` notes
    /// before it has `wanted` of the folder; the other way reads the folder's notes. The
    /// counts come from the `folders` table.
    fn browse_by_folder(&self, q: &SearchQuery, wanted: i64) -> Result<bool> {
        let Some(folder) = q.folder.as_deref().map(markdown::normalize_folder).filter(|f| !f.is_empty()) else {
            return Ok(false);
        };
        let (exact, below, end) = subtree_params(&folder);
        let (inside, vault): (i64, i64) = self.conn.prepare_cached(
            "SELECT coalesce(sum(notes) FILTER (WHERE path >= ?1 AND path < ?3 AND (path = ?1 OR path > ?2)), 0), coalesce(sum(notes), 0)
             FROM folders",
        )?
        .query_row(rusqlite::params![exact, below, end], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(inside.saturating_mul(inside) < wanted.max(1).saturating_mul(vault))
    }

    /// The number of notes in `folder` and its subfolders; every note for the root.
    pub fn count_in_folder(&self, folder: &str) -> Result<usize> {
        let folder = markdown::normalize_folder(folder);
        let count: i64 = if folder.is_empty() {
            self.conn.query_row("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL", [], |r| r.get(0))?
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
        self.strings("SELECT DISTINCT value FROM note_attributes WHERE key = ?1 ORDER BY value", [key])
    }

    fn strings(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params, |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// The notes a filtered meaning search may return (see [`Vault::search_filter_ids`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteFilter {
    Only(Vec<i64>),
    /// Every note but these.
    Except(Vec<i64>),
}

/// One page of a full-text ranking (see [`Vault::word_ranking`]).
struct WordRanking {
    /// Note ids, best first, with whether each matched as a phrase.
    rows: Vec<(i64, bool)>,
    newest_first: bool,
    /// The phrase query ranked first, if any.
    phrase: Option<String>,
}

/// A search without text: the notes matching `filters` (from [`filters`]), most recently
/// updated first, with the limit and offset as the last two parameters.
///
/// `by_folder` (see [`Vault::browse_by_folder`]) reads the folder's notes from
/// `notes_list_folder` and sorts them, instead of walking every note newest first until
/// the page is full, which for a small folder is the whole vault.
fn browse_sql(q: &SearchQuery, filters: &[String], by_folder: bool) -> String {
    // Left to itself SQLite walks `notes_list_updated` newest first and checks each
    // note's rating, which reads the whole index when few notes are rated. Most notes are
    // unrated, so reading the rated ones and sorting them is the cheaper bet. The same
    // goes for notes marked partly read or read; unread is nearly every note, so that
    // filter walks `notes_list_updated` as before.
    let index = if q.min_rating.is_some_and(|m| m > 0) {
        " INDEXED BY notes_rating"
    } else if matches!(q.read_status, Some(ReadFilter::PartlyRead | ReadFilter::Read)) {
        " INDEXED BY notes_read"
    } else if by_folder {
        " INDEXED BY notes_list_folder"
    } else {
        ""
    };
    let mut sql = format!("SELECT {SUMMARY_COLUMNS} FROM notes n{index} WHERE 1");
    for f in filters {
        sql.push_str(" AND ");
        sql.push_str(f);
    }
    sql.push_str(" ORDER BY n.updated_at DESC, n.id DESC LIMIT ? OFFSET ?");
    sql
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
            // The folder and all below it, as one range of an index on `folder` (see
            // `IN_SUBTREE`); `folder = ? OR folder LIKE ?` can only be checked row by row.
            filters.push("(n.folder >= ? AND n.folder < ? AND (n.folder = ? OR n.folder > ?))".into());
            let (exact, below, end) = subtree_params(&folder);
            args.extend([exact.clone().into(), end.into(), exact.into(), below.into()]);
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
    match q.read_status {
        None => {}
        Some(ReadFilter::Unread) => filters.push("n.read_status = 0".into()),
        Some(ReadFilter::NotFinished) => filters.push("n.read_status < 2".into()),
        Some(f @ (ReadFilter::PartlyRead | ReadFilter::Read)) => {
            // `read_status > 0` is the partial index's condition, spelled out so SQLite
            // can use it.
            filters.push("n.read_status > 0 AND n.read_status = ?".into());
            let status = if f == ReadFilter::Read { ReadStatus::Read } else { ReadStatus::PartlyRead };
            args.push(status.code().into());
        }
    }
    (filters, args)
}

/// Turns user input into a safe FTS5 query: every word, and every phrase the user put
/// in double quotes, is a quoted FTS5 string (so FTS syntax characters are literal), all
/// of them must match, and the last one is a prefix while the user is still typing its
/// last word, from [`PREFIX_MIN_CHARS`] characters on. An unclosed quote runs to the end
/// (`"9 yea` is the phrase `"9 yea"*`); a closed one is finished, so it gets no prefix.
fn fts_query(text: &str) -> Option<String> {
    let (terms, prefix) = terms(text);
    let last = terms.len().checked_sub(1)?;
    Some(
        terms
            .iter()
            .enumerate()
            .map(|(i, t)| if i == last && prefix { format!("\"{t}\"*") } else { format!("\"{t}\"") })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// For several words typed without quotes, the same words as one phrase (with the same
/// prefix rule): notes containing it are listed before those that only contain the words.
fn phrase_query(text: &str) -> Option<String> {
    let (terms, prefix) = terms(text);
    if text.contains('"') || terms.len() < 2 {
        return None;
    }
    let star = if prefix { "*" } else { "" };
    Some(format!("\"{}\"{star}", terms.join(" ")))
}

/// The words and quoted phrases of `text` (without quotes, never empty), and whether the
/// last one is a prefix.
fn terms(text: &str) -> (Vec<String>, bool) {
    let mut terms = Vec::new();
    let mut term = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' || (!quoted && c.is_whitespace()) {
            let phrase = term.split_whitespace().collect::<Vec<_>>().join(" ");
            if !phrase.is_empty() {
                terms.push(phrase);
            }
            term.clear();
            quoted ^= c == '"';
        } else {
            term.push(c);
        }
    }
    // Still typing when the text ends inside a word: not after a space or a quote.
    let phrase = term.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = !text.ends_with(char::is_whitespace)
        && phrase.split(' ').next_back().is_some_and(|w| w.chars().count() >= PREFIX_MIN_CHARS);
    if !phrase.is_empty() {
        terms.push(phrase);
    }
    (terms, prefix)
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
    use super::{ReadFilter, SearchQuery, browse_sql, clean_snippet, filters, fts_query, phrase_query};
    use crate::db::Vault;

    /// The query plan of a search without text, as `search_page` runs it.
    fn browse_plan(vault: &Vault, q: &SearchQuery) -> String {
        let sql = browse_sql(q, &filters(q).0, false);
        let mut stmt = vault.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        // Unbound parameters are NULL, which doesn't change the plan.
        let mut rows = stmt.raw_query();
        let mut out = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            out.push(r.get::<_, String>(3).unwrap());
        }
        out.join("\n")
    }

    /// Without text, "Unread" and "Not finished" walk the vault newest first like no
    /// filter does (most notes are unread, so a page fills quickly); "Partly read" and
    /// "Read" read only the marked notes, already in order, from the partial index.
    #[test]
    fn read_status_filters_use_the_right_index() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let q = |f: Option<ReadFilter>, folder: Option<&str>| SearchQuery {
            read_status: f,
            folder: folder.map(Into::into),
            ..Default::default()
        };
        let none = browse_plan(&vault, &q(None, None));
        assert!(none.contains("notes_list_updated"), "{none}");
        for f in [ReadFilter::Unread, ReadFilter::NotFinished] {
            assert_eq!(browse_plan(&vault, &q(Some(f), None)), none, "{f:?}");
            assert_eq!(browse_plan(&vault, &q(Some(f), Some("a"))), browse_plan(&vault, &q(None, Some("a"))), "{f:?} in a folder");
        }
        for f in [ReadFilter::PartlyRead, ReadFilter::Read] {
            for folder in [None, Some("a")] {
                let plan = browse_plan(&vault, &q(Some(f), folder));
                assert!(plan.contains("USING INDEX notes_read"), "{f:?}: {plan}");
                assert!(!plan.contains("TEMP B-TREE"), "{f:?}: already in order: {plan}");
            }
        }
    }

    /// Without text, the list comes from an index alone, in a folder too: the note rows
    /// hold the bodies, and the columns after the body can't be read without it.
    #[test]
    fn browsing_reads_an_index_not_the_notes() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        for folder in [None, Some("a")] {
            let plan = browse_plan(&vault, &SearchQuery { folder: folder.map(Into::into), ..Default::default() });
            assert!(plan.contains("USING COVERING INDEX notes_list_"), "{folder:?}: {plan}");
            assert!(!plan.contains("SCAN n\n") && !plan.ends_with("SCAN n"), "{folder:?}: {plan}");
        }
    }

    /// A small folder's list reads the folder's range of the index, not every note of
    /// the vault; a folder holding most of the vault is walked newest first.
    #[test]
    fn a_small_folders_list_reads_only_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        for i in 0..40 {
            vault.create_note("big/sub", &format!("n{i}"), "x").unwrap();
        }
        let small = vault.create_note("small", "a", "x").unwrap();
        vault.create_note("Small/Below", "b", "x").unwrap();
        vault.create_note("smaller", "c", "x").unwrap();
        let q = |folder: &str| SearchQuery { folder: Some(folder.into()), limit: Some(5), ..Default::default() };
        assert!(vault.browse_by_folder(&q("small"), 5).unwrap());
        assert!(!vault.browse_by_folder(&q("big"), 5).unwrap());
        assert!(!vault.browse_by_folder(&SearchQuery::default(), 5).unwrap());
        let sql = browse_sql(&q("small"), &filters(&q("small")).0, true);
        let mut plan = String::new();
        {
            let mut stmt = vault.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let mut rows = stmt.raw_query();
            while let Some(r) = rows.next().unwrap() {
                plan.push_str(&r.get::<_, String>(3).unwrap());
                plan.push('\n');
            }
        }
        assert!(plan.contains("USING COVERING INDEX notes_list_folder (folder>? AND folder<?)"), "{plan}");
        let titles: Vec<String> = vault.search(&q("small")).unwrap().into_iter().map(|h| h.note.title).collect();
        assert_eq!(titles, ["b", "a"], "newest first, with the subfolder, without `smaller`");
        // Timestamps are in milliseconds: make sure the edit is later than `b`.
        std::thread::sleep(std::time::Duration::from_millis(3));
        vault.update_body(small, "edited").unwrap();
        let titles: Vec<String> = vault.search(&q("SMALL")).unwrap().into_iter().map(|h| h.note.title).collect();
        assert_eq!(titles, ["a", "b"]);
        assert_eq!(vault.search(&q("big")).unwrap().len(), 5);
    }

    /// A broad read status filter (nearly every note passes) lists the few notes it
    /// excludes, from the partial index, rather than checking every note's row.
    #[test]
    fn broad_read_filters_list_the_notes_they_exclude() {
        use super::NoteFilter::{Except, Only};
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        let a = vault.create_note("", "a", "x").unwrap();
        let b = vault.create_note("", "b", "x").unwrap();
        let c = vault.create_note("f", "c", "x").unwrap();
        let d = vault.create_note("", "d", "x").unwrap();
        vault.set_read_status(&[b], crate::db::ReadStatus::PartlyRead).unwrap();
        vault.set_read_status(&[c], crate::db::ReadStatus::Read).unwrap();
        vault.trash_note(d).unwrap();
        let ids = |f: ReadFilter, folder: Option<&str>| {
            let q = SearchQuery { read_status: Some(f), folder: folder.map(Into::into), ..Default::default() };
            match vault.search_filter_ids(&q).unwrap().unwrap() {
                Only(mut ids) => Only({ ids.sort(); ids }),
                Except(mut ids) => Except({ ids.sort(); ids }),
            }
        };
        assert_eq!(ids(ReadFilter::Unread, None), Except(vec![b, c]));
        assert_eq!(ids(ReadFilter::NotFinished, None), Except(vec![c]));
        assert_eq!(ids(ReadFilter::Read, None), Only(vec![c]));
        assert_eq!(ids(ReadFilter::NotFinished, Some("f")), Only(vec![]));
        assert_eq!(ids(ReadFilter::Unread, Some("")), Except(vec![b, c]), "the vault root is no folder filter");
        let _ = a;
        assert_eq!(vault.search_filter_ids(&SearchQuery::default()).unwrap(), None);
        // The excluded notes come from the partial index alone.
        let mut stmt = vault.conn.prepare("EXPLAIN QUERY PLAN SELECT id FROM notes WHERE read_status > 0 AND read_status = 2").unwrap();
        let mut rows = stmt.raw_query();
        let mut plan = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            plan.push(r.get::<_, String>(3).unwrap());
        }
        assert!(plan.join(" ").contains("COVERING INDEX notes_read"), "{plan:?}");
    }

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
        assert_eq!(fts_query("foo bar").as_deref(), Some(r#""foo" "bar"*"#), "three letters start a word");
        assert_eq!(fts_query("foo ba").as_deref(), Some(r#""foo" "ba""#), "two letters are a whole word");
        assert_eq!(fts_query("foo b").as_deref(), Some(r#""foo" "b""#), "one letter is a whole word");
        assert_eq!(fts_query("é").as_deref(), Some(r#""é""#));
        assert_eq!(fts_query("éé").as_deref(), Some(r#""éé""#));
        assert_eq!(fts_query("ééé").as_deref(), Some(r#""ééé"*"#), "characters are counted, not bytes");
        assert_eq!(fts_query("foo bar ").as_deref(), Some(r#""foo" "bar""#));
        // Quotes make a phrase; an unclosed one runs to the end. FTS syntax inside stays literal.
        assert_eq!(fts_query(r#""9 years""#).as_deref(), Some(r#""9 years""#), "a closed quote is finished");
        assert_eq!(fts_query(r#""9 yea"#).as_deref(), Some(r#""9 yea"*"#));
        assert_eq!(fts_query(r#""9  years "#).as_deref(), Some(r#""9 years""#));
        assert_eq!(fts_query(r#""9 y"#).as_deref(), Some(r#""9 y""#));
        assert_eq!(fts_query(r#""9 ye"#).as_deref(), Some(r#""9 ye""#), "the last word of the phrase counts");
        assert_eq!(fts_query(r#"old "9 years" ag"#).as_deref(), Some(r#""old" "9 years" "ag""#));
        assert_eq!(fts_query(r#"old "9 years" ago"#).as_deref(), Some(r#""old" "9 years" "ago"*"#));
        assert_eq!(fts_query(r#"a"b OR cd*"#).as_deref(), Some(r#""a" "b OR cd*"*"#));
        assert_eq!(fts_query(r#"foo "" " "#).as_deref(), Some(r#""foo""#));
        assert_eq!(fts_query(r#"""#), None);
    }

    #[test]
    fn phrase_query_only_for_several_unquoted_words() {
        assert_eq!(phrase_query("9 years").as_deref(), Some(r#""9 years"*"#));
        assert_eq!(phrase_query(" 9   years ").as_deref(), Some(r#""9 years""#));
        assert_eq!(phrase_query("foo b").as_deref(), Some(r#""foo b""#));
        assert_eq!(phrase_query("foo ba").as_deref(), Some(r#""foo ba""#), "two letters are a whole word");
        assert_eq!(phrase_query("foo bar").as_deref(), Some(r#""foo bar"*"#));
        assert_eq!(phrase_query("years"), None);
        assert_eq!(phrase_query(r#""9 years""#), None, "quotes already ask for a phrase");
        assert_eq!(phrase_query(r#"old "9 years""#), None);
    }
}
