//! Full-text search with filters, and the values the filters offer.

use tauri::State;

use super::Result;
use serde::Serialize;

use crate::db::{Facets, SearchPage, SearchQuery};
use crate::session::Session;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    page: SearchPage,
    /// A newer search started while this one ran; the frontend drops the result.
    cancelled: bool,
}

/// With text: ranked by relevance (or newest first for very broad queries). Without:
/// most recently updated first, so this also lists notes. A newer search cancels an
/// older one still running.
#[tauri::command(async)]
pub fn search_notes(session: State<'_, Session>, query: SearchQuery) -> Result<SearchResult> {
    Ok(match session.search(|v, stop| v.search_page_until(&query, stop))? {
        Some(page) => SearchResult { page, cancelled: false },
        None => SearchResult { page: SearchPage { hits: Vec::new(), newest_first: false }, cancelled: true },
    })
}

#[tauri::command(async)]
pub fn search_facets(session: State<'_, Session>) -> Result<Facets> {
    Ok(session.with_vault(|v| v.facets())?)
}

#[tauri::command(async)]
pub fn attribute_values(session: State<'_, Session>, key: String) -> Result<Vec<String>> {
    Ok(session.with_vault(|v| v.attribute_values(&key))?)
}
