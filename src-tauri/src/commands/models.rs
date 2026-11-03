//! Settings → Model: downloading, importing and removing optional models. Downloads
//! run on their own thread and need no vault, so they carry on while it is locked.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::embeddings::{Indexer, models_changed};
use super::{CommandError, Result};
use crate::embed::download::{self, DownloadError, Downloader, Progress};
use crate::embed::{self, ModelDescriptor, models};
use crate::session::Session;

/// Emitted with a [`DownloadStatus`] whenever a download moves on (at most every
/// [`EMIT_EVERY`]) or ends.
pub const MODEL_DOWNLOAD_EVENT: &str = "model-download";
const EMIT_EVERY: Duration = Duration::from_millis(250);
/// The speed is measured over about this long.
const SPEED_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DownloadState {
    Running { done: u64, total: u64, bytes_per_sec: Option<f64> },
    Done,
    Cancelled { done: u64, total: u64 },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub model: String,
    #[serde(flatten)]
    pub state: DownloadState,
}

/// The downloads of this app run, by model ID: the running ones with their cancel flag,
/// and how the last one of each model ended.
#[derive(Default)]
pub struct Downloads {
    downloader: Downloader,
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    status: Mutex<HashMap<String, DownloadState>>,
}

impl Downloads {
    fn set(&self, app: &AppHandle, model: &str, state: DownloadState) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).insert(model.to_string(), state.clone());
        let _ = app.emit(MODEL_DOWNLOAD_EVENT, DownloadStatus { model: model.to_string(), state });
    }

    fn running(&self, model: &str) -> bool {
        self.cancel.lock().unwrap_or_else(|e| e.into_inner()).contains_key(model)
    }
}

fn optional(model: &str) -> Result<&'static ModelDescriptor> {
    let d = embed::descriptor(model).ok_or_else(|| CommandError::UnknownModel(model.to_string()))?;
    if d.bundled {
        return Err(CommandError::ModelFiles(format!("{} ships with Hush", d.name)));
    }
    Ok(d)
}

/// How each model's last download this run went (running ones included).
#[tauri::command]
pub fn model_downloads(downloads: State<'_, Downloads>) -> Vec<DownloadStatus> {
    let status = downloads.status.lock().unwrap_or_else(|e| e.into_inner());
    status.iter().map(|(model, state)| DownloadStatus { model: model.clone(), state: state.clone() }).collect()
}

/// Starts downloading `model` from its pinned URL; progress arrives as
/// [`MODEL_DOWNLOAD_EVENT`]. The only command that makes Hush go online.
#[tauri::command]
pub fn download_model(app: AppHandle, downloads: State<'_, Downloads>, model: String) -> Result<()> {
    let descriptor = optional(&model)?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut running = downloads.cancel.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains_key(&model) {
            return Ok(());
        }
        running.insert(model.clone(), cancel.clone());
    }
    let total = descriptor.model.size + descriptor.tokenizer.size;
    downloads.set(&app, &model, DownloadState::Running { done: 0, total, bytes_per_sec: None });
    std::thread::spawn(move || {
        let downloads = app.state::<Downloads>();
        let dir = app.state::<Indexer>().models.dirs.user.clone();
        let mut samples: Vec<(Instant, u64)> = Vec::new();
        let mut last_emit: Option<Instant> = None;
        let mut last = Progress { done: 0, total };
        let result = downloads.downloader.model(&dir, descriptor, &cancel, &mut |p: Progress| {
            last = p;
            let now = Instant::now();
            samples.push((now, p.done));
            samples.retain(|s| now.duration_since(s.0) <= SPEED_WINDOW);
            if last_emit.is_some_and(|t| now.duration_since(t) < EMIT_EVERY) {
                return;
            }
            last_emit = Some(now);
            let speed = samples.first().and_then(|&(t, d)| {
                let secs = now.duration_since(t).as_secs_f64();
                (secs >= 0.5 && p.done >= d).then(|| (p.done - d) as f64 / secs)
            });
            downloads.set(&app, descriptor.id, DownloadState::Running {
                done: p.done,
                total: p.total,
                bytes_per_sec: speed,
            });
        });
        downloads.cancel.lock().unwrap_or_else(|e| e.into_inner()).remove(descriptor.id);
        let state = match result {
            Ok(()) => DownloadState::Done,
            Err(DownloadError::Cancelled) => DownloadState::Cancelled { done: last.done, total },
            Err(e) => DownloadState::Failed { message: e.to_string() },
        };
        if state == DownloadState::Done {
            models_changed(&app);
        }
        downloads.set(&app, descriptor.id, state);
    });
    Ok(())
}

/// Stops `model`'s download; what arrived is kept and the next Download continues.
#[tauri::command]
pub fn cancel_model_download(downloads: State<'_, Downloads>, model: String) {
    if let Some(cancel) = downloads.cancel.lock().unwrap_or_else(|e| e.into_inner()).get(&model) {
        cancel.store(true, Ordering::SeqCst);
    }
}

/// Copies files the user picked into the models folder, each checked against `model`'s
/// pinned checksums; returns the names stored. A file that matches neither is refused
/// with the expected and actual checksum, and nothing after it is copied.
#[tauri::command]
pub async fn import_model_files(
    app: AppHandle,
    indexer: State<'_, Indexer>,
    downloads: State<'_, Downloads>,
    model: String,
    paths: Vec<PathBuf>,
) -> Result<Vec<String>> {
    let descriptor = embed::descriptor(&model).ok_or_else(|| CommandError::UnknownModel(model.clone()))?;
    if downloads.running(&model) {
        return Err(CommandError::ModelFiles(format!("{} is downloading; cancel it first", descriptor.name)));
    }
    let mut stored = Vec::new();
    let result = paths.iter().try_for_each(|path| {
        stored.push(download::import_file(&indexer.models.dirs.user, descriptor, path)?.to_string());
        Ok::<_, download::ImportError>(())
    });
    if !stored.is_empty() {
        models_changed(&app);
    }
    result.map_err(|e| CommandError::ModelFiles(e.to_string()))?;
    Ok(stored)
}

/// Deletes a downloaded or imported model's files. Refused for the bundled model, while
/// it downloads, and while the open vault uses it (embeds with it, or its vectors serve).
#[tauri::command]
pub async fn remove_model(
    app: AppHandle,
    session: State<'_, Session>,
    indexer: State<'_, Indexer>,
    downloads: State<'_, Downloads>,
    model: String,
) -> Result<u64> {
    let descriptor = embed::descriptor(&model).ok_or_else(|| CommandError::UnknownModel(model.clone()))?;
    if let Some(reason) = models::removal_blocked(&session, descriptor)? {
        return Err(CommandError::ModelFiles(reason));
    }
    if downloads.running(&model) {
        return Err(CommandError::ModelFiles(format!("{} is downloading; cancel it first", descriptor.name)));
    }
    indexer.models.unload(descriptor.id);
    let freed = download::remove_files(&indexer.models.dirs.user, descriptor)
        .map_err(|e| CommandError::ModelFiles(format!("could not delete the files: {e}")))?;
    downloads.status.lock().unwrap_or_else(|e| e.into_inner()).remove(descriptor.id);
    models_changed(&app);
    Ok(freed)
}
