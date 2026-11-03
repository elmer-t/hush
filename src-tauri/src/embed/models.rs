//! What Settings → Model shows: which models this computer has, the state of the
//! vault's model, and why features are off while it isn't ready (see
//! `docs/decisions.md`, Model section). Nothing here touches the network: a model is
//! available when its files are in the models folder with the pinned checksums.
//!
//! The state comes from what the indexing job already knows (the vault's model
//! settings it reads at the start of every pass, and how loading the model went), so
//! showing it costs no vault access.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{EmbedError, ModelDescriptor};
use crate::session::{Session, SessionError};

/// Notes per second assumed for a model not yet measured on this machine: the bundled
/// model on the Windows dev machine (`docs/decisions.md`, Scale).
pub const DEFAULT_RATE: f64 = 2.5;

/// Where model files are: the app's resource folder (the bundled default) and the
/// app data `models` folder (downloaded or imported models, per machine, outside any
/// vault).
#[derive(Debug, Clone)]
pub struct ModelDirs {
    pub bundled: PathBuf,
    pub user: PathBuf,
}

impl ModelDirs {
    /// The folders a model's files are looked for in, most likely first. Both are
    /// searched, so a damaged install can be fixed by importing the default model's files.
    pub fn search(&self, model: &ModelDescriptor) -> [&Path; 2] {
        if model.bundled {
            [&self.bundled, &self.user]
        } else {
            [&self.user, &self.bundled]
        }
    }
}

/// Whether a model's files are on this computer, intact.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Availability {
    /// `bytes`: the files' size on disk; `downloaded`: they are in the app data folder
    /// (downloaded or imported), so they can be removed.
    Available {
        bytes: u64,
        downloaded: bool,
    },
    Missing {
        file: String,
    },
    Checksum {
        file: String,
    },
    Unreadable {
        file: String,
        message: String,
    },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    fn downloaded(self, downloaded: bool) -> Self {
        match self {
            Self::Available { bytes, .. } => Self::Available { bytes, downloaded },
            other => other,
        }
    }

    /// Why the model can't be used, for an error message; `None` when it can.
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::Available { .. } => None,
            Self::Missing { file } => Some(format!("model file {file} is missing")),
            Self::Checksum { file } => Some(format!("model file {file} failed its checksum")),
            Self::Unreadable { file, message } => {
                Some(format!("could not read model file {file}: {message}"))
            }
        }
    }
}

/// Checks model files against their pinned SHA-256. A file's result is kept until its
/// size or modification time changes, so opening Settings doesn't hash tens of MB
/// every time.
#[derive(Default)]
pub struct FileChecks {
    seen: Mutex<Checked>,
}

