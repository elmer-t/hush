//! Wikilinks between notes: the details panel's backlinks and outgoing links, and
//! following a link from the preview.

use tauri::{AppHandle, Manager, State};

use super::Result;
use super::notes::{OpenedNote, new_note_body, opened};
use crate::db::{Backlink, OutgoingLink};
use crate::graph::{self, Graph, GraphOptions, GraphScope};
use crate::markdown;
use crate::session::Session;

/// Notes that link to note `id`, by title, each with the line around its link.
#[tauri::command]
pub fn backlinks(session: State<'_, Session>, id: i64) -> Result<Vec<Backlink>> {
    Ok(session.with_vault(|v| v.backlinks(id))?)
}

/// The targets note `id` links to, by target text, with what each resolves to.
#[tauri::command]
pub fn outgoing_links(session: State<'_, Session>, id: i64) -> Result<Vec<OutgoingLink>> {
    Ok(session.with_vault(|v| v.outgoing_links(id))?)
}

/// What a link target as written between `[[` and `]]` (without heading or alias)
/// resolves to.
#[tauri::command]
pub fn resolve_link(session: State<'_, Session>, target: String) -> Result<OutgoingLink> {
    Ok(session.with_vault(|v| v.resolve_link_notes(&target))?)
}

/// Creates the note an unresolved link target names (`path/Title` makes `Title` in
/// folder `path`) and opens it.
#[tauri::command]
pub fn create_linked_note(session: State<'_, Session>, target: String) -> Result<OpenedNote> {
    let (folder, title) = markdown::split_target(&target);
    Ok(session.with_vault(|v| {
        let body = new_note_body(v)?;
        let id = v.create_note(&folder, &title, &body)?;
        let note = opened(v, id)?;
        v.record_opened(id)?;
        Ok(note)
    })?)
}

/// The details panel's sections that are collapsed, as a comma-separated list of names.
pub const DETAILS_COLLAPSED_SETTING: &str = "details_collapsed";

/// Names of the collapsed details panel sections (`"related"`, `"backlinks"`, `"outgoing"`).
#[tauri::command]
pub fn collapsed_sections(session: State<'_, Session>) -> Result<Vec<String>> {
    let value = session
        .with_vault(|v| v.setting(DETAILS_COLLAPSED_SETTING))?
        .unwrap_or_default();
    Ok(value
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

#[tauri::command]
pub fn set_section_collapsed(
    session: State<'_, Session>,
    section: String,
    collapsed: bool,
) -> Result<()> {
    let section: String = section
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    Ok(session.with_vault(|v| {
        let value = v.setting(DETAILS_COLLAPSED_SETTING)?.unwrap_or_default();
        let mut sections: Vec<&str> = value
            .split(',')
            .filter(|s| !s.is_empty() && *s != section)
            .collect();
        if collapsed && !section.is_empty() {
            sections.push(&section);
        }
        v.set_setting(DETAILS_COLLAPSED_SETTING, &sections.join(","))
    })?)
}

/// The subgraph of `scope` for the graph view, built off the main thread.
#[tauri::command]
pub async fn graph(app: AppHandle, scope: GraphScope, options: GraphOptions) -> Result<Graph> {
    let graph = tauri::async_runtime::spawn_blocking(move || {
        graph::build(&app.state::<Session>(), &scope, &options)
    })
    .await
    .expect("graph build panicked")?;
    Ok(graph)
}
