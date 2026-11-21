//! Discover tags (Settings → Tags): group notes by meaning and create a tag from a
//! group (see `crate::discover`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::{CommandError, Result};
use crate::activity::{Job, Reporter, count};
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
    let run = app.state::<DiscoverJob>().begin();
    let activity = session.activity().reporter(generation);
    activity.start(Job::Discover, "grouping notes by meaning");
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let job = handle.state::<DiscoverJob>();
        discover::run(&handle.state::<Session>(), generation, &options, &|| job.cancelled(run))
    })
    .await
    .map_err(|e| e.to_string());
    let job = app.state::<DiscoverJob>();
    job.report(run, activity, &result);
    job.finish(run, generation, result)
}

/// A run's outcome; `Err` when its thread failed.
type RunResult = std::result::Result<std::result::Result<Discovery, DiscoverError>, String>;

impl DiscoverJob {
    /// Starts a run, which stops the one before it; returns its number.
    fn begin(&self) -> u64 {
        let run = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        self.started.store(run, Ordering::SeqCst);
        *self.last.lock().unwrap() = None;
        run
    }

    /// Whether a newer run or a cancel stopped run `run`.
    fn cancelled(&self, run: u64) -> bool {
        self.runs.load(Ordering::SeqCst) != run
    }

    /// Ends run `run` in the Activity panel as `result` says.
    fn report(&self, run: u64, activity: Reporter<'_>, result: &RunResult) {
        // A run that a newer one replaced leaves the panel to the newer one.
        if self.started.load(Ordering::SeqCst) != run {
            return;
        }
        match result {
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

    /// What `discover_tags` resolves to for run `run` of unlock `generation`; keeps the
    /// groups' members for *Select all* while the run is still the latest.
    fn finish(&self, run: u64, generation: u64, result: RunResult) -> Result<Option<DiscoverResult>> {
        match result {
            Ok(Ok(mut discovery)) => {
                let members = discovery.groups.iter_mut().map(|g| std::mem::take(&mut g.members)).collect();
                let mut last = self.last.lock().unwrap();
                if self.cancelled(run) {
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

    /// Every member of the group `whole` names, if that is a group of the latest run and
    /// it ran in unlock `generation`.
    fn members(&self, whole: &WholeGroup, generation: u64) -> Result<Vec<i64>> {
        let last = self.last.lock().unwrap();
        match last.as_ref() {
            Some(l) if l.run == whole.run && l.generation == generation => {
                l.members.get(whole.group).cloned().ok_or(CommandError::StaleGroups)
            }
            _ => Err(CommandError::StaleGroups),
        }
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
    let members = job.members(&whole, session.generation())?;
    Ok(session.with_vault(|v| {
        // Notes trashed since the run are left out rather than failing the lot.
        let live = v.live_notes(&members)?;
        v.create_tag_from_group(&tag, &description, &live)
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Activity, State};
    use crate::db;
    use crate::discover::Group;

    fn group(members: Vec<i64>) -> Group {
        Group { words: vec!["word".into()], size: members.len(), notes: Vec::new(), members }
    }

    fn discovery() -> Discovery {
        Discovery {
            model: Some("model".into()),
            notes: 1500,
            sampled: 1000,
            k: 2,
            groups: vec![group(vec![1, 2, 3]), group(vec![4, 5])],
        }
    }

    fn whole(run: u64, group: usize) -> WholeGroup {
        WholeGroup { run, group }
    }

    #[test]
    fn a_new_run_stops_the_last_one_and_forgets_its_groups() {
        let job = DiscoverJob::default();
        let first = job.begin();
        job.finish(first, 7, Ok(Ok(discovery()))).unwrap();
        let second = job.begin();
        assert_eq!(second, first + 1);
        assert!(job.cancelled(first));
        assert!(!job.cancelled(second));
        assert!(matches!(job.members(&whole(first, 0), 7), Err(CommandError::StaleGroups)));
    }

    #[test]
    fn a_finished_run_keeps_its_groups_members_and_does_not_send_them() {
        let job = DiscoverJob::default();
        let run = job.begin();
        let result = job.finish(run, 7, Ok(Ok(discovery()))).unwrap().expect("the latest run");
        assert_eq!(result.run, run);
        assert!(result.discovery.groups.iter().all(|g| g.members.is_empty()));
        assert_eq!(job.members(&whole(run, 0), 7).unwrap(), vec![1, 2, 3]);
        assert_eq!(job.members(&whole(run, 1), 7).unwrap(), vec![4, 5]);
    }

    #[test]
    fn a_run_stopped_while_it_finished_resolves_to_nothing() {
        let job = DiscoverJob::default();
        let run = job.begin();
        job.runs.fetch_add(1, Ordering::SeqCst);
        assert!(job.finish(run, 7, Ok(Ok(discovery()))).unwrap().is_none());
        assert!(job.last.lock().unwrap().is_none());
        assert!(job.finish(run, 7, Ok(Err(DiscoverError::Cancelled))).unwrap().is_none());
    }

    #[test]
    fn a_failed_run_becomes_a_command_error() {
        let job = DiscoverJob::default();
        let run = job.begin();
        assert!(matches!(
            job.finish(run, 7, Ok(Err(DiscoverError::Locked))),
            Err(CommandError::Session(SessionError::Locked))
        ));
        assert!(matches!(
            job.finish(run, 7, Ok(Err(DiscoverError::Db(db::Error::NoteNotFound)))),
            Err(CommandError::Session(SessionError::Db(db::Error::NoteNotFound)))
        ));
    }

    #[test]
    #[should_panic(expected = "tag discovery failed: boom")]
    fn a_failed_grouping_thread_panics() {
        let job = DiscoverJob::default();
        let run = job.begin();
        let _ = job.finish(run, 7, Err("boom".into()));
    }

    #[test]
    fn select_all_needs_the_latest_run_of_this_unlock_and_a_group_it_had() {
        let job = DiscoverJob::default();
        assert!(matches!(job.members(&whole(1, 0), 7), Err(CommandError::StaleGroups)));
        let run = job.begin();
        job.finish(run, 7, Ok(Ok(discovery()))).unwrap();
        assert!(matches!(job.members(&whole(run + 1, 0), 7), Err(CommandError::StaleGroups)));
        assert!(matches!(job.members(&whole(run, 0), 8), Err(CommandError::StaleGroups)));
        assert!(matches!(job.members(&whole(run, 2), 7), Err(CommandError::StaleGroups)));
    }

    fn reported(result: RunResult) -> (State, Option<String>, Option<String>) {
        let activity = Activity::default();
        let reporter = activity.reporter(0);
        reporter.start(Job::Discover, "grouping");
        let job = DiscoverJob::default();
        let run = job.begin();
        job.report(run, reporter, &result);
        let status = activity.status(Job::Discover);
        (status.state, status.reason, status.last_run.map(|r| r.summary))
    }

    #[test]
    fn the_activity_panel_shows_how_the_run_ended() {
        assert_eq!(
            reported(Ok(Ok(discovery()))),
            (State::Idle, None, Some("2 groups from 1,500 notes (1,000 sampled)".into()))
        );
        assert_eq!(reported(Ok(Err(DiscoverError::Cancelled))), (State::Stopped, None, Some("cancelled".into())));
        let db = db::Error::NoteNotFound.to_string();
        assert_eq!(
            reported(Ok(Err(DiscoverError::Db(db::Error::NoteNotFound)))),
            (State::Failed, Some(db.clone()), Some(format!("failed: {db}")))
        );
        assert_eq!(
            reported(Err("boom".into())),
            (
                State::Failed,
                Some("the grouping thread failed".into()),
                Some("failed: the grouping thread failed".into())
            )
        );
    }

    #[test]
    fn a_lock_leaves_the_panel_to_the_lock() {
        assert_eq!(reported(Ok(Err(DiscoverError::Locked))), (State::Running, None, None));
    }

    #[test]
    fn a_replaced_run_leaves_the_panel_to_the_newer_one() {
        let activity = Activity::default();
        let reporter = activity.reporter(0);
        reporter.start(Job::Discover, "grouping");
        let job = DiscoverJob::default();
        let old = job.begin();
        job.begin();
        job.report(old, reporter, &Ok(Err(DiscoverError::Cancelled)));
        assert_eq!(activity.status(Job::Discover).state, State::Running);
    }
}
