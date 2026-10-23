//! Tauri commands. Every error reaches the frontend as `{ kind, message }`.

pub mod notes;
pub mod search;
pub mod vault;

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
            Self::Session(SessionError::Db(e)) => match e {
                D::WrongPassphrase => "wrongPassphrase",
                D::EmptyPassphrase => "emptyPassphrase",
                D::VaultNotFound => "vaultNotFound",
                D::VaultExists => "vaultExists",
                D::UnsupportedVersion => "unsupportedVersion",
                D::InvalidTitle(_) => "invalidTitle",
                D::TitleTaken => "titleTaken",
                D::NoteNotFound => "noteNotFound",
                D::Sql(_) => "database",
            },
            Self::Config(_) => "config",
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
