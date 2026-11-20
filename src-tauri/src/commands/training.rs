//! The tag training job (per-tag classifier heads, see `classify`) as the app runs it:
//! started on unlock next to the indexing job, telling the frontend when heads change.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use super::Result;
use crate::activity::{Job, Reporter};
use crate::classify::job;
use crate::db::TagTraining;
use crate::retag::{Retag, RetagError};
use crate::session::Session;

/// Emitted (without a payload) after a pass that trained or dropped a head.
pub const TAGS_TRAINED_EVENT: &str = "tags-trained";

/// How often the job checks whether the vault changed.
const POLL: Duration = Duration::from_secs(1);

/// Starts the job for unlock `generation`. It stops by itself when the vault locks.
pub fn start(app: &AppHandle, generation: u64) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        job::run(&session, generation, POLL, job::QUIET, |_| tags_trained(&app, generation));
    });
}

/// Emits [`TAGS_TRAINED_EVENT`] unless the vault of unlock `generation` has closed.
pub(super) fn tags_trained(app: &AppHandle, generation: u64) {
    if app.state::<Session>().is_open(generation) {
        let _ = app.emit(TAGS_TRAINED_EVENT, ());
    }
}

/// Starts the tag re-derive pass ([`crate::retag`]) for unlock `generation`; it ends by
/// itself, at once when the vault's tags follow the current rules. Tells the frontend
/// when tags or heads went away, so the Tags settings reload.
pub fn start_retag(app: &AppHandle, generation: u64) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        let result = crate::retag::run(&session, generation, POLL);
        if retag_changed(result, session.activity().reporter(generation)) {
            tags_trained(&app, generation);
        }
    });
}

/// Whether the re-derive pass changed tags or dropped tags or heads. A failure is
/// reported to `activity`.
fn retag_changed(result: std::result::Result<Option<Retag>, RetagError>, activity: Reporter<'_>) -> bool {
    match result {
        Ok(Some(r)) => r.changed > 0 || r.end.tags_dropped > 0 || r.end.heads_dropped > 0,
        Ok(None) | Err(RetagError::Locked) => false,
        // Not fatal: the old tags stay, and the next unlock tries again.
        Err(e) => {
            eprintln!("hush: re-deriving tags failed: {e}");
            activity.fail(Job::Retag, e.to_string());
            false
        }
    }
}

/// Example counts, status and cross-validated quality of every used or described tag.
#[tauri::command(async)]
pub fn tag_training(session: State<'_, Session>) -> Result<Vec<TagTraining>> {
    Ok(session.with_vault(|v| v.tag_training())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Activity, State};
    use crate::db::{self, RetagEnd};

    fn retag(changed: usize, tags_dropped: usize, heads_dropped: usize) -> Retag {
        Retag { checked: 10, changed, end: RetagEnd { tags_dropped, heads_dropped } }
    }

    #[test]
    fn a_retag_that_changed_or_dropped_anything_counts_as_changed() {
        let activity = Activity::default();
        let reporter = activity.reporter(0);
        assert!(retag_changed(Ok(Some(retag(1, 0, 0))), reporter));
        assert!(retag_changed(Ok(Some(retag(0, 1, 0))), reporter));
        assert!(retag_changed(Ok(Some(retag(0, 0, 1))), reporter));
        assert!(!retag_changed(Ok(Some(retag(0, 0, 0))), reporter));
        assert!(!retag_changed(Ok(None), reporter));
        assert_eq!(activity.status(Job::Retag).state, State::Idle);
    }

    #[test]
    fn a_lock_ends_the_retag_quietly() {
        let activity = Activity::default();
        assert!(!retag_changed(Err(RetagError::Locked), activity.reporter(0)));
        assert_eq!(activity.status(Job::Retag).state, State::Idle);
    }

    #[test]
    fn a_failed_retag_shows_in_the_activity_panel() {
        let activity = Activity::default();
        assert!(!retag_changed(Err(RetagError::Db(db::Error::NoteNotFound)), activity.reporter(0)));
        let status = activity.status(Job::Retag);
        assert_eq!(status.state, State::Failed);
        assert_eq!(status.reason, Some(db::Error::NoteNotFound.to_string()));
    }
}
