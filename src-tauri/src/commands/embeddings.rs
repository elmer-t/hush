//! The indexing job (embeddings) as the app runs it: started on unlock, with the
//! vault's model from this computer's model folders, reporting its status to the
//! frontend. Also the related-notes lookup
//! the job's vectors feed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::CommandError;

use crate::config::Config;
use crate::embed::gpu::{self, Adapter};
use crate::embed::job::{self, EmbedderSource, IndexStatus};
use crate::embed::related::{self, RelatedNote};
use crate::embed::onnx::{Device, OnnxEmbedder};
use crate::embed::models::{self, Availability, FileChecks, LoadState, ModelDirs, ModelStatus, ModelTracker, SwitchError};
use crate::embed::{self, EmbedError, Embedder};
use crate::meaning::QueryModels;
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

/// Emitted with a [`GpuStatus`] whenever a model loads, or the GPU setting changes.
pub const GPU_STATUS_EVENT: &str = "gpu-status";

pub struct Indexer {
    status: Mutex<IndexStatus>,
    pub(super) models: LocalModels,
    /// The unlock whose job is running (0: none), so a switch or a re-check can start
    /// the job again after it stopped on a model that couldn't load; and the unlock
    /// the job last started for.
    running: Mutex<(u64, u64)>,
}

impl Indexer {
    /// `dirs` are the bundled and the downloaded models' folders; `all_cores` the
    /// "use all cores while indexing" choice; `gpu_off` the "don't use the GPU" one.
    pub fn new(app: AppHandle, dirs: ModelDirs, all_cores: bool, gpu_off: bool) -> Self {
        Self {
            status: Mutex::new(IndexStatus::Idle),
            models: LocalModels {
                app,
                dirs,
                all_cores: AtomicBool::new(all_cores),
                gpu_off: AtomicBool::new(gpu_off),
                gpu: Mutex::default(),
                loaded: Mutex::default(),
                query: Mutex::default(),
                tracker: ModelTracker::default(),
                checks: FileChecks::default(),
            },
            running: Mutex::new((0, 0)),
        }
    }
}

