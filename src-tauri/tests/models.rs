//! Settings → Model: switching between models on this computer, and the reduced-
//! functionality state as the indexing job reports it.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use hush_lib::db::Vault;
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::models::{self, FileChecks, LoadState, ModelDirs, ModelTracker, ReducedReason, SwitchError};
use hush_lib::embed::{self, ChunkPooling, EmbedError, Embedder, ModelDescriptor, ModelFile, TokenPooling};
use hush_lib::session::Session;

// sha256("hello") and sha256("{}").
const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
const BRACES: &str = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

const fn stub(id: &'static str, model: &'static str, tokenizer: &'static str, dims: usize) -> ModelDescriptor {
    ModelDescriptor {
        id,
        name: id,
        licence: "MIT",
        licence_url: "",
        bundled: false,
        model: ModelFile { name: model, url: "", sha256: HELLO, size: 5 },
        tokenizer: ModelFile { name: tokenizer, url: "", sha256: BRACES, size: 2 },
        dims,
        max_tokens: 16,
        token_pooling: TokenPooling::Mean,
        chunk_pooling: ChunkPooling::Mean,
        prefix: "",
        query_prefix: "",
        suggestion_threshold: 0.5,
        gpu: false,
        batch_across_notes: false,
    }
}

const CATALOGUE: &[ModelDescriptor] =
    &[stub("stub-4", "stub-4.onnx", "stub-4.json", 4), stub("stub-8", "stub-8.onnx", "stub-8.json", 8)];

/// Optional models live in the user folder; nothing is bundled here.
fn dirs(user: &Path) -> ModelDirs {
    ModelDirs { bundled: user.join("bundled"), user: user.to_path_buf() }
}

fn install(dir: &Path, d: &ModelDescriptor) {
    std::fs::write(dir.join(d.model.name), b"hello").unwrap();
    std::fs::write(dir.join(d.tokenizer.name), b"{}").unwrap();
}

/// Vectors of the descriptor's length from the text's bytes.
struct Fake(&'static ModelDescriptor);

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        self.0.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.01; self.0.dims];
        for (i, b) in text.bytes().enumerate() {
            v[i % self.0.dims] += f32::from(b);
        }
        embed::normalize(&mut v);
        Ok(v)
    }
}

/// Loads like the app: only models whose files check out, reporting to a tracker.
struct Source<'a> {
    dir: &'a Path,
    checks: FileChecks,
    tracker: ModelTracker,
    vault_reports: Mutex<Vec<(String, Option<String>)>>,
}

impl EmbedderSource for Source<'_> {
    fn embedder(&self, model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        let d = CATALOGUE.iter().find(|d| d.id == model).ok_or_else(|| EmbedError::UnknownModel(model.into()))?;
        let result = match self.checks.availability(self.dir, d) {
            models::Availability::Available { .. } => Ok(Arc::new(Fake(d)) as Arc<dyn Embedder>),
            models::Availability::Missing { file } => Err(EmbedError::Missing(self.dir.join(file))),
            _ => Err(EmbedError::Checksum(self.dir.join(d.model.name))),
        };
        match &result {
            Ok(_) => self.tracker.load(model, LoadState::Ready),
            Err(e) => self.tracker.load(model, LoadState::from_error(e)),
        };
        result
    }

    fn vault_models(&self, model: &str, ready: Option<&str>) {
        self.vault_reports.lock().unwrap().push((model.into(), ready.map(Into::into)));
        self.tracker.vault_models(model, ready);
    }
}

fn index(session: &Session, g: u64, source: &dyn EmbedderSource) {
    for _ in 0..10 {
        if job::pass(session, g, source, &mut |_| {}).unwrap().stale == 0 {
            return;
        }
    }
    panic!("indexing never finished");
}

fn ready(session: &Session) -> Option<String> {
    session.with_vault(|v| v.embedding_model_ready()).unwrap()
}

