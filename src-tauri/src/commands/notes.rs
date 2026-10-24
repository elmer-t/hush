//! Notes: opening, autosave, rename/move, delete and frontmatter edits.

use serde::Serialize;
use tauri::State;

use super::Result;
use crate::db::{Note, NoteSummary, Vault};
use crate::markdown;
use crate::session::Session;

pub const DEFAULT_AUTHOR_SETTING: &str = "default_author";

/// What the metadata panel shows, parsed from the body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteMeta {
    author: Option<String>,
    topic: Option<String>,
    tags: Vec<String>,
    frontmatter_tags: Vec<String>,
    attributes: Vec<(String, String)>,
}

impl NoteMeta {
    fn of(body: &str) -> Self {
        let p = markdown::parse(body);
        Self {
            author: p.author,
            topic: p.topic,
            tags: p.tags,
            frontmatter_tags: p.frontmatter_tags,
            attributes: p.attributes,
        }
    }
}

#[derive(Serialize)]
pub struct OpenedNote {
    note: Note,
    meta: NoteMeta,
}

#[derive(Serialize)]
pub struct SavedNote {
    summary: NoteSummary,
    meta: NoteMeta,
}

fn opened(vault: &Vault, id: i64) -> crate::db::Result<OpenedNote> {
    let note = vault.note(id)?;
    let meta = NoteMeta::of(&note.body);
    Ok(OpenedNote { note, meta })
}

#[tauri::command]
pub fn folders(session: State<'_, Session>) -> Result<Vec<String>> {
    Ok(session.with_vault(|v| v.folders())?)
}

/// The explorer tree's sidebar view: `"tree"` or `"recent"`, stored in the vault.
pub const SIDEBAR_VIEW_SETTING: &str = "sidebar_view";

/// Notes directly in `folder` (not its subfolders), sorted by title, one page at a time.
#[tauri::command]
pub fn folder_notes(session: State<'_, Session>, folder: String, offset: u32, limit: u32) -> Result<Vec<NoteSummary>> {
    Ok(session.with_vault(|v| v.folder_notes(&folder, offset, limit))?)
}

#[tauri::command]
pub fn sidebar_view(session: State<'_, Session>) -> Result<String> {
    let view = session.with_vault(|v| v.setting(SIDEBAR_VIEW_SETTING))?;
    Ok(match view.as_deref() {
        Some("recent") => "recent".into(),
        _ => "tree".into(),
    })
}

#[tauri::command]
pub fn set_sidebar_view(session: State<'_, Session>, view: String) -> Result<()> {
    let view = if view == "recent" { "recent" } else { "tree" };
    Ok(session.with_vault(|v| v.set_setting(SIDEBAR_VIEW_SETTING, view))?)
}

/// Creates an untitled note in `folder`, with the vault's default author in its frontmatter.
#[tauri::command]
pub fn create_note(session: State<'_, Session>, folder: String) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| {
        let body = match v.setting(DEFAULT_AUTHOR_SETTING)? {
            Some(author) if !author.trim().is_empty() => markdown::set_frontmatter_field("", "author", &[author]),
            _ => String::new(),
        };
        let id = v.create_untitled(&folder, &body)?;
        opened(v, id)
    })?)
}

#[tauri::command]
pub fn open_note(session: State<'_, Session>, id: i64) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| opened(v, id))?)
}

#[tauri::command]
pub fn save_note(session: State<'_, Session>, id: i64, body: String) -> Result<SavedNote> {
    Ok(session.with_vault(|v| {
        v.update_body(id, &body)?;
        let summary = v.note(id)?.summary();
        Ok(SavedNote { summary, meta: NoteMeta::of(&body) })
    })?)
}

/// Renames and/or moves a note. Returns it fresh: its own links may have been rewritten.
#[tauri::command]
pub fn rename_note(session: State<'_, Session>, id: i64, folder: String, title: String) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| {
        v.rename_note(id, &folder, &title)?;
        opened(v, id)
    })?)
}

#[tauri::command]
pub fn delete_note(session: State<'_, Session>, id: i64) -> Result<()> {
    Ok(session.with_vault(|v| v.delete_note(id))?)
}

#[tauri::command]
pub fn edit_frontmatter(body: String, key: String, values: Vec<String>) -> String {
    markdown::set_frontmatter_field(&body, &key, &values)
}

#[tauri::command]
pub fn default_author(session: State<'_, Session>) -> Result<Option<String>> {
    Ok(session.with_vault(|v| v.setting(DEFAULT_AUTHOR_SETTING))?)
}

#[tauri::command]
pub fn set_default_author(session: State<'_, Session>, author: String) -> Result<()> {
    Ok(session.with_vault(|v| v.set_setting(DEFAULT_AUTHOR_SETTING, author.trim()))?)
}