/// The models on this computer: bundled with the app, downloaded or imported. A loaded
/// model holds no note data, so it is kept across lock and unlock, once per core count
/// and device: the job asks for its model at the start of every pass, so a change of
/// [`LocalModels::all_cores`] or [`LocalModels::gpu_off`] applies from the next pass.
///
/// A model made for the GPU ([`embed::ModelDescriptor::gpu`]) is loaded on the GPU
/// through DirectML unless Settings turned that off; if that fails it is loaded on the
/// CPU instead, and the GPU isn't tried again for it until the setting changes or the
/// models are checked again.
pub(super) struct LocalModels {
    app: AppHandle,
    pub(super) dirs: ModelDirs,
    all_cores: AtomicBool,
    gpu_off: AtomicBool,
    /// Where the last model loaded, and why a GPU model isn't on the GPU.
    gpu: Mutex<GpuState>,
    loaded: Mutex<HashMap<(&'static str, usize, Device), Arc<dyn Embedder>>>,
    /// A second, small CPU session of the ready model for search queries (#146): the
    /// job's session runs one batch at a time behind a lock (ort's `Session::run` takes
    /// `&mut self`), so a query would wait for the batch running, and a GPU session
    /// can't be shared with the job's runs either.
    query: Mutex<Option<(&'static str, Arc<dyn Embedder>)>>,
    pub(super) tracker: ModelTracker,
    pub(super) checks: FileChecks,
}

/// What the GPU is doing for indexing, for Settings → Indexing.
#[derive(Debug, Default, Clone)]
struct GpuState {
    /// The model last loaded, and where.
    loaded: Option<(&'static str, Device)>,
    /// A GPU model that failed to load on the GPU, and the error.
    failed: Option<(&'static str, String)>,
}

/// Settings → Indexing's GPU line.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuStatus {
    /// The GPUs Windows reports (none off Windows).
    pub adapters: Vec<Adapter>,
    /// The discrete GPU a GPU model would use, if any.
    pub best: Option<Adapter>,
    /// Settings allows the GPU.
    pub enabled: bool,
    /// The model last loaded, where it runs, and why not on the GPU when it could have.
    pub model: Option<String>,
    pub device: Option<Device>,
    pub reason: Option<String>,
    /// The name of the model made for the GPU, so Settings can say which one to pick.
    pub gpu_model: Option<&'static str>,
}

impl GpuState {
    /// Where `descriptor` should load, and why not on the GPU when it is a GPU model;
    /// `gpu_off` is the Settings choice.
    fn placement(&self, descriptor: &embed::ModelDescriptor, on_windows: bool, gpu_off: bool) -> (Device, Option<String>) {
        if !descriptor.gpu {
            return (Device::Cpu, None);
        }
        if !on_windows {
            return (Device::Cpu, Some("the GPU is only used on Windows".into()));
        }
        if gpu_off {
            return (
                Device::Cpu,
                Some("turned off in Settings → Indexing".into()),
            );
        }
        match &self.failed {
            Some((id, error)) if *id == descriptor.id => (Device::Cpu, Some(error.clone())),
            _ => (Device::Gpu, None),
        }
    }
}

/// Loads with `load` on `device`. A GPU load that fails tells `gpu_failed` why and
/// loads on the CPU instead; returns the result and the device it was loaded on.
fn load_falling_back<T>(
    device: Device,
    mut load: impl FnMut(Device) -> Result<T, EmbedError>,
    gpu_failed: impl FnOnce(&str),
) -> (Result<T, EmbedError>, Device) {
    let result = load(device);
    if device == Device::Gpu
        && let Err(EmbedError::Load(error)) = &result
    {
        gpu_failed(error);
        return (load(Device::Cpu), Device::Cpu);
    }
    (result, device)
}

/// The job's run state: the unlock whose job is running (0: none) and the unlock it
/// last started for. Claims it for unlock `generation`: `None` when the job already
/// runs for it, else whether it last started for another unlock.
fn claim_run(running: &mut (u64, u64), generation: u64) -> Option<bool> {
    if running.0 == generation {
        return None;
    }
    let other_unlock = running.1 != generation;
    *running = (generation, generation);
    Some(other_unlock)
}

/// The job of unlock `generation` stopped (see [`claim_run`]).
fn release_run(running: &mut (u64, u64), generation: u64) {
    if running.0 == generation {
        running.0 = 0;
    }
}

impl LocalModels {
    /// Where `descriptor` should load, and why not on the GPU when it is a GPU model.
    fn device_for(&self, descriptor: &'static embed::ModelDescriptor) -> (Device, Option<String>) {
        let gpu_off = self.gpu_off.load(Ordering::SeqCst);
        self.gpu.lock().unwrap_or_else(|e| e.into_inner()).placement(descriptor, cfg!(windows), gpu_off)
    }

    /// Keeps the model that loaded as the one loaded under `key`, and reports how
    /// loading `model` went.
    fn finish_load(
        &self,
        model: &str,
        loaded: &mut HashMap<(&'static str, usize, Device), Arc<dyn Embedder>>,
        key: (&'static str, usize, Device),
        result: Result<OnnxEmbedder, EmbedError>,
    ) -> Result<Arc<dyn Embedder>, EmbedError> {
        match result {
            Ok(e) => {
                let embedder: Arc<dyn Embedder> = Arc::new(e);
                loaded.insert(key, embedder.clone());
                self.gpu.lock().unwrap_or_else(|e| e.into_inner()).loaded = Some((key.0, key.2));
                self.emit_gpu();
                self.emit_if(self.tracker.load(model, LoadState::Ready));
                Ok(embedder)
            }
            Err(e) => {
                self.emit_if(self.tracker.load(model, LoadState::from_error(&e)));
                Err(e)
            }
        }
    }

    pub(super) fn gpu_status(&self) -> GpuStatus {
        let adapters = gpu::adapters();
        let best = gpu::best(&adapters).cloned();
        let state = self.gpu.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (model, device, reason) = match state.loaded {
            Some((id, device)) => {
                let reason = embed::descriptor(id).and_then(|d| self.device_for(d).1);
                (Some(id.to_string()), Some(device), reason)
            }
            None => (None, None, None),
        };
        GpuStatus {
            adapters,
            best,
            enabled: !self.gpu_off.load(Ordering::SeqCst),
            model,
            device,
            reason,
            gpu_model: embed::MODELS.iter().find(|m| m.gpu).map(|m| m.name),
        }
    }

    fn emit_gpu(&self) {
        let _ = self.app.emit(GPU_STATUS_EVENT, self.gpu_status());
    }

    /// The threads a model gets, and the machine's cores.
    fn threads(&self) -> (usize, usize) {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = if self.all_cores.load(Ordering::SeqCst) { cores } else { (cores / 2).max(1) };
        (threads, cores)
    }

    /// Lets a GPU model try the GPU again (the setting changed, or the models were
    /// checked again).
    fn retry_gpu(&self) {
        self.gpu.lock().unwrap_or_else(|e| e.into_inner()).failed = None;
    }

    /// Drops `model` from memory (its files are being removed).
    pub(super) fn unload(&self, model: &str) {
        self.loaded.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| k.0 != model);
        let mut query = self.query.lock().unwrap_or_else(|e| e.into_inner());
        if query.as_ref().is_some_and(|(id, _)| *id == model) {
            *query = None;
        }
    }

    /// Sends the model status when `changed`.
    pub(super) fn emit_if(&self, changed: bool) {
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

/// Threads of the query session: a query is one short chunk, so more threads mostly
/// add start-up cost, and the job keeps the other cores.
const QUERY_THREADS: usize = 2;

impl QueryModels for LocalModels {
    fn query_loaded(&self, model: &str) -> bool {
        self.query.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(id, _)| *id == model)
    }

    fn query_embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let descriptor = embed::descriptor(model).ok_or_else(|| EmbedError::UnknownModel(model.to_string()))?;
        // Held while loading, so two searches don't load it twice.
        let mut query = self.query.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, e)) = query.as_ref() {
            if *id == descriptor.id {
                return Ok(e.clone());
            }
        }
        *query = None;
        let dir = self
            .checks
            .locate(&self.dirs, descriptor)
            .1
            .unwrap_or_else(|| self.dirs.search(descriptor)[0].to_path_buf());
        let embedder: Arc<dyn Embedder> = Arc::new(OnnxEmbedder::load_on(&dir, descriptor, QUERY_THREADS, Device::Cpu)?);
        *query = Some((descriptor.id, embedder.clone()));
        Ok(embedder)
    }
}

impl EmbedderSource for LocalModels {
    fn is_loaded(&self, model: &str) -> bool {
        let Some(descriptor) = embed::descriptor(model) else { return false };
        let (threads, _) = self.threads();
        let (device, _) = self.device_for(descriptor);
        self.loaded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&(descriptor.id, threads, device))
    }

    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let Some(descriptor) = embed::descriptor(model) else {
            self.emit_if(self.tracker.load(model, LoadState::Unknown));
            return Err(EmbedError::UnknownModel(model.to_string()));
        };
        let (threads, _) = self.threads();
        let (device, _) = self.device_for(descriptor);
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = loaded.get(&(descriptor.id, threads, device)) {
            return Ok(e.clone());
        }
        // Only one core count and device is kept loaded.
        loaded.clear();
        self.emit_if(self.tracker.load(model, LoadState::Loading));
        // The folder that has the files intact, else the likeliest one (for the error).
        let dir = self
            .checks
            .locate(&self.dirs, descriptor)
            .1
            .unwrap_or_else(|| self.dirs.search(descriptor)[0].to_path_buf());
        let (result, device) = load_falling_back(
            device,
            |device| OnnxEmbedder::load_on(&dir, descriptor, threads, device),
            |error| {
                // No usable GPU: the CPU, and not the GPU again until something changes.
                eprintln!("hush: {} runs on the CPU: {error}", descriptor.id);
                self.gpu.lock().unwrap_or_else(|e| e.into_inner()).failed = Some((descriptor.id, error.to_string()));
            },
        );
        self.finish_load(model, &mut loaded, (descriptor.id, threads, device), result)
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
        let Some(other_unlock) = claim_run(&mut running, generation) else {
            return;
        };
        if other_unlock {
            // The last vault's model settings don't apply to this one.
            indexer.models.emit_if(indexer.models.tracker.forget_vault());
        }
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
            || related_changed(&app, &session, generation),
        );
        release_run(&mut indexer.running.lock().unwrap_or_else(|e| e.into_inner()), generation);
    });
}

