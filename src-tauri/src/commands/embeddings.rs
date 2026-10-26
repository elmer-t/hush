//! The indexing job (embeddings) as the app runs it: started on unlock, with the
//! bundled model, reporting its status to the frontend. Also the related-notes lookup
//! the job's vectors feed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::Config;
use crate::embed::job::{self, EmbedderSource, IndexStatus};
use crate::embed::related::{self, RelatedNote};
use crate::embed::onnx::OnnxEmbedder;
use crate::embed::{self, EmbedError, Embedder};
use crate::session::Session;

/// Emitted with an [`IndexStatus`] whenever it changes.
pub const INDEX_STATUS_EVENT: &str = "index-status";

/// Emitted whenever the related-notes index changed (a note was re-embedded, trashed,
/// restored or deleted, or the ready model changed), so an open Related panel refreshes.
pub const RELATED_CHANGED_EVENT: &str = "related-changed";

/// How often the job checks whether the vault changed.
const POLL: Duration = Duration::from_secs(1);

pub struct Indexer {
    status: Mutex<IndexStatus>,
    models: BundledModels,
}

impl Indexer {
    /// `models_dir` is the app's `models` resource folder; `all_cores` the
    /// "use all cores while indexing" choice.
    pub fn new(models_dir: PathBuf, all_cores: bool) -> Self {
        Self {
            status: Mutex::new(IndexStatus::Idle),
            models: BundledModels { dir: models_dir, all_cores: AtomicBool::new(all_cores), loaded: Mutex::default() },
        }
    }
}

/// The models shipped with the app. A loaded model holds no note data, so it is kept
/// across lock and unlock, once per core count: the job asks for its model at the
/// start of every pass, so a change of [`BundledModels::all_cores`] applies from the
/// next pass.
struct BundledModels {
    dir: PathBuf,
    all_cores: AtomicBool,
    loaded: Mutex<HashMap<(&'static str, usize), Arc<dyn Embedder>>>,
}

impl BundledModels {
    /// The threads a model gets, and the machine's cores.
    fn threads(&self) -> (usize, usize) {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = if self.all_cores.load(Ordering::SeqCst) { cores } else { (cores / 2).max(1) };
        (threads, cores)
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
        let descriptor = embed::descriptor(model).ok_or_else(|| EmbedError::UnknownModel(model.to_string()))?;
        let (threads, _) = self.threads();
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = loaded.get(&(descriptor.id, threads)) {
            return Ok(e.clone());
        }
        // Only one core count is kept loaded.
        loaded.clear();
        let embedder: Arc<dyn Embedder> = Arc::new(OnnxEmbedder::load_with_threads(&self.dir, descriptor, threads)?);
        loaded.insert((descriptor.id, threads), embedder.clone());
        Ok(embedder)
    }
}

/// Starts the job for unlock `generation`. It stops by itself when the vault locks.
pub fn start(app: &AppHandle, generation: u64) {
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
