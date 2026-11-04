//! Discover tags (Settings → Tags): group notes by meaning and create a tag from a
//! group (see `crate::discover`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::{CommandError, Result};
use crate::activity::{Job, count};
use crate::db::MetadataEditReport;
use crate::discover::{self, DiscoverError, Discovery, Options};
use crate::session::{Session, SessionError};

/// Counts runs, so a newer run (or a cancel) stops the one before it, and keeps the
/// members of the latest run's groups for *Select all*, until the next run, a cancel or
/// a lock.
#[derive(Default)]
pub struct DiscoverJob {
    runs: AtomicU64,
    /// The latest run started (a cancel bumps `runs` without starting one), so a run
    /// cancelled by a newer one doesn't mark that one stopped in the Activity panel.
    started: AtomicU64,
    last: Mutex<Option<LastRun>>,
}

struct LastRun {
    run: u64,
    generation: u64,
    /// Every member of each group, in the order the groups were returned.
    members: Vec<Vec<i64>>,
}

/// A run's result, and its number, which *Select all* names the groups by.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    pub run: u64,
    #[serde(flatten)]
    pub discovery: Discovery,
}

/// Group `group` of run `run`, all of its members.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WholeGroup {
    pub run: u64,
    pub group: usize,
}

/// Groups the notes off the main thread. Resolves to `null` when a newer run or
/// `cancel_discover_tags` stopped this one.
#[tauri::command]
pub async fn discover_tags(app: AppHandle, options: Options) -> Result<Option<DiscoverResult>> {
    let session = app.state::<Session>();
    let generation = session.generation();
    if generation == 0 {
        return Err(SessionError::Locked.into());
    }
    // Starting a run is the user's doing; the run itself is not activity.
    session.touch();
    let job = app.state::<DiscoverJob>();
    let run = job.runs.fetch_add(1, Ordering::SeqCst) + 1;
    job.started.store(run, Ordering::SeqCst);
    *job.last.lock().unwrap() = None;
    let activity = session.activity().reporter(generation);
    activity.start(Job::Discover, "grouping notes by meaning");
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let job = handle.state::<DiscoverJob>();
        let cancelled = || job.runs.load(Ordering::SeqCst) != run;
        discover::run(&handle.state::<Session>(), generation, &options, &cancelled)
    })
    .await;
    // A run that a newer one replaced leaves the panel to the newer one.
    if app.state::<DiscoverJob>().started.load(Ordering::SeqCst) == run {
        match &result {
            Ok(Ok(d)) => activity.finish(
                Job::Discover,
                format!("{} groups from {} notes ({} sampled)", count(d.groups.len() as u64), count(d.notes as u64), count(d.sampled as u64)),
            ),
            Ok(Err(DiscoverError::Cancelled)) => activity.stop(Job::Discover, "cancelled"),
            Ok(Err(DiscoverError::Locked)) => {}
            Ok(Err(e)) => activity.fail(Job::Discover, e.to_string()),
            Err(_) => activity.fail(Job::Discover, "the grouping thread failed"),
        }
    }
    match result {
        Ok(Ok(mut discovery)) => {
            let members = discovery.groups.iter_mut().map(|g| std::mem::take(&mut g.members)).collect();
            let job = app.state::<DiscoverJob>();
            let mut last = job.last.lock().unwrap();
            if job.runs.load(Ordering::SeqCst) != run {
                return Ok(None);
            }
            *last = Some(LastRun { run, generation, members });
            Ok(Some(DiscoverResult { run, discovery }))
        }
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
    *job.last.lock().unwrap() = None;
}

/// Saves the tag's description and adds the tag to `ids`, or with `whole_group` to
/// every member of that group still live, recording each as an accepted example; all
/// or nothing.
#[tauri::command(async)]
pub fn create_tag_from_group(
    session: State<'_, Session>,
    job: State<'_, DiscoverJob>,
    tag: String,
    description: String,
    ids: Vec<i64>,
    whole_group: Option<WholeGroup>,
) -> Result<MetadataEditReport> {
    let Some(whole) = whole_group else {
        return Ok(session.with_vault(|v| v.create_tag_from_group(&tag, &description, &ids))?);
    };
    let members = {
        let last = job.last.lock().unwrap();
        match last.as_ref() {
            Some(l) if l.run == whole.run && l.generation == session.generation() => {
                l.members.get(whole.group).cloned().ok_or(CommandError::StaleGroups)?
            }
            _ => return Err(CommandError::StaleGroups),
        }
    };
    Ok(session.with_vault(|v| {
        // Notes trashed since the run are left out rather than failing the lot.
        let live = v.live_notes(&members)?;
        v.create_tag_from_group(&tag, &description, &live)
    })?)
}
