//! The tag training job (per-tag classifier heads, see `classify`) as the app runs it:
//! started on unlock next to the indexing job, telling the frontend when heads change.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use super::Result;
use crate::classify::job;
use crate::db::TagTraining;
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
        job::run(&session, generation, POLL, job::QUIET, |_| {
            if session.is_open(generation) {
                let _ = app.emit(TAGS_TRAINED_EVENT, ());
            }
        });
    });
}

/// Example counts, status and cross-validated quality of every used or described tag.
#[tauri::command]
pub fn tag_training(session: State<'_, Session>) -> Result<Vec<TagTraining>> {
    Ok(session.with_vault(|v| v.tag_training())?)
}
