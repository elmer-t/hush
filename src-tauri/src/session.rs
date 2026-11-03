//! The unlocked vault, if any, and when it was last used. Held in Tauri state; every
//! vault access goes through [`Session::with_vault`], which also counts as activity,
//! except background work, which uses [`Session::with_vault_quiet`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant, SystemTime};

use crate::db::{self, Vault};
use crate::embed::related::RelatedIndex;

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
    /// Counts unlocks, so a background job started for one unlock never touches the
    /// vault of a later one.
    unlocks: AtomicU64,
    /// The generation of the open vault, 0 while locked. Readable without the mutex.
    current: AtomicU64,
    /// The related-notes index of the open vault, kept apart from the vault so a lookup
    /// never waits for it. Dropped (and zeroed) on lock.
    related: RwLock<Option<RelatedIndex>>,
    /// Numbers searches, so a newer one can cancel an older one (see [`Session::search`]).
    searches: AtomicU64,
    /// Interrupts the open vault's connection; `Some` only while a search runs on it,
    /// so an interrupt can never hit a save or the background jobs.
    running_search: Mutex<Option<rusqlite::InterruptHandle>>,
    /// Imports writing to the vault right now (see [`Session::begin_import`]).
    imports: AtomicUsize,
    /// What the background jobs are doing, for the Activity panel. Cleared on lock.
    activity: crate::activity::Activity,
}

/// Held while an import writes notes; see [`Session::begin_import`].
pub struct ImportRunning<'a>(&'a Session);

