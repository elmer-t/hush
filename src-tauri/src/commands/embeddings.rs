//! The indexing job (embeddings) as the app runs it: started on unlock, with the
//! bundled model, reporting its status to the frontend. Also the related-notes lookup
//! the job's vectors feed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::CommandError;

use crate::config::Config;
use crate::embed::job::{self, EmbedderSource, IndexStatus};
use crate::embed::related::{self, RelatedNote};
use crate::embed::onnx::OnnxEmbedder;
use crate::embed::models::{self, Availability, FileChecks, LoadState, ModelStatus, ModelTracker, SwitchError};
use crate::embed::{self, EmbedError, Embedder};
use crate::session::Session;

/// Emitted with an [`IndexStatus`] whenever it changes.
pub const INDEX_STATUS_EVENT: &str = "index-status";

/// Emitted whenever the related-notes index changed (a note was re-embedded, trashed,
/// restored or deleted, or the ready model changed), so an open Related panel refreshes.
pub const RELATED_CHANGED_EVENT: &str = "related-changed";

/// How often the job checks whether the vault changed.
const POLL: Duration = Duration::from_secs(1);

/// Emitted with a [`ModelStatus`] whenever it changes.
pub const MODEL_STATUS_EVENT: &str = "model-status";

pub struct Indexer {
    status: Mutex<IndexStatus>,
    models: BundledModels,
    /// The unlock whose job is running (0: none), so a switch or a re-check can start
    /// the job again after it stopped on a model that couldn't load; and the unlock
    /// the job last started for.
    running: Mutex<(u64, u64)>,
}

impl Indexer {
    /// `models_dir` is the app's `models` resource folder; `all_cores` the
    /// "use all cores while indexing" choice.
    pub fn new(app: AppHandle, models_dir: PathBuf, all_cores: bool) -> Self {
        Self {
            status: Mutex::new(IndexStatus::Idle),
            models: BundledModels {
                app,
                dir: models_dir,
                all_cores: AtomicBool::new(all_cores),
                loaded: Mutex::default(),
                tracker: ModelTracker::default(),
                checks: FileChecks::default(),
            },
            running: Mutex::new((0, 0)),
        }
    }
}

/// The models shipped with the app. A loaded model holds no note data, so it is kept
/// across lock and unlock, once per core count: the job asks for its model at the
/// start of every pass, so a change of [`BundledModels::all_cores`] applies from the
/// next pass.
struct BundledModels {
    app: AppHandle,
    dir: PathBuf,
    all_cores: AtomicBool,
    loaded: Mutex<HashMap<(&'static str, usize), Arc<dyn Embedder>>>,
    tracker: ModelTracker,
    checks: FileChecks,
}

impl BundledModels {
    /// The threads a model gets, and the machine's cores.
    fn threads(&self) -> (usize, usize) {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = if self.all_cores.load(Ordering::SeqCst) { cores } else { (cores / 2).max(1) };
        (threads, cores)
    }

    /// Sends the model status when `changed`.
    fn emit_if(&self, changed: bool) {
        if changed {
            let _ = self.app.emit(MODEL_STATUS_EVENT, self.tracker.status());
        }
    }
}

impl Indexer {
    /// For the Activity panel: whether `model` is loaded with the current core count,
    /// the threads it uses and the machine's cores.
    pub fn model_state(&self, model: &str) -> (bool, usize, usize) {
        let (threads, cores) = self.models.threads();
        (self.models.is_loaded(model), threads, cores)
    }
}

impl EmbedderSource for BundledModels {
    fn is_loaded(&self, model: &str) -> bool {
        let Some(descriptor) = embed::descriptor(model) else { return false };
        let (threads, _) = self.threads();
        self.loaded.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&(descriptor.id, threads))
    }

    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let Some(descriptor) = embed::descriptor(model) else {
            self.emit_if(self.tracker.load(model, LoadState::Unknown));
            return Err(EmbedError::UnknownModel(model.to_string()));
        };
        let (threads, _) = self.threads();
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = loaded.get(&(descriptor.id, threads)) {
            return Ok(e.clone());
        }
        // Only one core count is kept loaded.
        loaded.clear();
        self.emit_if(self.tracker.load(model, LoadState::Loading));
        match OnnxEmbedder::load_with_threads(&self.dir, descriptor, threads) {
            Ok(e) => {
                let embedder: Arc<dyn Embedder> = Arc::new(e);
                loaded.insert((descriptor.id, threads), embedder.clone());
                self.emit_if(self.tracker.load(model, LoadState::Ready));
                Ok(embedder)
            }
            Err(e) => {
                self.emit_if(self.tracker.load(model, LoadState::from_error(&e)));
                Err(e)
            }
        }
    }

    fn vault_models(&self, model: &str, ready: Option<&str>) {
        self.emit_if(self.tracker.vault_models(model, ready));
    }

    fn measured_rate(&self, model: &str, notes_per_sec: f64) {
        let _ = self.app.state::<Config>().update(|c| {
            c.index_rates.insert(model.to_string(), notes_per_sec);
        });
    }
}