/// By path and expected checksum: the size and time the file had, and whether it matched.
type Checked = HashMap<(PathBuf, &'static str), (u64, Option<SystemTime>, bool)>;

impl FileChecks {
    /// Looks for `descriptor`'s files in `dirs`: the folder that has all of them intact,
    /// or why none does (as found in the most likely folder).
    pub fn locate(
        &self,
        dirs: &ModelDirs,
        descriptor: &ModelDescriptor,
    ) -> (Availability, Option<PathBuf>) {
        let [first, second] = dirs.search(descriptor);
        let found = self.availability(first, descriptor);
        if found.is_available() {
            return (
                found.downloaded(first == dirs.user),
                Some(first.to_path_buf()),
            );
        }
        let other = self.availability(second, descriptor);
        if other.is_available() {
            return (
                other.downloaded(second == dirs.user),
                Some(second.to_path_buf()),
            );
        }
        (found, None)
    }

    /// Checks `descriptor`'s files in one folder.
    pub fn availability(&self, dir: &Path, descriptor: &ModelDescriptor) -> Availability {
        let mut bytes = 0;
        for file in [descriptor.tokenizer, descriptor.model] {
            let path = dir.join(file.name);
            let name = file.name.to_string();
            let meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Availability::Missing { file: name };
                }
                Err(e) => {
                    return Availability::Unreadable {
                        file: name,
                        message: e.to_string(),
                    };
                }
            };
            let stamp = (meta.len(), meta.modified().ok());
            let key = (path.clone(), file.sha256);
            let cached = self
                .lock()
                .get(&key)
                .filter(|c| (c.0, c.1) == stamp)
                .map(|c| c.2);
            let ok = match cached {
                Some(ok) => ok,
                None => match sha256_hex(&path) {
                    Ok(hex) => {
                        let ok = hex == file.sha256;
                        self.lock().insert(key, (stamp.0, stamp.1, ok));
                        ok
                    }
                    Err(e) => {
                        return Availability::Unreadable {
                            file: name,
                            message: e.to_string(),
                        };
                    }
                },
            };
            if !ok {
                return Availability::Checksum { file: name };
            }
            bytes += meta.len();
        }
        Availability::Available {
            bytes,
            downloaded: false,
        }
    }

    /// Forgets every result (the user asked to check again).
    pub fn forget(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Checked> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Lowercase hex SHA-256 of a file, read in pieces.
pub fn sha256_hex(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error("this version of Hush doesn't know model “{0}”")]
    Unknown(String),
    /// Why the model's files can't be used.
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Switches the open vault to model `id` from `catalogue`, whose files must be in one of
/// `dirs` with the right checksums. Every live note is queued for it; until the new vectors
/// are complete the old model's keep serving.
pub fn switch(
    session: &Session,
    checks: &FileChecks,
    dirs: &ModelDirs,
    catalogue: &[ModelDescriptor],
    id: &str,
) -> Result<(), SwitchError> {
    let descriptor = catalogue
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| SwitchError::Unknown(id.to_string()))?;
    if let Some(problem) = checks.locate(dirs, descriptor).0.problem() {
        return Err(SwitchError::Unavailable(problem));
    }
    session.with_vault(|v| v.set_embedding_model(descriptor.id))?;
    Ok(())
}

/// Why `model`'s files can't be removed, if they can't: it ships with Hush, or the open
/// vault embeds with it or its vectors serve (named by the vault's file name).
pub fn removal_blocked(
    session: &Session,
    model: &ModelDescriptor,
) -> Result<Option<String>, SessionError> {
    if model.bundled {
        return Ok(Some(format!(
            "{} ships with Hush and can't be removed",
            model.name
        )));
    }
    let Some(path) = session.path() else {
        return Ok(None);
    };
    let (active, ready) =
        session.with_vault(|v| Ok((v.embedding_model()?, v.embedding_model_ready()?)))?;
    if active == model.id || ready.as_deref() == Some(model.id) {
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        return Ok(Some(format!(
            "the open vault “{name}” uses {}; switch it to another model first",
            model.name
        )));
    }
    Ok(None)
}

/// How loading a model went, this session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LoadState {
    /// Not asked for yet.
    NotLoaded,
    Loading,
    Ready,
    Missing {
        file: String,
    },
    Checksum {
        file: String,
    },
    /// The vault names a model this version of Hush doesn't know.
    Unknown,
    Failed {
        message: String,
    },
}

