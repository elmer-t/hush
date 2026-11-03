//! The Suggestions view: one tag's suggestions across the vault, bulk accept and reject,
//! and a training pass on demand (see `db::review` and `classify::review`).

use tauri::{AppHandle, Emitter, Manager, State};

use super::Result;
use super::training::TAGS_TRAINED_EVENT;
use crate::classify::{job, review};
use crate::db::{MetadataEditReport, ReviewOrder, SuggestionReview};
use crate::session::{Session, SessionError};

/// The first `limit` notes suggested for `tag`, in `order`, and how many there are.
#[tauri::command]
pub fn review_tag_suggestions(
    session: State<'_, Session>,
    tag: String,
    order: ReviewOrder,
    limit: usize,
) -> Result<SuggestionReview> {
    Ok(review::review(&session, &tag, order, limit)?)
}

/// Adds `tag` to the notes and records each as an accepted example, all or nothing. The
/// frontend flushes the open note first and reloads it after, as for a bulk edit.
#[tauri::command]
pub fn accept_tag_suggestions(session: State<'_, Session>, tag: String, ids: Vec<i64>) -> Result<MetadataEditReport> {
    Ok(session.with_vault(|v| v.accept_tag_suggestions(&ids, &tag))?)
}

/// Records `tag` as rejected on the notes, all or nothing; the notes don't change.
#[tauri::command]
pub fn reject_tag_suggestions(session: State<'_, Session>, tag: String, ids: Vec<i64>) -> Result<()> {
    Ok(session.with_vault(|v| v.reject_tag_suggestions(&ids, &tag))?)
}

/// Runs a training pass now instead of waiting for the job's quiet period, off the main
/// thread; resolves to whether a head was (re)trained or dropped (and then emits
/// `tags-trained`, as the job does). The Suggestions view asks after every 10 decisions.
#[tauri::command]
pub async fn retrain_tags(app: AppHandle) -> Result<bool> {
    let generation = app.state::<Session>().generation();
    if generation == 0 {
        return Err(SessionError::Locked.into());
    }
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || job::pass(&handle.state::<Session>(), generation)).await;
    match result {
        Ok(Ok(pass)) => {
            let changed = !pass.trained.is_empty() || pass.dropped > 0;
            if changed && app.state::<Session>().is_open(generation) {
                let _ = app.emit(TAGS_TRAINED_EVENT, ());
            }
            Ok(changed)
        }
        Ok(Err(job::TrainError::Locked)) => Err(SessionError::Locked.into()),
        Ok(Err(job::TrainError::Db(e))) => Err(e.into()),
        Err(e) => panic!("tag training failed: {e}"),
    }
}