fn related_changed(app: &AppHandle, session: &Session, generation: u64) {
    // A job outliving its vault must not report over the next one's.
    if session.is_open(generation) {
        let _ = app.emit(RELATED_CHANGED_EVENT, ());
    }
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

/// Whether models made for the GPU may use it, which GPUs there are, and where the
/// last model loaded.
#[tauri::command]
pub fn gpu_status(indexer: State<'_, Indexer>) -> GpuStatus {
    indexer.models.gpu_status()
}

/// Saves the GPU choice for this machine; the job uses it from its next pass (a GPU
/// model is then reloaded on the new device).
#[tauri::command]
pub fn set_index_gpu(
    app: AppHandle,
    config: State<'_, Config>,
    indexer: State<'_, Indexer>,
    on: bool,
) -> super::Result<()> {
    config.update(|c| c.index_gpu_off = !on)?;
    indexer.models.gpu_off.store(!on, Ordering::SeqCst);
    indexer.models.retry_gpu();
    indexer.models.emit_gpu();
    // A job that stopped on the model starts again.
    models_changed(&app);
    Ok(())
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
#[tauri::command(async)]
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
    /// Shipped with the app; otherwise it can be downloaded or imported.
    pub bundled: bool,
    /// Bytes to download.
    pub size: u64,
    /// Where a download comes from.
    pub host: String,
    pub availability: Availability,
    /// Notes per second last measured on this machine.
    pub rate: Option<f64>,
    /// Made for the GPU (see [`GpuStatus`]).
    pub gpu: bool,
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
            bundled: d.bundled,
            size: d.model.size + d.tokenizer.size,
            host: host(d.model.url),
            availability: indexer.models.checks.locate(&indexer.models.dirs, d).0,
            rate: rates.get(d.id).copied(),
            gpu: d.gpu,
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
    models::switch(&session, &indexer.models.checks, &indexer.models.dirs, embed::MODELS, &model).map_err(|e| match e {
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
pub fn recheck_models(app: AppHandle) {
    models_changed(&app);
}

/// Model files were added, removed or put back: checks them again, and starts the job
/// if it had stopped on a model that couldn't load.
pub(super) fn models_changed(app: &AppHandle) {
    let indexer = app.state::<Indexer>();
    indexer.models.checks.forget();
    indexer.models.retry_gpu();
    indexer
        .models
        .emit_if(indexer.models.tracker.forget_failures());
    let generation = app.state::<Session>().generation();
    if generation != 0 {
        start(app, generation);
    }
}

fn host(url: &str) -> String {
    url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    fn gpu_model() -> &'static embed::ModelDescriptor {
        embed::MODELS.iter().find(|m| m.gpu).expect("a GPU model")
    }

    fn cpu_model() -> &'static embed::ModelDescriptor {
        embed::MODELS.iter().find(|m| !m.gpu).expect("a CPU model")
    }

    #[test]
    fn a_cpu_model_always_loads_on_the_cpu() {
        assert_eq!(GpuState::default().placement(cpu_model(), true, false), (Device::Cpu, None));
    }

    #[test]
    fn a_gpu_model_loads_on_the_gpu_unless_something_says_otherwise() {
        let gpu = GpuState::default();
        assert_eq!(gpu.placement(gpu_model(), true, false), (Device::Gpu, None));
        assert_eq!(
            gpu.placement(gpu_model(), false, false),
            (Device::Cpu, Some("the GPU is only used on Windows".into()))
        );
        assert_eq!(
            gpu.placement(gpu_model(), true, true),
            (Device::Cpu, Some("turned off in Settings → Indexing".into()))
        );
    }

    #[test]
    fn a_gpu_model_that_failed_on_the_gpu_stays_on_the_cpu() {
        let failed = GpuState { loaded: None, failed: Some((gpu_model().id, "no adapter".into())) };
        assert_eq!(failed.placement(gpu_model(), true, false), (Device::Cpu, Some("no adapter".into())));
        let other = GpuState { loaded: None, failed: Some(("another model", "no adapter".into())) };
        assert_eq!(other.placement(gpu_model(), true, false), (Device::Gpu, None));
    }

    #[test]
    fn a_failed_gpu_load_falls_back_to_the_cpu() {
        let tried = RefCell::new(Vec::new());
        let mut failure = None;
        let (result, device) = load_falling_back(
            Device::Gpu,
            |d| {
                tried.borrow_mut().push(d);
                match d {
                    Device::Gpu => Err(EmbedError::Load("no adapter".into())),
                    Device::Cpu => Ok("on the cpu"),
                }
            },
            |error| failure = Some(error.to_string()),
        );
        assert_eq!(result.unwrap(), "on the cpu");
        assert_eq!(device, Device::Cpu);
        assert_eq!(*tried.borrow(), vec![Device::Gpu, Device::Cpu]);
        assert_eq!(failure.as_deref(), Some("no adapter"));
    }

    #[test]
    fn a_gpu_load_that_works_stays_on_the_gpu() {
        let (result, device) = load_falling_back(Device::Gpu, |_| Ok("on the gpu"), |_| panic!("no failure"));
        assert_eq!(result.unwrap(), "on the gpu");
        assert_eq!(device, Device::Gpu);
    }

    #[test]
    fn only_a_gpu_load_error_falls_back() {
        let missing = || EmbedError::Missing(PathBuf::from("model.onnx"));
        let (result, device) = load_falling_back::<()>(Device::Gpu, |_| Err(missing()), |_| panic!("no fallback"));
        assert!(matches!(result, Err(EmbedError::Missing(_))));
        assert_eq!(device, Device::Gpu);

        let mut calls = 0;
        let (result, device) = load_falling_back::<()>(
            Device::Cpu,
            |_| {
                calls += 1;
                Err(EmbedError::Load("broken".into()))
            },
            |_| panic!("no fallback"),
        );
        assert!(matches!(result, Err(EmbedError::Load(_))));
        assert_eq!((device, calls), (Device::Cpu, 1));
    }

    #[test]
    fn the_job_runs_once_per_unlock() {
        let mut running = (0, 0);
        assert_eq!(claim_run(&mut running, 1), Some(true));
        assert_eq!(claim_run(&mut running, 1), None);
        release_run(&mut running, 1);
        assert_eq!(running, (0, 1));
        // Started again for the same vault (a switch, or a re-check).
        assert_eq!(claim_run(&mut running, 1), Some(false));
        // Another unlock.
        assert_eq!(claim_run(&mut running, 2), Some(true));
    }

    #[test]
    fn an_old_job_stopping_leaves_the_new_one_running() {
        let mut running = (0, 0);
        claim_run(&mut running, 1);
        claim_run(&mut running, 2);
        release_run(&mut running, 1);
        assert_eq!(running, (2, 2));
    }
}