impl Drop for ImportRunning<'_> {
    fn drop(&mut self) {
        self.0.imports.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Unlocked {
    vault: Vault,
    path: PathBuf,
    generation: u64,
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
    /// Makes `vault` the open vault, locking any vault that was open before. Notes past
    /// the trash retention period are deleted first. Returns the unlock's generation,
    /// for [`Session::with_vault_quiet`].
    pub fn unlock(&self, mut vault: Vault, path: PathBuf) -> Result<u64, SessionError> {
        vault.purge_trash()?;
        let minutes = match vault.setting(AUTO_LOCK_SETTING)? {
            Some(v) => v.parse().ok().filter(|m| valid_minutes(*m)).unwrap_or(DEFAULT_AUTO_LOCK_MINUTES),
            None => DEFAULT_AUTO_LOCK_MINUTES,
        };
        let mut guard = self.guard();
        let generation = self.unlocks.fetch_add(1, Ordering::SeqCst) + 1;
        let previous = guard.replace(Unlocked {
            vault,
            path,
            generation,
            auto_lock: minutes_to_duration(minutes),
            activity: Activity::now(),
        });
        self.current.store(generation, Ordering::SeqCst);
        drop(guard);
        self.drop_related_index();
        self.activity.clear(generation);
        if let Some(previous) = previous {
            let _ = previous.vault.close();
        }
        Ok(generation)
    }

    /// Closes the open vault. Returns whether one was open.
    pub fn lock(&self) -> bool {
        let Some(unlocked) = self.take() else { return false };
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
        self.current.store(0, Ordering::SeqCst);
        if let Some(unlocked) = guard.take() {
            let _ = unlocked.vault.close();
        }
        drop(guard);
        self.drop_related_index();
        self.activity.clear(0);
        true
    }

    fn take(&self) -> Option<Unlocked> {
        let mut guard = self.guard();
        self.current.store(0, Ordering::SeqCst);
        let unlocked = guard.take();
        drop(guard);
        self.drop_related_index();
        self.activity.clear(0);
        unlocked
    }

    /// Called after `current` changed, so a job of the old unlock can't put its index
    /// back (see [`Session::update_related_index`]). The buffers zero themselves on drop.
    fn drop_related_index(&self) {
        *self.related.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The related-notes index of the open vault; `None` while locked or until the
    /// embedding job first builds it.
    pub fn related_index(&self) -> RwLockReadGuard<'_, Option<RelatedIndex>> {
        self.related.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the related-notes index, unless the vault of unlock `generation` is no
    /// longer open (then returns `None`). Checked under the index lock, which lock and
    /// unlock take after changing the generation, so a stale job can't write.
    pub fn update_related_index<T>(
        &self,
        generation: u64,
        f: impl FnOnce(&mut Option<RelatedIndex>) -> T,
    ) -> Option<T> {
        let mut index = self.related.write().unwrap_or_else(|e| e.into_inner());
        if generation == 0 || !self.is_open(generation) {
            return None;
        }
        Some(f(&mut index))
    }

    /// What the background jobs are doing; jobs report through
    /// [`crate::activity::Activity::reporter`] with their unlock's generation.
    pub fn activity(&self) -> &crate::activity::Activity {
        &self.activity
    }

    /// The generation of the open vault, 0 while locked.
    pub fn generation(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
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

    /// Runs search `f` on the vault, cancelling any older search still running or
    /// waiting for the vault: a search is only worth finishing while it is the newest.
    /// A running one is interrupted mid-statement, and `f` gets a `stop` check to ask
    /// between statements. Returns `None` when a newer search cancelled this one (or
    /// `f` returned `None` because `stop` said so); that is not an error.
    pub fn search<T>(
        &self,
        f: impl FnOnce(&mut Vault, &dyn Fn() -> bool) -> db::Result<Option<T>>,
    ) -> Result<Option<T>, SessionError> {
        let ticket = self.searches.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(handle) = self.running_search.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            handle.interrupt();
        }
        let result = self.with_vault(|v| {
            if self.searches.load(Ordering::SeqCst) != ticket {
                return Ok(None);
            }
            *self.running_search.lock().unwrap_or_else(|e| e.into_inner()) = Some(v.interrupt_handle());
            let stop = || self.searches.load(Ordering::SeqCst) != ticket;
            let result = f(v, &stop);
            // Cleared before the vault is released, so the next user of the connection
            // can't be interrupted.
            *self.running_search.lock().unwrap_or_else(|e| e.into_inner()) = None;
            result
        });
        match result {
            Err(SessionError::Db(db::Error::Sql(e))) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) => {
                Ok(None)
            }
            other => other,
        }
    }

    /// Marks an import as running until the returned guard drops, however the import
    /// ends. The background jobs (embedding, tag training) don't start work meanwhile
    /// and the embedding job stops between batches: the notes they would work on are
    /// still arriving, and they would compete with the import for the cores and the
    /// vault. The embedding queue keeps filling, so nothing is missed.
    pub fn begin_import(&self) -> ImportRunning<'_> {
        self.imports.fetch_add(1, Ordering::SeqCst);
        ImportRunning(self)
    }

    /// Whether an import is running (see [`Session::begin_import`]).
    pub fn import_running(&self) -> bool {
        self.imports.load(Ordering::SeqCst) > 0
    }

    /// Whether a search is running on the vault right now.
    pub fn search_running(&self) -> bool {
        self.running_search.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Like [`Session::with_vault`], but for background work: it does not count as
    /// activity, so it never postpones auto-lock, and it fails with
    /// [`SessionError::Locked`] unless the vault of unlock `generation` is still open.
    pub fn with_vault_quiet<T>(
        &self,
        generation: u64,
        f: impl FnOnce(&mut Vault) -> db::Result<T>,
    ) -> Result<T, SessionError> {
        let mut guard = self.guard();
        match guard.as_mut() {
            Some(unlocked) if unlocked.generation == generation => Ok(f(&mut unlocked.vault)?),
            _ => Err(SessionError::Locked),
        }
    }

    /// Whether the vault of unlock `generation` is still open. Doesn't wait for the vault.
    pub fn is_open(&self, generation: u64) -> bool {
        self.current.load(Ordering::SeqCst) == generation
    }

    /// How long the vault has been idle; `None` while locked.
    pub fn idle_for(&self) -> Option<Duration> {
        self.guard().as_ref().map(|u| u.activity.idle_at(Activity::now()))
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

    #[test]
    fn unlock_purges_expired_trash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        let mut vault = Vault::create(&path, PASS).unwrap();
        let old = vault.create_note("", "Old", "").unwrap();
        let recent = vault.create_note("", "Recent", "").unwrap();
        vault.trash_note(old).unwrap();
        vault.trash_note(recent).unwrap();
        vault.backdate_trash(old, i64::from(db::DEFAULT_TRASH_DAYS)).unwrap();
        vault.backdate_trash(recent, i64::from(db::DEFAULT_TRASH_DAYS) - 1).unwrap();

        let session = Session::default();
        session.unlock(vault, path.clone()).unwrap();
        let left: Vec<i64> = session.with_vault(|v| v.trashed_notes()).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(left, [recent]);

        // 0 = never: nothing is purged however old.
        session.with_vault(|v| v.set_trash_days(0)).unwrap();
        session.lock();
        let vault = Vault::open(&path, PASS).unwrap();
        vault.backdate_trash(recent, 10_000).unwrap();
        session.unlock(vault, path).unwrap();
        assert_eq!(session.with_vault(|v| v.trashed_notes()).unwrap().len(), 1);
    }
}
