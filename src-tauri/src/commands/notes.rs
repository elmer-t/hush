//! Notes: opening, autosave, rename/move, delete and frontmatter edits.

use serde::Serialize;
use tauri::State;

use super::Result;
use crate::db::{FolderContents, MetadataEdit, MetadataEditReport, Note, NoteSummary, TrashEntry, Vault};
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

pub(super) fn opened(vault: &Vault, id: i64) -> crate::db::Result<OpenedNote> {
    let note = vault.note(id)?;
    let meta = NoteMeta::of(&note.body);
    Ok(OpenedNote { note, meta })
}

#[tauri::command]
pub fn folders(session: State<'_, Session>) -> Result<Vec<String>> {
    Ok(session.with_vault(|v| v.folders())?)
}

/// The left pane's view: `"tree"`, `"recent"` or `"suggestions"`, stored in the vault.
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
        Some(v @ ("recent" | "suggestions")) => v.into(),
        _ => "tree".into(),
    })
}

#[tauri::command]
pub fn set_sidebar_view(session: State<'_, Session>, view: String) -> Result<()> {
    let view = match view.as_str() {
        "recent" => "recent",
        "suggestions" => "suggestions",
        _ => "tree",
    };
    Ok(session.with_vault(|v| v.set_setting(SIDEBAR_VIEW_SETTING, view))?)
}

/// Whether moving notes to the trash asks first: `"false"` turns it off; anything else, or
/// no value, leaves it on.
pub const CONFIRM_TRASH_SETTING: &str = "confirm_trash";

#[tauri::command]
pub fn confirm_trash(session: State<'_, Session>) -> Result<bool> {
    let value = session.with_vault(|v| v.setting(CONFIRM_TRASH_SETTING))?;
    Ok(value.as_deref() != Some("false"))
}

#[tauri::command]
pub fn set_confirm_trash(session: State<'_, Session>, confirm: bool) -> Result<()> {
    Ok(session.with_vault(|v| v.set_setting(CONFIRM_TRASH_SETTING, if confirm { "true" } else { "false" }))?)
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
        let note = opened(v, id)?;
        v.record_opened(id)?;
        Ok(note)
    })?)
}

/// Opens a note and, unless `record` is false (the frontend reading the open note
/// again), puts it at the front of the recently opened list.
#[tauri::command]
pub fn open_note(session: State<'_, Session>, id: i64, record: Option<bool>) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| {
        let note = opened(v, id)?;
        if record != Some(false) {
            v.record_opened(id)?;
        }
        Ok(note)
    })?)
}

/// The recently opened notes, most recent first, without trashed ones.
#[tauri::command]
pub fn recent_opened_notes(session: State<'_, Session>) -> Result<Vec<NoteSummary>> {
    Ok(session.with_vault(|v| v.recent_opened_notes())?)
}

#[tauri::command]
pub fn recent_opened_count(session: State<'_, Session>) -> Result<u32> {
    Ok(session.with_vault(|v| v.recent_opened_count())?)
}

/// Sets how many recently opened notes are kept (0 turns the list off and clears it).
#[tauri::command]
pub fn set_recent_opened_count(session: State<'_, Session>, count: u32) -> Result<()> {
    Ok(session.with_vault(|v| v.set_recent_opened_count(count))?)
}

#[tauri::command]
pub fn clear_recent_opened(session: State<'_, Session>) -> Result<()> {
    Ok(session.with_vault(|v| v.clear_recent_opened())?)
}

/// Whether the "Recently opened" section is collapsed: `"true"` collapses it.
pub const RECENT_OPENED_COLLAPSED_SETTING: &str = "recent_opened_collapsed";

#[tauri::command]
pub fn recent_opened_collapsed(session: State<'_, Session>) -> Result<bool> {
    Ok(session.with_vault(|v| v.setting(RECENT_OPENED_COLLAPSED_SETTING))?.as_deref() == Some("true"))
}

