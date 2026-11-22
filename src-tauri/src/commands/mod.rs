//! Tauri commands. Every error reaches the frontend as `{ kind, message }`.
//!
//! Every command that uses the vault is `#[tauri::command(async)]` (or an `async fn`):
//! a plain `#[tauri::command]` runs on the main thread, which also runs the window, so
//! a command waiting for the vault while a background job holds it froze the whole
//! window ("Not Responding"). Off the main thread only the call waits. New commands
//! that take `State<Session>` must do the same.

pub mod about;
pub mod activity;
pub mod discover;
pub mod embeddings;
pub mod import;
pub mod layout;
pub mod links;
pub mod locale;
pub mod models;
pub mod notes;
pub mod review;
pub mod search;
pub mod tags;
pub mod training;
pub mod vault;
pub mod window;

use serde::{Serialize, Serializer};

use crate::db;
use crate::session::SessionError;

/// Emitted when the window is closed while a vault is open; the frontend saves pending
/// edits and calls `exit_app`.
pub const CLOSE_EVENT: &str = "close-requested";

/// Emitted with the reason (`"manual"` or `"idle"`) whenever the vault locks.
pub const LOCKED_EVENT: &str = "vault-locked";

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("could not save app config: {0}")]
    Config(#[from] std::io::Error),
    #[error("could not read the folder to import: {0}")]
    ImportSource(std::io::Error),
    #[error("an import is already running")]
    ImportRunning,
    #[error("the groups are out of date; group the notes again")]
    StaleGroups,
    #[error("this version of Hush doesn't know model “{0}”")]
    UnknownModel(String),
    #[error("{0}")]
    ModelUnavailable(String),
    /// Downloading, importing or removing a model's files.
    #[error("{0}")]
    ModelFiles(String),
    /// Reading the bundled third-party notices.
    #[error("could not read the third-party notices: {0}")]
    Notices(String),
}

impl From<crate::import::ImportError> for CommandError {
    fn from(e: crate::import::ImportError) -> Self {
        use crate::import::ImportError as I;
        match e {
            I::Session(e) => Self::Session(e),
            I::Source(e) => Self::ImportSource(e),
        }
    }
}

impl From<db::Error> for CommandError {
    fn from(e: db::Error) -> Self {
        Self::Session(e.into())
    }
}

impl CommandError {
    fn kind(&self) -> &'static str {
        use db::Error as D;
        match self {
            Self::Session(SessionError::Locked) => "locked",
            Self::Session(SessionError::InvalidAutoLock) => "invalidAutoLock",
            Self::Session(SessionError::Compacting) => "compacting",
            Self::Session(SessionError::Db(e)) => match e {
                D::WrongPassphrase => "wrongPassphrase",
                D::EmptyPassphrase => "emptyPassphrase",
                D::PassphraseTooShort => "passphraseTooShort",
                D::VaultNotFound => "vaultNotFound",
                D::VaultExists => "vaultExists",
                D::VaultInUse => "vaultInUse",
                D::Lock(_) => "vaultLock",
                D::UnsupportedVersion => "unsupportedVersion",
                D::InvalidTitle(_) => "invalidTitle",
                D::TitleTaken => "titleTaken",
                D::NoteNotFound => "noteNotFound",
                D::MoveClash(_) => "moveClash",
                D::InvalidFolderMove(_) => "invalidFolderMove",
                D::InvalidFolder(_) => "invalidFolder",
                D::InvalidTag(_) => "invalidTag",
                D::InvalidRule(_) => "invalidRule",
                D::InvalidTrashDays => "invalidTrashDays",
                D::InvalidThreshold => "invalidThreshold",
                D::InvalidRecentOpenedCount => "invalidRecentOpenedCount",
                D::InvalidRating => "invalidRating",
                D::InvalidVaultIcon => "invalidVaultIcon",
                D::NotEnoughDiskSpace { .. } => "notEnoughDiskSpace",
                D::DiskSpace(_) => "diskSpace",
                D::UpgradeDiskFull => "upgradeDiskFull",
                D::Sql(_) => "database",
            },
            Self::Config(_) => "config",
            Self::ImportSource(_) => "importSource",
            Self::ImportRunning => "importRunning",
            Self::StaleGroups => "staleGroups",
            Self::UnknownModel(_) => "unknownModel",
            Self::ModelUnavailable(_) => "modelUnavailable",
            Self::ModelFiles(_) => "modelFiles",
            Self::Notices(_) => "notices",
        }
    }
}

