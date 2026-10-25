//! Discover tags (Settings → Tags): group notes by meaning and create a tag from a
//! group (see `crate::discover`).

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager, State};

use super::Result;
use crate::db::MetadataEditReport;
use crate::discover::{self, DiscoverError, Discovery, Options};
use crate::session::{Session, SessionError};

/// Counts runs, so a newer run (or a cancel) stops the one before it.
#[derive(Default)]
pub struct DiscoverJob {
    runs: AtomicU64,
}

/// Groups the notes off the main thread. Resolves to `null` when a newer run or
/// `cancel_discover_tags` stopped this one.
#[tauri::command]
pub async fn discover_tags(app: AppHandle, options: Options) -> Result<Option<Discovery>> {
    let session = app.state::<Session>();
    let generation = session.generation();
    if generation == 0 {
        return Err(SessionError::Locked.into());
    }
    // Starting a run is the user's doing; the run itself is not activity.
    session.touch();
    let run = app.state::<DiscoverJob>().runs.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let job = handle.state::<DiscoverJob>();
        let cancelled = || job.runs.load(Ordering::SeqCst) != run;
        discover::run(&handle.state::<Session>(), generation, &options, &cancelled)
    })
    .await;
    match result {
        Ok(Ok(discovery)) => Ok(Some(discovery)),
        Ok(Err(DiscoverError::Cancelled)) => Ok(None),
        Ok(Err(DiscoverError::Locked)) => Err(SessionError::Locked.into()),
        Ok(Err(DiscoverError::Db(e))) => Err(e.into()),
        Err(e) => panic!("tag discovery failed: {e}"),
    }
}

/// Stops a running `discover_tags`; it resolves to `null`.
#[tauri::command]
pub fn cancel_discover_tags(job: State<'_, DiscoverJob>) {
    job.runs.fetch_add(1, Ordering::SeqCst);
}

/// Saves the tag's description and adds the tag to `ids`, recording each as an
/// accepted example; all or nothing.
#[tauri::command]
pub fn create_tag_from_group(
    session: State<'_, Session>,
    tag: String,
    description: String,
    ids: Vec<i64>,
) -> Result<MetadataEditReport> {
    Ok(session.with_vault(|v| v.create_tag_from_group(&tag, &description, &ids))?)
}