#[tauri::command]
pub fn set_recent_opened_collapsed(session: State<'_, Session>, collapsed: bool) -> Result<()> {
    Ok(session.with_vault(|v| v.set_setting(RECENT_OPENED_COLLAPSED_SETTING, if collapsed { "true" } else { "false" }))?)
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

/// The default delete: moves the note to the trash.
#[tauri::command]
pub fn trash_note(session: State<'_, Session>, id: i64) -> Result<()> {
    Ok(session.with_vault(|v| v.trash_note(id))?)
}

/// Moves notes into `folder`, all or nothing.
#[tauri::command]
pub fn move_notes(session: State<'_, Session>, ids: Vec<i64>, folder: String) -> Result<()> {
    Ok(session.with_vault(|v| v.move_notes(&ids, &folder))?)
}

/// Moves folder `from` and everything in it into folder `to`, all or nothing. Returns
/// the folder's new path.
#[tauri::command]
pub fn move_folder(session: State<'_, Session>, from: String, to: String) -> Result<String> {
    Ok(session.with_vault(|v| v.move_folder(&from, &to))?)
}

/// Bulk delete: moves notes to the trash, all or nothing.
#[tauri::command]
pub fn trash_notes(session: State<'_, Session>, ids: Vec<i64>) -> Result<()> {
    Ok(session.with_vault(|v| v.trash_notes(&ids))?)
}

/// Adds or removes a tag, or sets the topic or author, on several notes, all or nothing.
#[tauri::command]
pub fn edit_notes_metadata(session: State<'_, Session>, ids: Vec<i64>, edit: MetadataEdit) -> Result<MetadataEditReport> {
    Ok(session.with_vault(|v| v.edit_notes_metadata(&ids, &edit))?)
}

/// Deletes a note for good, whether live or in the trash.
#[tauri::command]
pub fn delete_note(session: State<'_, Session>, id: i64) -> Result<()> {
    Ok(session.with_vault(|v| v.delete_note(id))?)
}

/// One page of the trash: folder groups as one entry each, and loose notes.
#[tauri::command]
pub fn trash_entries(session: State<'_, Session>, offset: u32, limit: u32) -> Result<Vec<TrashEntry>> {
    Ok(session.with_vault(|v| v.trash_entries(offset, limit))?)
}

/// How many notes are in the trash.
#[tauri::command]
pub fn trash_count(session: State<'_, Session>) -> Result<usize> {
    Ok(session.with_vault(|v| v.trash_count())?)
}

/// What deleting `folder` would move to the trash, for the confirmation.
#[tauri::command]
pub fn folder_contents(session: State<'_, Session>, folder: String) -> Result<FolderContents> {
    Ok(session.with_vault(|v| v.folder_contents(&folder))?)
}

/// Moves a folder and every note in it and below it to the trash as one entry. Returns
/// how many notes were trashed.
#[tauri::command]
pub fn trash_folder(session: State<'_, Session>, folder: String) -> Result<usize> {
    Ok(session.with_vault(|v| v.trash_folder(&folder))?)
}

/// Restores a folder group from the trash. Returns how many notes came back.
#[tauri::command]
pub fn restore_folder(session: State<'_, Session>, deleted_at: i64) -> Result<usize> {
    Ok(session.with_vault(|v| v.restore_folder(deleted_at))?)
}

/// Deletes a folder group from the trash for good. Returns how many notes.
#[tauri::command]
pub fn delete_trash_folder(session: State<'_, Session>, deleted_at: i64) -> Result<usize> {
    Ok(session.with_vault(|v| v.delete_trash_folder(deleted_at))?)
}

/// Restores a trashed note and returns it opened (its title may have changed).
#[tauri::command]
pub fn restore_note(session: State<'_, Session>, id: i64) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| {
        v.restore_note(id)?;
        opened(v, id)
    })?)
}

#[tauri::command]
pub fn empty_trash(session: State<'_, Session>) -> Result<usize> {
    Ok(session.with_vault(|v| v.empty_trash())?)
}

#[tauri::command]
pub fn trash_days(session: State<'_, Session>) -> Result<u32> {
    Ok(session.with_vault(|v| v.trash_days())?)
}

#[tauri::command]
pub fn set_trash_days(session: State<'_, Session>, days: u32) -> Result<()> {
    Ok(session.with_vault(|v| v.set_trash_days(days))?)
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