impl LoadState {
    pub fn from_error(e: &EmbedError) -> Self {
        let file = |p: &Path| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        };
        match e {
            EmbedError::Missing(p) => Self::Missing { file: file(p) },
            EmbedError::Checksum(p) => Self::Checksum { file: file(p) },
            EmbedError::UnknownModel(_) => Self::Unknown,
            e => Self::Failed {
                message: e.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReducedReason {
    Missing,
    Checksum,
    Unknown,
    Failed,
    Loading,
    /// No model's vectors cover every note yet.
    FirstIndex,
}

/// Why related notes, tag suggestions, tag classifiers and Discover tags are off.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reduced {
    pub reason: ReducedReason,
    /// One short line, e.g. `model file x.onnx is missing`.
    pub message: String,
}

/// The vault's model as the job last saw it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// The model the vault embeds with (`embedding_model`); `None` until the job has
    /// read it this unlock.
    pub model: Option<String>,
    /// The model whose vectors cover every note (`embedding_model_ready`).
    pub ready_model: Option<String>,
    /// How loading `model` went.
    pub load: LoadState,
    /// Set while features are off; see [`reduced`].
    pub reduced: Option<Reduced>,
}

/// Whether features that need vectors are off, and why. They are while the vault's
/// model can't be loaded, and while no model's vectors cover every note yet (the
/// first index). During a switch the old model's vectors keep serving, so a switch
/// alone doesn't reduce anything.
pub fn reduced(model: Option<&str>, ready: Option<&str>, load: &LoadState) -> Option<Reduced> {
    let model = model?;
    let (reason, message) = match load {
        LoadState::Missing { file } => (
            ReducedReason::Missing,
            format!("model file {file} is missing"),
        ),
        LoadState::Checksum { file } => (
            ReducedReason::Checksum,
            format!("model file {file} failed its checksum"),
        ),
        LoadState::Unknown => (
            ReducedReason::Unknown,
            format!("this version of Hush doesn't know model {model}"),
        ),
        LoadState::Failed { message } => (
            ReducedReason::Failed,
            format!("the model failed to load: {message}"),
        ),
        _ if ready.is_some() => return None,
        LoadState::NotLoaded | LoadState::Loading => {
            (ReducedReason::Loading, "loading the model".to_string())
        }
        LoadState::Ready => (
            ReducedReason::FirstIndex,
            "the first index isn't finished".to_string(),
        ),
    };
    Some(Reduced { reason, message })
}

/// Keeps [`ModelStatus`] up to date from the job's reports.
#[derive(Default)]
pub struct ModelTracker {
    inner: Mutex<Tracked>,
}

#[derive(Default)]
struct Tracked {
    model: Option<String>,
    ready: Option<String>,
    loads: HashMap<String, LoadState>,
}

impl ModelTracker {
    pub fn status(&self) -> ModelStatus {
        let t = self.lock();
        let load = t
            .model
            .as_ref()
            .and_then(|m| t.loads.get(m))
            .cloned()
            .unwrap_or(LoadState::NotLoaded);
        ModelStatus {
            reduced: reduced(t.model.as_deref(), t.ready.as_deref(), &load),
            model: t.model.clone(),
            ready_model: t.ready.clone(),
            load,
        }
    }

    /// The vault's settings are unknown again (another unlock). Returns whether the
    /// status changed.
    pub fn forget_vault(&self) -> bool {
        self.change(|t| {
            t.model = None;
            t.ready = None;
        })
    }

    /// The job read the vault's model settings. Returns whether the status changed.
    pub fn vault_models(&self, model: &str, ready: Option<&str>) -> bool {
        self.change(|t| {
            t.model = Some(model.to_string());
            t.ready = ready.map(str::to_string);
        })
    }

    /// Loading `model` started, worked or failed. Returns whether the status changed.
    pub fn load(&self, model: &str, state: LoadState) -> bool {
        self.change(|t| {
            t.loads.insert(model.to_string(), state);
        })
    }

    /// Forgets failed loads, so the next attempt shows as loading. Returns whether the
    /// status changed.
    pub fn forget_failures(&self) -> bool {
        self.change(|t| {
            t.loads
                .retain(|_, s| matches!(s, LoadState::Ready | LoadState::Loading));
        })
    }

    fn change(&self, f: impl FnOnce(&mut Tracked)) -> bool {
        let before = self.status();
        f(&mut self.lock());
        self.status() != before
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Tracked> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::{ChunkPooling, ModelFile, TokenPooling};

    fn descriptor(model_sha: &'static str) -> ModelDescriptor {
        ModelDescriptor {
            id: "stub",
            name: "Stub",
            licence: "MIT",
            licence_url: "",
            bundled: false,
            model: ModelFile {
                name: "stub.onnx",
                url: "",
                sha256: model_sha,
                size: 5,
            },
            // sha256("{}")
            tokenizer: ModelFile {
                name: "stub.tokenizer.json",
                url: "",
                sha256: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                size: 2,
            },
            dims: 4,
            max_tokens: 8,
            token_pooling: TokenPooling::Mean,
            chunk_pooling: ChunkPooling::Mean,
            prefix: "",
            query_prefix: "",
            suggestion_threshold: 0.5,
            gpu: false,
            batch_across_notes: false,
        }
    }

    // sha256("hello")
    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn availability_checks_every_file_and_notices_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = descriptor(HELLO);
        let checks = FileChecks::default();
        assert_eq!(
            checks.availability(dir.path(), &d),
            Availability::Missing {
                file: "stub.tokenizer.json".into()
            }
        );
        std::fs::write(dir.path().join("stub.tokenizer.json"), b"{}").unwrap();
        assert_eq!(
            checks.availability(dir.path(), &d),
            Availability::Missing {
                file: "stub.onnx".into()
            }
        );
        std::fs::write(dir.path().join("stub.onnx"), b"hello").unwrap();
        assert_eq!(
            checks.availability(dir.path(), &d),
            Availability::Available {
                bytes: 7,
                downloaded: false
            }
        );
        let dirs = ModelDirs {
            bundled: dir.path().join("none"),
            user: dir.path().to_path_buf(),
        };
        assert_eq!(
            checks.locate(&dirs, &d),
            (
                Availability::Available {
                    bytes: 7,
                    downloaded: true
                },
                Some(dirs.user.clone())
            )
        );
        // A different size can't hit the cached result.
        std::fs::write(dir.path().join("stub.onnx"), b"hello, world").unwrap();
        assert_eq!(
            checks.availability(dir.path(), &d),
            Availability::Checksum {
                file: "stub.onnx".into()
            }
        );
        assert!(
            checks
                .availability(dir.path(), &d)
                .problem()
                .unwrap()
                .contains("checksum")
        );
    }

