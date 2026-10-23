//! The unlocked vault, if any, and when it was last used. Held in Tauri state; every
//! vault access goes through [`Session::with_vault`], which also counts as activity.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use crate::db::{self, Vault};

pub const AUTO_LOCK_SETTING: &str = "auto_lock_minutes";
pub const DEFAULT_AUTO_LOCK_MINUTES: u32 = 10;
pub const MAX_AUTO_LOCK_MINUTES: u32 = 24 * 60;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("vault is locked")]
    Locked,
    #[error("auto-lock must be between 1 and {MAX_AUTO_LOCK_MINUTES} minutes")]
    InvalidAutoLock,
    #[error(transparent)]
    Db(#[from] db::Error),
}

#[derive(Default)]
pub struct Session {
    open: Mutex<Option<Unlocked>>,
}

struct Unlocked {
    vault: Vault,
    path: PathBuf,
    auto_lock: Duration,
    activity: Activity,
}

/// Last activity on both clocks: `Instant` may stop while the machine sleeps, and the
/// wall clock can be changed, so idle time is the larger of the two.
#[derive(Clone, Copy)]
struct Activity {
    instant: Instant,
    wall: SystemTime,
}

impl Activity {
    fn now() -> Self {
        Self { instant: Instant::now(), wall: SystemTime::now() }
    }

    fn idle_at(&self, now: Activity) -> Duration {
        let wall = now.wall.duration_since(self.wall).unwrap_or_default();
        now.instant.saturating_duration_since(self.instant).max(wall)
    }
}

impl Session {
    /// Makes `vault` the open vault, locking any vault that was open before.
    pub fn unlock(&self, vault: Vault, path: PathBuf) -> Result<(), SessionError> {
        let minutes = match vault.setting(AUTO_LOCK_SETTING)? {
            Some(v) => v.parse().ok().filter(|m| valid_minutes(*m)).unwrap_or(DEFAULT_AUTO_LOCK_MINUTES),
            None => DEFAULT_AUTO_LOCK_MINUTES,
        };
        let previous = self.guard().replace(Unlocked {
            vault,
            path,
            auto_lock: minutes_to_duration(minutes),
            activity: Activity::now(),
        });
        if let Some(previous) = previous {
            let _ = previous.vault.close();
        }
        Ok(())
    }

    /// Closes the open vault. Returns whether one was open.
    pub fn lock(&self) -> bool {
        let Some(unlocked) = self.guard().take() else { return false };
        // On failure the connection is dropped, which closes it all the same.
        let _ = unlocked.vault.close();
        true
    }

    /// Locks the vault if it has been idle for its auto-lock period. Returns whether it locked.
    pub fn lock_if_idle(&self) -> bool {
        self.lock_if_idle_at(Activity::now())
    }

    fn lock_if_idle_at(&self, now: Activity) -> bool {
        let mut guard = self.guard();
        match guard.as_ref() {
            Some(u) if u.activity.idle_at(now) >= u.auto_lock => {}
            _ => return false,
        }
        if let Some(unlocked) = guard.take() {
            let _ = unlocked.vault.close();
        }
        true
    }

    pub fn touch(&self) {
        if let Some(u) = self.guard().as_mut() {
            u.activity = Activity::now();
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.guard().as_ref().map(|u| u.path.clone())
    }

    pub fn with_vault<T>(&self, f: impl FnOnce(&mut Vault) -> db::Result<T>) -> Result<T, SessionError> {
        let mut guard = self.guard();
        let unlocked = guard.as_mut().ok_or(SessionError::Locked)?;
        unlocked.activity = Activity::now();
        Ok(f(&mut unlocked.vault)?)
    }

    pub fn auto_lock_minutes(&self) -> Result<u32, SessionError> {
        let guard = self.guard();
        let unlocked = guard.as_ref().ok_or(SessionError::Locked)?;
        Ok((unlocked.auto_lock.as_secs() / 60) as u32)
    }

    pub fn set_auto_lock_minutes(&self, minutes: u32) -> Result<(), SessionError> {
        if !valid_minutes(minutes) {
            return Err(SessionError::InvalidAutoLock);
        }
        let mut guard = self.guard();
        let unlocked = guard.as_mut().ok_or(SessionError::Locked)?;
        unlocked.vault.set_setting(AUTO_LOCK_SETTING, &minutes.to_string())?;
        unlocked.auto_lock = minutes_to_duration(minutes);
        unlocked.activity = Activity::now();
        Ok(())
    }

    fn guard(&self) -> MutexGuard<'_, Option<Unlocked>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The file name of a vault without its extension, for display.
pub fn vault_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn valid_minutes(minutes: u32) -> bool {
    (1..=MAX_AUTO_LOCK_MINUTES).contains(&minutes)
}

fn minutes_to_duration(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes) * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "session test passphrase";

    fn unlocked() -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        let session = Session::default();
        session.unlock(Vault::create(&path, PASS).unwrap(), path).unwrap();
        (dir, session)
    }

    fn later(minutes: u64) -> Activity {
        let d = Duration::from_secs(minutes * 60);
        let now = Activity::now();
        Activity { instant: now.instant + d, wall: now.wall + d }
    }

    #[test]
    fn defaults_to_ten_minutes_and_locks_when_idle() {
        let (_dir, session) = unlocked();
        assert_eq!(session.auto_lock_minutes().unwrap(), DEFAULT_AUTO_LOCK_MINUTES);
        assert!(!session.lock_if_idle_at(later(9)));
        assert!(session.path().is_some());
        assert!(session.lock_if_idle_at(later(10)));
        assert!(session.path().is_none());
        assert!(matches!(session.with_vault(|_| Ok(())), Err(SessionError::Locked)));
    }

    #[test]
    fn idle_counts_wall_clock_when_monotonic_clock_stalls() {
        let (_dir, session) = unlocked();
        let now = Activity::now();
        let asleep = Activity { instant: now.instant, wall: now.wall + Duration::from_secs(3600) };
        assert!(session.lock_if_idle_at(asleep));
    }

    #[test]
    fn auto_lock_setting_is_stored_in_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        let session = Session::default();
        session.unlock(Vault::create(&path, PASS).unwrap(), path.clone()).unwrap();
        assert!(matches!(session.set_auto_lock_minutes(0), Err(SessionError::InvalidAutoLock)));
        session.set_auto_lock_minutes(3).unwrap();
        assert!(session.lock());
        assert!(!session.lock());

        session.unlock(Vault::open(&path, PASS).unwrap(), path).unwrap();
        assert_eq!(session.auto_lock_minutes().unwrap(), 3);
        assert!(session.lock_if_idle_at(later(3)));
    }
}
