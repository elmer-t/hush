//! Vault lifecycle: recent vaults, create/open, lock, auto-lock and passphrase changes.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroizing;

use super::{LOCKED_EVENT, Result};
use crate::config::Config;
use crate::db::Vault;
use crate::session::{self, Session};

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

/// Exits after the frontend has saved pending edits; the vault is locked on exit.
#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}
