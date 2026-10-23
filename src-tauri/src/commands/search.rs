//! Full-text search with filters, and the values the filters offer.

use tauri::State;

use super::Result;
use crate::db::{Facets, SearchHit, SearchQuery};
use crate::session::Session;

/// With text: ranked by relevance. Without: newest first, so this also lists notes.
#[tauri::command]
pub fn search_notes(session: State<'_, Session>, query: SearchQuery) -> Result<Vec<SearchHit>> {
    Ok(session.with_vault(|v| v.search(&query))?)
}

#[tauri::command]
pub fn search_facets(session: State<'_, Session>) -> Result<Facets> {
    Ok(session.with_vault(|v| v.facets())?)
}

#[tauri::command]
pub fn attribute_values(session: State<'_, Session>, key: String) -> Result<Vec<String>> {
    Ok(session.with_vault(|v| v.attribute_values(&key))?)
}
