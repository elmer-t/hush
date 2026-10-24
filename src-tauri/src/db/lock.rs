//! Keeps a vault open in one process only. Two processes on the same vault would
//! silently lose edits (autosave is last-write-wins), and locking one would leave the
//! vault decrypted in the other.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use super::{Error, Result};

/// An exclusive OS lock on a file next to the vault (`vault.hush-lock`, beside SQLite's
/// own `-wal` and `-shm` files). Not on the vault file itself: on Windows a lock there
/// would block SQLite's own reads and writes. The file is empty and stays on disk.
#[derive(Debug)]
pub(super) struct VaultLock {
    // Never read: holding it open holds the lock.
    _file: File,
}

impl VaultLock {
    pub(super) fn acquire(vault: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(vault))
            .map_err(Error::Lock)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::VaultInUse),
            Err(TryLockError::Error(e)) => Err(Error::Lock(e)),
        }
    }
}

/// The lock file of the vault at `vault`.
pub fn lock_path(vault: &Path) -> PathBuf {
    let mut name = vault.as_os_str().to_owned();
    name.push("-lock");
    PathBuf::from(name)
}
