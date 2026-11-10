//! Full-text search with filters, and the values the filters offer.

use tauri::State;

use super::Result;
use serde::Serialize;

use super::embeddings::Indexer;
use crate::db::{Facets, SearchMode, SearchPage, SearchQuery};
use crate::meaning::{self, QueryModels};
use crate::session::Session;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    page: SearchPage,
    /// A newer search started while this one ran; the frontend drops the result.
    cancelled: bool,
}

/// With text: ranked by relevance (or newest first for very broad queries), or by
/// meaning and words fused in `Meaning` mode (see [`meaning::search`]). Without: most
/// recently updated first, so this also lists notes. A newer search cancels an older
/// one still running.
#[tauri::command(async)]
pub fn search_notes(session: State<'_, Session>, indexer: State<'_, Indexer>, query: SearchQuery) -> Result<SearchResult> {
    let page = match query.mode {
        SearchMode::Words => session.search(|v, stop| v.search_page_until(&query, stop))?,
        SearchMode::Meaning => meaning::search(&session, &indexer.models, &query)?,
    };
    Ok(match page {
        Some(page) => SearchResult { page, cancelled: false },
        None => SearchResult { page: SearchPage::words(Vec::new(), false), cancelled: true },
    })
}

/// Whether a meaning search would have to load the model for its query first (the
/// sidebar then says "Loading model…" while it waits).
#[tauri::command(async)]
pub fn meaning_model_loaded(session: State<'_, Session>, indexer: State<'_, Indexer>) -> Result<bool> {
    let ready = session.with_vault(|v| v.embedding_model_ready())?;
    Ok(ready.is_some_and(|m| indexer.models.query_loaded(&m)))
}

#[tauri::command(async)]
pub fn search_facets(session: State<'_, Session>) -> Result<Facets> {
    Ok(session.with_vault(|v| v.facets())?)
}

#[tauri::command(async)]
pub fn attribute_values(session: State<'_, Session>, key: String) -> Result<Vec<String>> {
    Ok(session.with_vault(|v| v.attribute_values(&key))?)
}
