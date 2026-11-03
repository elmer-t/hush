//! Import: a background job with progress and cancel, and the label rules.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{CommandError, Result};
use crate::activity::{Job, count};
use crate::import::rules::{self, Rule, RuleInput, RuleOutput};
use crate::import::{self, ImportReport};
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
#[tauri::command]
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
    if job.running.swap(true, Ordering::SeqCst) {
        return Err(CommandError::ImportRunning);
    }
    job.cancel.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        let job = app.state::<ImportJob>();
        let activity = session.activity().reporter(session.generation());
        let verb = if dry_run { "previewing" } else { "importing" };
        activity.start(Job::Import, format!("{verb}: finding files"));
        // A dry run only reads, so indexing carries on.
        let pause = (!dry_run).then(|| session.begin_import());
        let result = import::run(&session, &source, &target, dry_run, &job.cancel, |p| {
            let _ = app.emit(IMPORT_PROGRESS_EVENT, p);
            activity.progress(
                Job::Import,
                format!(
                    "{verb} {} / {} files",
                    count(p.done as u64),
                    count(p.total as u64)
                ),
                p.done as u64,
                Some(p.total as u64),
            );
        });
        drop(pause);
        job.running.store(false, Ordering::SeqCst);
        match &result {
            Ok(r) => {
                let what = if r.dry_run { "preview" } else { "import" };
                let summary = format!(
                    "{what}: {} of {} files {}, {} skipped, {} warnings",
                    count(r.imported as u64),
                    count(r.files as u64),
                    if r.dry_run {
                        "would be imported"
                    } else {
                        "imported"
                    },
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
        let finished = match result {
            Ok(report) => ImportFinished::Done { report },
            Err(e) => ImportFinished::Failed { error: e.into() },
        };
        // Errors aren't `Clone`, which `emit` needs; their serialized form is.
        if let Ok(payload) = serde_json::to_value(finished) {
            let _ = app.emit(IMPORT_FINISHED_EVENT, payload);
        }
    });
    Ok(())
}

/// The target folder suggested for importing `source`: the source folder's name.
#[tauri::command]
pub fn suggest_import_target(source: String) -> String {
    import::suggest_target(&source)
}

/// How many notes are already in `target` and its subfolders, to warn that an import
/// would merge into them.
#[tauri::command]
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

#[tauri::command]
pub fn import_rules(session: State<'_, Session>) -> Result<Vec<Rule>> {
    Ok(session.with_vault(|v| v.import_rules())?)
}

#[tauri::command]
pub fn set_import_rules(session: State<'_, Session>, rules: Vec<Rule>) -> Result<()> {
    Ok(session.with_vault(|v| v.set_import_rules(&rules))?)
}

#[tauri::command]
pub fn default_import_rules() -> Vec<Rule> {
    Rule::defaults()
}

/// Runs `rules` (enabled ones, in order) on a pasted sample, as the import would.
#[tauri::command]
pub fn test_import_rules(
    rules: Vec<Rule>,
    file_name: String,
    sample: String,
) -> Result<RuleOutput> {
    let compiled = rules
        .iter()
        .map(|r| r.compile().map_err(crate::db::Error::InvalidRule))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let sample = sample.replace("\r\n", "\n");
    let (frontmatter, start) = markdown::split_frontmatter(&sample).unwrap_or(("", 0));
    Ok(rules::apply(
        &compiled,
        &RuleInput {
            file_name: &file_name,
            frontmatter,
            content: &sample[start..],
        },
    ))
}
