//! Vault lifecycle: recent vaults, create/open, lock, auto-lock and passphrase changes.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroizing;

use std::time::Instant;

use super::{LOCKED_EVENT, Result, activity, embeddings, training};
use crate::activity::{Job, count};
use crate::config::Config;
use crate::db::{self, Compacted, Vault, VaultSpace};
use crate::session::{self, Session};

/// Emitted with a [`crate::db::Upgrade`] while opening a vault runs a migration that
/// rebuilds derived data, so the unlock screen can show progress.
pub const VAULT_UPGRADE_EVENT: &str = "vault-upgrade";

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
    app: AppHandle,
    session: State<'_, Session>,
    config: State<'_, Config>,
    path: PathBuf,
    passphrase: String,
) -> Result<VaultInfo> {
    let passphrase = Zeroizing::new(passphrase);
    let mut vault = Vault::create(&path, &passphrase)?;
    // Only here, not in `Vault::create`: a vault made by the app starts with the README
    // (#177), one made by a test or a benchmark starts empty.
    vault.add_readme()?;
    unlock(&app, &session, &config, vault, path)
}

#[tauri::command]
pub async fn open_vault(
    app: AppHandle,
    session: State<'_, Session>,
    config: State<'_, Config>,
    path: PathBuf,
    passphrase: String,
) -> Result<VaultInfo> {
    let passphrase = Zeroizing::new(passphrase);
    let started = Instant::now();
    let mut upgraded = None;
    let vault = Vault::open_with_progress(&path, &passphrase, &mut |progress| {
        upgraded = Some(progress);
        let _ = app.emit(VAULT_UPGRADE_EVENT, progress);
    })?;
    let opened = started.elapsed();
    let info = unlock(&app, &session, &config, vault, path)?;
    // The migration ran before the workspace (and its Activity panel) existed: it shows
    // as the last run.
    if let Some(upgrade) = upgraded {
        session.activity().reporter(session.generation()).record(
            Job::Migration,
            opened,
            format!("upgraded the vault, rebuilding the search index for {} notes", count(upgrade.total as u64)),
        );
    }
    Ok(info)
}

fn unlock(app: &AppHandle, session: &Session, config: &Config, vault: Vault, path: PathBuf) -> Result<VaultInfo> {
    let generation = session.unlock(vault, path.clone())?;
    embeddings::start(app, generation);
    training::start(app, generation);
    training::start_retag(app, generation);
    activity::start(app, generation);
    // A config write failure must not keep the user out of an unlocked vault.
    let _ = config.update(|c| c.remember(&path));
    Ok(VaultInfo::new(path))
}

#[tauri::command(async)]
pub fn lock_vault(app: AppHandle, session: State<'_, Session>) {
    session.lock();
    let _ = app.emit(LOCKED_EVENT, "manual");
}

#[tauri::command(async)]
pub fn vault_status(session: State<'_, Session>) -> Option<VaultInfo> {
    session.path().map(VaultInfo::new)
}

/// Reports user activity, postponing auto-lock.
#[tauri::command(async)]
pub fn touch(session: State<'_, Session>) {
    session.touch();
}

#[tauri::command(async)]
pub fn auto_lock_minutes(session: State<'_, Session>) -> Result<u32> {
    Ok(session.auto_lock_minutes()?)
}

#[tauri::command(async)]
pub fn set_auto_lock_minutes(session: State<'_, Session>, minutes: u32) -> Result<()> {
    Ok(session.set_auto_lock_minutes(minutes)?)
}

/// The memory-wiping choice for this machine: what is saved, and what this process
/// started with. They differ until Hush is restarted.
#[derive(Serialize)]
pub struct MemorySecurity {
    on: bool,
    active: bool,
}

#[tauri::command]
pub fn memory_security(config: State<'_, Config>) -> MemorySecurity {
    MemorySecurity { on: config.get().memory_security, active: db::memory_security() }
}

/// Saves the choice; it takes effect on the next start (see [`db::set_memory_security`]).
#[tauri::command]
pub fn set_memory_security(config: State<'_, Config>, on: bool) -> Result<MemorySecurity> {
    config.update(|c| c.memory_security = on)?;
    Ok(MemorySecurity { on, active: db::memory_security() })
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

/// The vault file's size and how much compacting would give back, for Settings.
#[tauri::command(async)]
pub fn vault_space(session: State<'_, Session>) -> Result<VaultSpace> {
    Ok(session.with_vault(|v| v.space())?)
}

/// Compacts the vault (see [`crate::db::Vault::compact`] and [`Session::compact`]). Runs
/// on a blocking thread: it holds the vault for minutes on a large vault.
#[tauri::command]
pub async fn compact_vault(app: AppHandle) -> Result<Compacted> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(app.state::<Session>().compact(|v| v.compact(&db::available_space))?)
    })
    .await
    .expect("compaction panicked")
}

/// Exits after the frontend has saved pending edits; the vault is locked on exit.
#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}