/// Starts the job for unlock `generation`, unless it is already running. It stops by
/// itself when the vault locks, or when its model can't be loaded.
pub fn start(app: &AppHandle, generation: u64) {
    let indexer = app.state::<Indexer>();
    {
        let mut running = indexer.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.0 == generation {
            return;
        }
        if running.1 != generation {
            // The last vault's model settings don't apply to this one.
            indexer.models.emit_if(indexer.models.tracker.forget_vault());
        }
        *running = (generation, generation);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        let indexer = app.state::<Indexer>();
        set_status(&app, &session, generation, &IndexStatus::Idle);
        job::run_with(
            &session,
            generation,
            &indexer.models,
            POLL,
            |status| set_status(&app, &session, generation, status),
            || {
                if session.is_open(generation) {
                    let _ = app.emit(RELATED_CHANGED_EVENT, ());
                }
            },
        );
        let mut running = indexer.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.0 == generation {
            running.0 = 0;
        }
    });
}

fn set_status(app: &AppHandle, session: &Session, generation: u64, status: &IndexStatus) {
    // A job outliving its vault must not report over the next one's.
    if !session.is_open(generation) {
        return;
    }
    *app.state::<Indexer>().status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
    let _ = app.emit(INDEX_STATUS_EVENT, status);
}

/// Whether the indexing job's model uses every core (else half of them).
#[tauri::command]
pub fn index_all_cores(config: State<'_, Config>) -> bool {
    config.get().index_all_cores
}

/// Saves the choice for this machine; the job uses it from its next pass.
#[tauri::command]
pub fn set_index_all_cores(config: State<'_, Config>, indexer: State<'_, Indexer>, on: bool) -> super::Result<()> {
    config.update(|c| c.index_all_cores = on)?;
    indexer.models.all_cores.store(on, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn index_status(indexer: State<'_, Indexer>) -> IndexStatus {
    indexer.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Up to `limit` notes closest in meaning to note `id`, most similar first, with their
/// score and reasons; `None` when the note has no vector yet.
#[tauri::command]
pub fn related_notes(session: State<'_, Session>, id: i64, limit: usize) -> super::Result<Option<Vec<RelatedNote>>> {
    Ok(related::related_notes(&session, id, limit)?)
}

/// The vault's model as the job last saw it; changes arrive as [`MODEL_STATUS_EVENT`].
#[tauri::command]
pub fn model_status(indexer: State<'_, Indexer>) -> ModelStatus {
    indexer.models.tracker.status()
}

/// A model Hush knows, as Settings → Model lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub licence: &'static str,
    pub licence_url: &'static str,
    pub dims: usize,
    pub max_tokens: usize,
    pub availability: Availability,
    /// Notes per second last measured on this machine.
    pub rate: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOverview {
    pub status: ModelStatus,
    /// The vault's model, from its settings (the status may not know it yet).
    pub model: String,
    /// Live notes: what a switch re-embeds.
    pub notes: usize,
    pub models: Vec<ModelInfo>,
    /// Notes per second assumed for a model without a measured rate.
    pub default_rate: f64,
}

/// Everything Settings → Model shows. Checks each model's files (hashing them once
/// per change of size or time).
#[tauri::command]
pub async fn model_overview(
    session: State<'_, Session>,
    indexer: State<'_, Indexer>,
    config: State<'_, Config>,
) -> super::Result<ModelOverview> {
    let (model, notes) = session.with_vault(|v| Ok((v.embedding_model()?, v.live_note_count()?)))?;
    let rates = config.get().index_rates;
    let models = embed::MODELS
        .iter()
        .map(|d| ModelInfo {
            id: d.id,
            name: d.name,
            licence: d.licence,
            licence_url: d.licence_url,
            dims: d.dims,
            max_tokens: d.max_tokens,
            availability: indexer.models.checks.availability(&indexer.models.dir, d),
            rate: rates.get(d.id).copied(),
        })
        .collect();
    Ok(ModelOverview { status: indexer.models.tracker.status(), model, notes, models, default_rate: models::DEFAULT_RATE })
}

/// Switches the vault to `model`, which must be available on this computer: every note
/// is queued for it, and the old model's vectors keep serving until it is done.
#[tauri::command]
pub async fn set_embedding_model(
    app: AppHandle,
    session: State<'_, Session>,
    indexer: State<'_, Indexer>,
    model: String,
) -> super::Result<()> {
    models::switch(&session, &indexer.models.checks, &indexer.models.dir, embed::MODELS, &model).map_err(|e| match e {
        SwitchError::Unknown(id) => CommandError::UnknownModel(id),
        SwitchError::Unavailable(problem) => CommandError::ModelUnavailable(problem),
        SwitchError::Session(e) => CommandError::Session(e),
    })?;
    // A job that stopped on the old model's files starts again.
    start(&app, session.generation());
    Ok(())
}

/// Checks the model files again (after the user put them back) and, if the job had
/// stopped on them, starts it again.
#[tauri::command]
pub fn recheck_models(app: AppHandle, session: State<'_, Session>, indexer: State<'_, Indexer>) {
    indexer.models.checks.forget();
    indexer.models.emit_if(indexer.models.tracker.forget_failures());
    let generation = session.generation();
    if generation != 0 {
        start(&app, generation);
    }
}
