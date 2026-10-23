//! Tauri commands for the vault lifecycle: recent vaults, create/open, lock, auto-lock and
//! passphrase changes.

use std::path::PathBuf;

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroizing;

use crate::config::Config;
use crate::db::{self, Vault};
use crate::session::{self, Session, SessionError};

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

/// Reaches the frontend as `{ kind, message }`.
impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CommandError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

type Result<T> = std::result::Result<T, CommandError>;

#[derive(Serialize)]
pub struct VaultInfo {
    path: PathBuf,
    name: String,
}

impl VaultInfo {
    fn new(path: PathBuf) -> Self {
        let name = session::vault_name(&path);
        Self { path, name }
    }
}

#[derive(Serialize)]
pub struct RecentVault {
    path: PathBuf,
    name: String,
    exists: bool,
}

#[tauri::command]
pub fn recent_vaults(config: State<'_, Config>) -> Vec<RecentVault> {
    config
        .get()
        .recent_vaults
        .iter()
        .map(|p| RecentVault { path: p.clone(), name: session::vault_name(p), exists: p.is_file() })
        .collect()
}

#[tauri::command]
pub fn forget_recent_vault(config: State<'_, Config>, path: PathBuf) -> Result<()> {
    Ok(config.update(|c| c.forget(&path))?)
}

// Opening derives the key with PBKDF2, so these run off the main thread (async).

#[tauri::command]
pub async fn create_vault(
    session: State<'_, Session>,
    config: State<'_, Config>,
    path: PathBuf,
    passphrase: String,
) -> Result<VaultInfo> {
    let passphrase = Zeroizing::new(passphrase);
    let vault = Vault::create(&path, &passphrase)?;
    unlock(&session, &config, vault, path)
}

#[tauri::command]
pub async fn open_vault(
    session: State<'_, Session>,
    config: State<'_, Config>,
    path: PathBuf,
    passphrase: String,
) -> Result<VaultInfo> {
    let passphrase = Zeroizing::new(passphrase);
    let vault = Vault::open(&path, &passphrase)?;
    unlock(&session, &config, vault, path)
}

fn unlock(session: &Session, config: &Config, vault: Vault, path: PathBuf) -> Result<VaultInfo> {
    session.unlock(vault, path.clone())?;
    // A config write failure must not keep the user out of an unlocked vault.
    let _ = config.update(|c| c.remember(&path));
    Ok(VaultInfo::new(path))
}

#[tauri::command]
pub fn lock_vault(app: AppHandle, session: State<'_, Session>) {
    session.lock();
    let _ = app.emit(LOCKED_EVENT, "manual");
}

#[tauri::command]
pub fn vault_status(session: State<'_, Session>) -> Option<VaultInfo> {
    session.path().map(VaultInfo::new)
}

/// Reports user activity, postponing auto-lock.
#[tauri::command]
pub fn touch(session: State<'_, Session>) {
    session.touch();
}

#[tauri::command]
pub fn auto_lock_minutes(session: State<'_, Session>) -> Result<u32> {
    Ok(session.auto_lock_minutes()?)
}

#[tauri::command]
pub fn set_auto_lock_minutes(session: State<'_, Session>, minutes: u32) -> Result<()> {
    Ok(session.set_auto_lock_minutes(minutes)?)
}

#[tauri::command]
pub async fn change_passphrase(session: State<'_, Session>, current: String, new: String) -> Result<()> {
    let current = Zeroizing::new(current);
    let new = Zeroizing::new(new);
    Ok(session.with_vault(|v| {
        v.verify_passphrase(&current)?;
        v.change_passphrase(&new)
    })?)
}
