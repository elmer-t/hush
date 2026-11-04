//! The parts of the app layout that name note content: the last open note, the graph's
//! scope and the folder selected in the sidebar. They live in the vault's encrypted
//! `settings` table and are read once after unlock. The rest of the layout (panels shown,
//! editor mode, graph display controls) holds no note data and is stored per device by
//! the frontend.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::Result;
use super::notes::{OpenedNote, opened};
use crate::session::Session;

/// The graph's scope as JSON: `{ kind, folder, tag }`.
pub const GRAPH_SCOPE_SETTING: &str = "graph_scope";
/// The folder path selected in the sidebar.
pub const SIDEBAR_FOLDER_SETTING: &str = "sidebar_folder";

/// The note that was open last, read again without recording it as opened. `None` when
/// there is none, history is off, or it was trashed or deleted since (the key is then
/// cleared).
#[tauri::command(async)]
pub fn last_opened_note(session: State<'_, Session>) -> Result<Option<OpenedNote>> {
    Ok(session.with_vault(|v| v.last_opened()?.map(|id| opened(v, id)).transpose())?)
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphScope {
    /// `"folder"`, `"tag"`, `"search"` or `"all"`.
    kind: String,
    /// `None`: the open note's folder.
    folder: Option<String>,
    tag: String,
}

impl GraphScope {
    fn valid(&self) -> bool {
        matches!(self.kind.as_str(), "folder" | "tag" | "search" | "all")
    }
}

/// The stored graph scope; `None` when there is none or it can't be read.
#[tauri::command(async)]
pub fn graph_scope(session: State<'_, Session>) -> Result<Option<GraphScope>> {
    let value = session.with_vault(|v| v.setting(GRAPH_SCOPE_SETTING))?;
    Ok(value.and_then(|v| serde_json::from_str::<GraphScope>(&v).ok()).filter(GraphScope::valid))
}

#[tauri::command(async)]
pub fn set_graph_scope(session: State<'_, Session>, scope: GraphScope) -> Result<()> {
    if !scope.valid() {
        return Ok(());
    }
    let json = serde_json::to_string(&scope).expect("a scope serialises");
    Ok(session.with_vault(|v| v.set_setting(GRAPH_SCOPE_SETTING, &json))?)
}

#[tauri::command(async)]
pub fn sidebar_folder(session: State<'_, Session>) -> Result<Option<String>> {
    Ok(session.with_vault(|v| v.setting(SIDEBAR_FOLDER_SETTING))?)
}

/// Stores the selected folder; `None` (no folder selected) removes the key.
#[tauri::command(async)]
pub fn set_sidebar_folder(session: State<'_, Session>, folder: Option<String>) -> Result<()> {
    Ok(session.with_vault(|v| match folder {
        Some(f) => v.set_setting(SIDEBAR_FOLDER_SETTING, &f),
        None => v.delete_setting(SIDEBAR_FOLDER_SETTING),
    })?)
}
