//! The Suggestions view: one tag's suggestions across the vault, bulk accept and reject,
//! and a training pass on demand (see `db::review` and `classify::review`).

use tauri::{AppHandle, Manager, State};

use super::Result;
use super::training::tags_trained;
use crate::classify::job::{self, TrainError, TrainPass};
use crate::classify::review;
use crate::db::{MetadataEditReport, ReviewOrder, SuggestionReview};
use crate::session::{Session, SessionError};

/// The first `limit` notes suggested for `tag`, in `order`, and how many there are.
#[tauri::command(async)]
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
#[tauri::command(async)]
pub fn accept_tag_suggestions(session: State<'_, Session>, tag: String, ids: Vec<i64>) -> Result<MetadataEditReport> {
    Ok(session.with_vault(|v| v.accept_tag_suggestions(&ids, &tag))?)
}

/// Records `tag` as rejected on the notes, all or nothing; the notes don't change.
#[tauri::command(async)]
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
    let result = tauri::async_runtime::spawn_blocking(move || job::pass(&handle.state::<Session>(), generation))
        .await
        .map_err(|e| e.to_string());
    let changed = pass_changed(result)?;
    if changed {
        tags_trained(&app, generation);
    }
    Ok(changed)
}

/// Whether a training pass (`Err`: its thread failed) trained or dropped a head.
fn pass_changed(result: std::result::Result<std::result::Result<TrainPass, TrainError>, String>) -> Result<bool> {
    match result {
        Ok(Ok(pass)) => Ok(!pass.trained.is_empty() || pass.dropped > 0),
        Ok(Err(TrainError::Locked)) => Err(SessionError::Locked.into()),
        Ok(Err(TrainError::Db(e))) => Err(e.into()),
        Err(e) => panic!("tag training failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::commands::CommandError;

    #[test]
    fn a_pass_that_trained_or_dropped_a_head_changed_something() {
        let trained = TrainPass { trained: vec!["work".into()], ..TrainPass::default() };
        let dropped = TrainPass { dropped: 1, ..TrainPass::default() };
        let current = TrainPass { current: 3, ..TrainPass::default() };
        assert!(pass_changed(Ok(Ok(trained))).unwrap());
        assert!(pass_changed(Ok(Ok(dropped))).unwrap());
        assert!(!pass_changed(Ok(Ok(current))).unwrap());
    }

    #[test]
    fn a_failed_pass_becomes_a_command_error() {
        assert!(matches!(
            pass_changed(Ok(Err(TrainError::Locked))),
            Err(CommandError::Session(SessionError::Locked))
        ));
        assert!(matches!(
            pass_changed(Ok(Err(TrainError::Db(db::Error::NoteNotFound)))),
            Err(CommandError::Session(SessionError::Db(db::Error::NoteNotFound)))
        ));
    }

    #[test]
    #[should_panic(expected = "tag training failed: boom")]
    fn a_failed_training_thread_panics() {
        let _ = pass_changed(Err("boom".into()));
    }
}
