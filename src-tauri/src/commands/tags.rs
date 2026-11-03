//! Tag descriptions and zero-shot tag suggestions (see `db::tags`).

use serde::Serialize;
use tauri::State;

use super::Result;
use super::notes::{OpenedNote, opened};
use crate::db::{TagInfo, TagSuggestion, Vault};
use crate::embed;
use crate::session::Session;

/// Every tag used or described, for the Tags settings.
#[tauri::command]
pub fn tag_list(session: State<'_, Session>) -> Result<Vec<TagInfo>> {
    Ok(session.with_vault(|v| v.tag_list())?)
}

/// Sets a tag's description; an empty one removes it.
#[tauri::command]
pub fn set_tag_description(session: State<'_, Session>, tag: String, description: String) -> Result<()> {
    Ok(session.with_vault(|v| v.set_tag_description(&tag, &description))?)
}

#[tauri::command]
pub fn tag_suggestions(session: State<'_, Session>, id: i64) -> Result<Vec<TagSuggestion>> {
    Ok(session.with_vault(|v| v.tag_suggestions(id))?)
}

/// Adds the tag to the note's frontmatter and returns the note fresh; the frontend
/// flushes its autosave first and reloads the editor after.
#[tauri::command]
pub fn accept_tag_suggestion(session: State<'_, Session>, id: i64, tag: String) -> Result<OpenedNote> {
    Ok(session.with_vault(|v| {
        v.accept_tag_suggestion(id, &tag)?;
        opened(v, id)
    })?)
}

#[tauri::command]
pub fn reject_tag_suggestion(session: State<'_, Session>, id: i64, tag: String) -> Result<()> {
    Ok(session.with_vault(|v| v.reject_tag_suggestion(id, &tag))?)
}

/// The suggestion threshold of the model suggestions use: the ready model, or the
/// active one while no model is ready yet.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionThreshold {
    model: String,
    /// What is used now: the vault's override, else `default`.
    threshold: Option<f32>,
    /// The model's built-in default, if Hush knows the model.
    default: Option<f32>,
}

fn threshold_model(v: &Vault) -> crate::db::Result<String> {
    match v.embedding_model_ready()? {
        Some(model) => Ok(model),
        None => v.embedding_model(),
    }
}

#[tauri::command]
pub fn suggestion_threshold(session: State<'_, Session>) -> Result<SuggestionThreshold> {
    Ok(session.with_vault(|v| {
        let model = threshold_model(v)?;
        Ok(SuggestionThreshold {
            threshold: v.suggestion_threshold(&model)?,
            default: embed::descriptor(&model).map(|d| d.suggestion_threshold),
            model,
        })
    })?)
}

/// Overrides the threshold for the model suggestions use; `null` restores the default.
#[tauri::command]
pub fn set_suggestion_threshold(session: State<'_, Session>, threshold: Option<f32>) -> Result<()> {
    Ok(session.with_vault(|v| v.set_suggestion_threshold(&threshold_model(v)?, threshold))?)
}