#[test]
fn switching_between_two_available_models_works_end_to_end() {
    let models = tempfile::tempdir().unwrap();
    install(models.path(), &CATALOGUE[0]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model("stub-4").unwrap();
    let session = Session::default();
    let g = session.unlock(vault, path).unwrap();
    let ids: Vec<i64> = session
        .with_vault(|v| (0..5).map(|i| v.create_note("", &format!("N{i}"), &format!("text {i}"))).collect())
        .unwrap();
    let source = Source {
        dir: models.path(),
        checks: FileChecks::default(),
        tracker: ModelTracker::default(),
        vault_reports: Mutex::default(),
    };

    // Before the first pass the tracker knows nothing; after the first index nothing is off.
    assert_eq!(source.tracker.status().reduced, None);
    job::pass(&session, g, &source, &mut |_| {}).unwrap();
    assert_eq!(
        source.vault_reports.lock().unwrap().first(),
        Some(&("stub-4".to_string(), None)),
        "the pass reports the vault's models before its first index"
    );
    index(&session, g, &source);
    assert_eq!(ready(&session).as_deref(), Some("stub-4"));
    assert_eq!(source.tracker.status().ready_model.as_deref(), Some("stub-4"));
    assert_eq!(source.tracker.status().reduced, None);

    // Not on this computer, or unknown: refused, and the vault keeps its model.
    let refused = models::switch(&session, &source.checks, &dirs(models.path()), CATALOGUE, "stub-8");
    assert!(matches!(&refused, Err(SwitchError::Unavailable(p)) if p.contains("stub-8.json is missing")), "{refused:?}");
    let unknown = models::switch(&session, &source.checks, &dirs(models.path()), CATALOGUE, "nope");
    assert!(matches!(unknown, Err(SwitchError::Unknown(_))));
    assert_eq!(session.with_vault(|v| v.embedding_model()).unwrap(), "stub-4");

    // Installed: the switch queues every note; the old vectors serve meanwhile.
    install(models.path(), &CATALOGUE[1]);
    models::switch(&session, &source.checks, &dirs(models.path()), CATALOGUE, "stub-8").unwrap();
    assert_eq!(session.with_vault(|v| v.embedding_queue_len()).unwrap(), ids.len());
    assert_eq!(session.with_vault(|v| v.note_embedding(ids[0])).unwrap().unwrap().len(), 4);
    source.tracker.vault_models("stub-8", Some("stub-4"));
    assert_eq!(source.tracker.status().reduced, None, "mid-switch nothing is off");

    index(&session, g, &source);
    assert_eq!(ready(&session).as_deref(), Some("stub-8"));
    assert_eq!(source.tracker.status().ready_model.as_deref(), Some("stub-8"));
    for &id in &ids {
        assert_eq!(session.with_vault(|v| v.note_embedding(id)).unwrap().unwrap().len(), 8);
    }

    // And back again.
    models::switch(&session, &source.checks, &dirs(models.path()), CATALOGUE, "stub-4").unwrap();
    index(&session, g, &source);
    assert_eq!(ready(&session).as_deref(), Some("stub-4"));
}

#[test]
fn a_missing_model_shows_as_reduced_functionality_with_the_file_named() {
    let models = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model("stub-4").unwrap();
    let session = Session::default();
    let g = session.unlock(vault, path).unwrap();
    session.with_vault(|v| v.create_note("", "T", "Text")).unwrap();
    let source = Source {
        dir: models.path(),
        checks: FileChecks::default(),
        tracker: ModelTracker::default(),
        vault_reports: Mutex::default(),
    };

    assert!(job::pass(&session, g, &source, &mut |_| {}).is_err());
    let reduced = source.tracker.status().reduced.unwrap();
    assert_eq!(reduced.reason, ReducedReason::Missing);
    assert_eq!(reduced.message, "model file stub-4.json is missing");

    // The files are put back: the next pass loads it and runs the first index.
    install(models.path(), &CATALOGUE[0]);
    source.checks.forget();
    source.tracker.forget_failures();
    index(&session, g, &source);
    assert_eq!(source.tracker.status().reduced, None);
}
