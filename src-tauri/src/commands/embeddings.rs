//! The indexing job (embeddings) as the app runs it: started on unlock, with the
//! bundled model, reporting its status to the frontend.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::embed::job::{self, EmbedderSource, IndexStatus};
use crate::embed::onnx::OnnxEmbedder;
use crate::embed::{self, EmbedError, Embedder};
use crate::session::Session;

/// Emitted with an [`IndexStatus`] whenever it changes.
pub const INDEX_STATUS_EVENT: &str = "index-status";

/// How often the job checks whether the vault changed.
const POLL: Duration = Duration::from_secs(1);

pub struct Indexer {
    status: Mutex<IndexStatus>,
    models: BundledModels,
}

impl Indexer {
    /// `models_dir` is the app's `models` resource folder.
    pub fn new(models_dir: PathBuf) -> Self {
        Self { status: Mutex::new(IndexStatus::Idle), models: BundledModels { dir: models_dir, loaded: Mutex::default() } }
    }
}

/// The models shipped with the app. A loaded model holds no note data, so it is kept
/// across lock and unlock.
struct BundledModels {
    dir: PathBuf,
    loaded: Mutex<HashMap<&'static str, Arc<dyn Embedder>>>,
}

impl EmbedderSource for BundledModels {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let descriptor = embed::descriptor(model).ok_or_else(|| EmbedError::UnknownModel(model.to_string()))?;
        let mut loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = loaded.get(descriptor.id) {
            return Ok(e.clone());
        }
        let embedder: Arc<dyn Embedder> = Arc::new(OnnxEmbedder::load(&self.dir, descriptor)?);
        loaded.insert(descriptor.id, embedder.clone());
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
        job::run(&session, generation, &indexer.models, POLL, |status| {
            set_status(&app, &session, generation, status);
        });
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

#[tauri::command]
pub fn index_status(indexer: State<'_, Indexer>) -> IndexStatus {
    indexer.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
