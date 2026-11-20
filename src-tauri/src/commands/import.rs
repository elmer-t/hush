//! Import: a background job with progress and cancel, and the label rules.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{CommandError, Result};
use crate::activity::{Job, Reporter, count};
use crate::import::rules::{self, Rule, RuleInput, RuleOutput};
use crate::import::{self, ImportError, ImportReport};
use crate::markdown;
use crate::session::{Session, SessionError};

/// Emitted with an [`import::Progress`] after each batch.
pub const IMPORT_PROGRESS_EVENT: &str = "import-progress";
/// Emitted with an [`ImportFinished`] when the job ends.
pub const IMPORT_FINISHED_EVENT: &str = "import-finished";

/// At most one import runs at a time.
#[derive(Default)]
pub struct ImportJob {
    running: AtomicBool,
    cancel: AtomicBool,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ImportFinished {
    Done { report: ImportReport },
    Failed { error: CommandError },
}

/// Starts a dry run or an import of `source` into the open vault and returns at once;
/// progress and the result arrive as events.
#[tauri::command(async)]
pub fn start_import(
    app: AppHandle,
    session: State<'_, Session>,
    job: State<'_, ImportJob>,
    source: PathBuf,
    target: String,
    dry_run: bool,
) -> Result<()> {
    if session.path().is_none() {
        return Err(SessionError::Locked.into());
    }
    job.claim()?;
    std::thread::spawn(move || run_import(&app, &source, &target, dry_run));
    Ok(())
}

impl ImportJob {
    /// Marks an import as running, unless one already is.
    fn claim(&self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CommandError::ImportRunning);
        }
        self.cancel.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// The import thread of [`start_import`].
fn run_import(app: &AppHandle, source: &Path, target: &str, dry_run: bool) {
    let session = app.state::<Session>();
    let job = app.state::<ImportJob>();
    let activity = session.activity().reporter(session.generation());
    let verb = if dry_run { "previewing" } else { "importing" };
    activity.start(Job::Import, format!("{verb}: finding files"));
    // A dry run only reads, so indexing carries on.
    let pause = (!dry_run).then(|| session.begin_import());
    let result = import::run(&session, source, target, dry_run, &job.cancel, |p| {
        let _ = app.emit(IMPORT_PROGRESS_EVENT, p);
        activity.progress(
            Job::Import,
            format!("{verb} {} / {} files", count(p.done as u64), count(p.total as u64)),
            p.done as u64,
            Some(p.total as u64),
        );
    });
    drop(pause);
    job.running.store(false, Ordering::SeqCst);
    report_import(activity, &result);
    // Errors aren't `Clone`, which `emit` needs; their serialized form is.
    if let Ok(payload) = serde_json::to_value(ImportFinished::from(result)) {
        let _ = app.emit(IMPORT_FINISHED_EVENT, payload);
    }
}

/// Ends the Activity panel's import run as the import ended.
fn report_import(activity: Reporter<'_>, result: &std::result::Result<ImportReport, ImportError>) {
    match result {
        Ok(r) => {
            let what = if r.dry_run { "preview" } else { "import" };
            let summary = format!(
                "{what}: {} of {} files {}, {} skipped, {} warnings",
                count(r.imported as u64),
                count(r.files as u64),
                if r.dry_run { "would be imported" } else { "imported" },
                count(r.skipped_count as u64),
                count(r.warning_count as u64),
            );
            if r.cancelled {
                activity.stop(Job::Import, format!("cancelled {summary}"));
            } else {
                activity.finish(Job::Import, summary);
            }
        }
        Err(e) => activity.fail(Job::Import, e.to_string()),
    }
}

impl From<std::result::Result<ImportReport, ImportError>> for ImportFinished {
    fn from(result: std::result::Result<ImportReport, ImportError>) -> Self {
        match result {
            Ok(report) => Self::Done { report },
            Err(e) => Self::Failed { error: e.into() },
        }
    }
}

/// The target folder suggested for importing `source`: the source folder's name.
#[tauri::command]
pub fn suggest_import_target(source: String) -> String {
    import::suggest_target(&source)
}

/// How many notes are already in `target` and its subfolders, to warn that an import
/// would merge into them.
#[tauri::command(async)]
pub fn import_target_notes(session: State<'_, Session>, target: String) -> Result<usize> {
    let target = markdown::normalize_folder(&target);
    if target.is_empty() {
        return Ok(0);
    }
    Ok(session.with_vault(|v| v.count_in_folder(&target))?)
}

/// Stops the running import after its current batch.
#[tauri::command]
pub fn cancel_import(job: State<'_, ImportJob>) {
    job.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command(async)]
pub fn import_rules(session: State<'_, Session>) -> Result<Vec<Rule>> {
    Ok(session.with_vault(|v| v.import_rules())?)
}

#[tauri::command(async)]
pub fn set_import_rules(session: State<'_, Session>, rules: Vec<Rule>) -> Result<()> {
    Ok(session.with_vault(|v| v.set_import_rules(&rules))?)
}

#[tauri::command]
pub fn default_import_rules() -> Vec<Rule> {
    Rule::defaults()
}

/// Runs `rules` (enabled ones, in order) on a pasted sample, as the import would.
#[tauri::command]
pub fn test_import_rules(rules: Vec<Rule>, file_name: String, sample: String) -> Result<RuleOutput> {
    let compiled = rules
        .iter()
        .map(|r| r.compile().map_err(crate::db::Error::InvalidRule))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let sample = sample.replace("\r\n", "\n");
    let (frontmatter, start) = markdown::split_frontmatter(&sample).unwrap_or(("", 0));
    Ok(rules::apply(&compiled, &RuleInput { file_name: &file_name, frontmatter, content: &sample[start..] }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Activity, State};

    fn report(dry_run: bool, cancelled: bool) -> ImportReport {
        ImportReport {
            dry_run,
            cancelled,
            files: 1200,
            imported: 1100,
            skipped_count: 100,
            warning_count: 3,
            ..ImportReport::default()
        }
    }

    fn summary(activity: &Activity) -> String {
        activity.status(Job::Import).last_run.expect("a finished run").summary
    }

    #[test]
    fn only_one_import_runs_at_a_time() {
        let job = ImportJob::default();
        job.cancel.store(true, Ordering::SeqCst);
        job.claim().unwrap();
        assert!(!job.cancel.load(Ordering::SeqCst), "a new import starts uncancelled");
        assert!(matches!(job.claim(), Err(CommandError::ImportRunning)));
        job.running.store(false, Ordering::SeqCst);
        job.claim().unwrap();
    }

    #[test]
    fn a_finished_import_is_summed_up() {
        let activity = Activity::default();
        report_import(activity.reporter(0), &Ok(report(false, false)));
        assert_eq!(activity.status(Job::Import).state, State::Idle);
        assert_eq!(summary(&activity), "import: 1,100 of 1,200 files imported, 100 skipped, 3 warnings");
    }

    #[test]
    fn a_preview_says_what_would_be_imported() {
        let activity = Activity::default();
        report_import(activity.reporter(0), &Ok(report(true, false)));
        assert_eq!(summary(&activity), "preview: 1,100 of 1,200 files would be imported, 100 skipped, 3 warnings");
    }

    #[test]
    fn a_cancelled_import_stops_with_what_it_did() {
        let activity = Activity::default();
        report_import(activity.reporter(0), &Ok(report(false, true)));
        assert_eq!(activity.status(Job::Import).state, State::Stopped);
        assert_eq!(summary(&activity), "cancelled import: 1,100 of 1,200 files imported, 100 skipped, 3 warnings");
    }

    #[test]
    fn a_failed_import_fails_in_the_panel_and_in_the_event() {
        let activity = Activity::default();
        let error = || ImportError::Source(std::io::Error::other("gone"));
        report_import(activity.reporter(0), &Err(error()));
        let status = activity.status(Job::Import);
        assert_eq!(status.state, State::Failed);
        assert_eq!(status.reason, Some(error().to_string()));

        let payload = serde_json::to_value(ImportFinished::from(Err(error()))).unwrap();
        assert_eq!(payload["status"], "failed");
        assert_eq!(payload["error"]["kind"], "importSource");
    }

    #[test]
    fn a_finished_import_sends_its_report() {
        let payload = serde_json::to_value(ImportFinished::from(Ok(report(true, false)))).unwrap();
        assert_eq!(payload["status"], "done");
        assert_eq!(payload["report"]["imported"], 1100);
        assert_eq!(payload["report"]["dryRun"], true);
    }
}