/// Serialized as `{ kind, message }`.
impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CommandError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

pub type Result<T> = std::result::Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    /// The position of each `CommandError` variant in `command_errors`. No wildcard arm:
    /// adding a variant stops compiling here, so its `kind` gets a row too.
    fn command_slot(e: &CommandError) -> usize {
        match e {
            CommandError::Session(_) => 0,
            CommandError::Config(_) => 1,
            CommandError::ImportSource(_) => 2,
            CommandError::ImportRunning => 3,
            CommandError::StaleGroups => 4,
            CommandError::UnknownModel(_) => 5,
            CommandError::ModelUnavailable(_) => 6,
            CommandError::ModelFiles(_) => 7,
            CommandError::Notices(_) => 8,
        }
    }

    fn command_errors() -> Vec<(CommandError, &'static str)> {
        let io = || io::Error::other("x");
        vec![
            (SessionError::Locked.into(), "locked"),
            (CommandError::Config(io()), "config"),
            (CommandError::ImportSource(io()), "importSource"),
            (CommandError::ImportRunning, "importRunning"),
            (CommandError::StaleGroups, "staleGroups"),
            (CommandError::UnknownModel("m".into()), "unknownModel"),
            (CommandError::ModelUnavailable("m".into()), "modelUnavailable"),
            (CommandError::ModelFiles("m".into()), "modelFiles"),
            (CommandError::Notices("m".into()), "notices"),
        ]
    }

    /// Same for `SessionError`; the `Db` variant is covered by `db_slot`.
    fn session_slot(e: &SessionError) -> usize {
        match e {
            SessionError::Locked => 0,
            SessionError::InvalidAutoLock => 1,
            SessionError::Compacting => 2,
            SessionError::Db(_) => 3,
        }
    }

    fn session_errors() -> Vec<(SessionError, &'static str)> {
        vec![
            (SessionError::Locked, "locked"),
            (SessionError::InvalidAutoLock, "invalidAutoLock"),
            (SessionError::Compacting, "compacting"),
            (SessionError::Db(db::Error::VaultInUse), "vaultInUse"),
        ]
    }

    /// Same for `db::Error`.
    fn db_slot(e: &db::Error) -> usize {
        use db::Error as D;
        match e {
            D::WrongPassphrase => 0,
            D::EmptyPassphrase => 1,
            D::PassphraseTooShort => 2,
            D::VaultNotFound => 3,
            D::VaultExists => 4,
            D::VaultInUse => 5,
            D::Lock(_) => 6,
            D::UnsupportedVersion => 7,
            D::InvalidTitle(_) => 8,
            D::TitleTaken => 9,
            D::NoteNotFound => 10,
            D::MoveClash(_) => 11,
            D::InvalidFolderMove(_) => 12,
            D::InvalidFolder(_) => 13,
            D::InvalidTag(_) => 14,
            D::InvalidRule(_) => 15,
            D::InvalidTrashDays => 16,
            D::InvalidThreshold => 17,
            D::InvalidRecentOpenedCount => 18,
            D::InvalidRating => 19,
            D::InvalidVaultIcon => 20,
            D::NotEnoughDiskSpace { .. } => 21,
            D::DiskSpace(_) => 22,
            D::UpgradeDiskFull => 23,
            D::Sql(_) => 24,
        }
    }

    fn db_errors() -> Vec<(db::Error, &'static str)> {
        use db::Error as D;
        let io = || io::Error::other("x");
        vec![
            (D::WrongPassphrase, "wrongPassphrase"),
            (D::EmptyPassphrase, "emptyPassphrase"),
            (D::PassphraseTooShort, "passphraseTooShort"),
            (D::VaultNotFound, "vaultNotFound"),
            (D::VaultExists, "vaultExists"),
            (D::VaultInUse, "vaultInUse"),
            (D::Lock(io()), "vaultLock"),
            (D::UnsupportedVersion, "unsupportedVersion"),
            (D::InvalidTitle("t".into()), "invalidTitle"),
            (D::TitleTaken, "titleTaken"),
            (D::NoteNotFound, "noteNotFound"),
            (D::MoveClash(vec!["a".into()]), "moveClash"),
            (D::InvalidFolderMove("m".into()), "invalidFolderMove"),
            (D::InvalidFolder("m".into()), "invalidFolder"),
            (D::InvalidTag("t".into()), "invalidTag"),
            (D::InvalidRule("r".into()), "invalidRule"),
            (D::InvalidTrashDays, "invalidTrashDays"),
            (D::InvalidThreshold, "invalidThreshold"),
            (D::InvalidRecentOpenedCount, "invalidRecentOpenedCount"),
            (D::InvalidRating, "invalidRating"),
            (D::InvalidVaultIcon, "invalidVaultIcon"),
            (D::NotEnoughDiskSpace { needed: 2, available: 1 }, "notEnoughDiskSpace"),
            (D::DiskSpace(io()), "diskSpace"),
            (D::UpgradeDiskFull, "upgradeDiskFull"),
            (D::Sql(rusqlite::Error::InvalidQuery), "database"),
        ]
    }

    /// Row `i` of the list is the variant `slot` maps to `i`: nothing missing, nothing twice.
    fn assert_every_slot<T>(rows: &[(T, &str)], slot: impl Fn(&T) -> usize) {
        let slots: Vec<usize> = rows.iter().map(|(e, _)| slot(e)).collect();
        assert_eq!(slots, (0..rows.len()).collect::<Vec<_>>());
    }

    fn kind_of(e: CommandError) -> String {
        serde_json::to_value(&e).unwrap()["kind"].as_str().unwrap().to_owned()
    }

    #[test]
    fn every_command_error_serializes_its_kind() {
        let rows = command_errors();
        assert_every_slot(&rows, command_slot);
        for (e, kind) in rows {
            assert_eq!(kind_of(e), kind);
        }
    }

    #[test]
    fn every_session_error_serializes_its_kind() {
        let rows = session_errors();
        assert_every_slot(&rows, session_slot);
        for (e, kind) in rows {
            assert_eq!(kind_of(e.into()), kind);
        }
    }

    #[test]
    fn every_db_error_serializes_its_kind() {
        let rows = db_errors();
        assert_every_slot(&rows, db_slot);
        for (e, kind) in rows {
            assert_eq!(kind_of(e.into()), kind);
        }
    }

    #[test]
    fn the_serialized_error_carries_the_message() {
        let v = serde_json::to_value(CommandError::ImportRunning).unwrap();
        assert_eq!(v["message"], "an import is already running");
    }

    /// Every command module, as compiled.
    const SOURCES: &[(&str, &str)] = &[
        ("about", include_str!("about.rs")),
        ("activity", include_str!("activity.rs")),
        ("discover", include_str!("discover.rs")),
        ("embeddings", include_str!("embeddings.rs")),
        ("import", include_str!("import.rs")),
        ("layout", include_str!("layout.rs")),
        ("links", include_str!("links.rs")),
        ("models", include_str!("models.rs")),
        ("notes", include_str!("notes.rs")),
        ("review", include_str!("review.rs")),
        ("search", include_str!("search.rs")),
        ("tags", include_str!("tags.rs")),
        ("training", include_str!("training.rs")),
        ("vault", include_str!("vault.rs")),
        ("window", include_str!("window.rs")),
    ];

    /// A command on the main thread that waits for the vault freezes the window.
    #[test]
    fn commands_that_use_the_vault_stay_off_the_main_thread() {
        let mut main_thread = Vec::new();
        for (module, source) in SOURCES {
            let mut rest = *source;
            while let Some(at) = rest.find("#[tauri::command]") {
                rest = &rest[at + "#[tauri::command]".len()..];
                let signature = &rest[..rest.find('{').unwrap_or(rest.len())];
                if !signature.contains("async fn") && signature.contains("Session") {
                    let name = signature.split("fn ").nth(1).and_then(|s| s.split('(').next()).unwrap_or("?");
                    main_thread.push(format!("{module}::{name}"));
                }
            }
        }
        assert!(main_thread.is_empty(), "use #[tauri::command(async)] for: {}", main_thread.join(", "));
    }
}