    #[test]
    fn load_errors_name_the_file_only() {
        let missing = EmbedError::Missing(PathBuf::from("/some/where/m.onnx"));
        assert_eq!(
            LoadState::from_error(&missing),
            LoadState::Missing {
                file: "m.onnx".into()
            }
        );
        let checksum = EmbedError::Checksum(PathBuf::from("/x/t.json"));
        assert_eq!(
            LoadState::from_error(&checksum),
            LoadState::Checksum {
                file: "t.json".into()
            }
        );
        assert_eq!(
            LoadState::from_error(&EmbedError::UnknownModel("z".into())),
            LoadState::Unknown
        );
    }

    #[test]
    fn features_are_reduced_until_a_model_is_ready_and_loadable() {
        let r = |model, ready, load: LoadState| reduced(model, ready, &load).map(|r| r.reason);
        assert_eq!(
            r(None, None, LoadState::NotLoaded),
            None,
            "locked: nothing to say"
        );
        assert_eq!(
            r(Some("a"), None, LoadState::Loading),
            Some(ReducedReason::Loading)
        );
        assert_eq!(
            r(Some("a"), None, LoadState::Ready),
            Some(ReducedReason::FirstIndex)
        );
        assert_eq!(r(Some("a"), Some("a"), LoadState::Ready), None);
        // Mid-switch the old model's vectors keep serving.
        assert_eq!(r(Some("b"), Some("a"), LoadState::Loading), None);
        assert_eq!(r(Some("b"), Some("a"), LoadState::Ready), None);
        // A model that can't load reduces features even when old vectors exist: new
        // and changed notes get none.
        assert_eq!(
            r(
                Some("a"),
                Some("a"),
                LoadState::Missing { file: "m".into() }
            ),
            Some(ReducedReason::Missing)
        );
        assert_eq!(
            r(Some("a"), None, LoadState::Checksum { file: "m".into() }),
            Some(ReducedReason::Checksum)
        );
        assert_eq!(
            r(Some("a"), None, LoadState::Unknown),
            Some(ReducedReason::Unknown)
        );
    }

    #[test]
    fn the_tracker_reports_changes_only() {
        let t = ModelTracker::default();
        assert!(!t.forget_vault());
        assert!(t.vault_models("a", None));
        assert!(!t.vault_models("a", None));
        assert_eq!(t.status().reduced.unwrap().reason, ReducedReason::Loading);
        assert!(t.load(
            "a",
            LoadState::Missing {
                file: "a.onnx".into()
            }
        ));
        assert_eq!(
            t.status().reduced.unwrap().message,
            "model file a.onnx is missing"
        );
        // Another model's load doesn't show.
        assert!(!t.load("b", LoadState::Ready));
        assert!(t.forget_failures());
        assert_eq!(t.status().load, LoadState::NotLoaded);
        assert!(t.load("a", LoadState::Ready));
        assert!(t.vault_models("a", Some("a")));
        assert_eq!(t.status().reduced, None);
        assert!(t.forget_vault());
        assert_eq!(t.status().model, None);
    }
}
