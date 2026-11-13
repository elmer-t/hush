//! The encrypted vault: one SQLCipher database file per vault. All database access goes
//! through [`Vault`].

mod compact;
mod discover;
mod embeddings;
mod graph;
mod heads;
mod internals;
mod links;
mod lock;
mod metadata;
mod notes;
mod readme;
mod reasons;
mod recent;
mod retag;
mod review;
mod rules;
mod schema;
mod search;
mod tags;
mod trash;

use std::cell::Cell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};

pub use compact::{COMPACT_MIN_BYTES, COMPACT_MIN_PERCENT, Compacted, VaultSpace, available_space, byte_text};
pub use discover::{DiscoverScope, ScopeCursor};
pub use embeddings::{
    EMBEDDING_MODEL_READY_SETTING, EMBEDDING_MODEL_SETTING, EmbeddingScan, NoteText, QueueCounts, QueueCursor, QueuePage, QueuePhase, QueuedText,
    QueuedVector, StoredVector,
};
pub use graph::{GraphScopeNotes, LinkTarget, Resolver};
pub use heads::{ScoreMethod, StoredHead, TagLabels, TagScorer, TagTraining, TrainingPage, TrainingStatus, new_stored_head};
pub use internals::{HeadTime, NoteInternals, NoteTag, NoteVector, TagSource};
pub use links::{Backlink, OutgoingLink, Resolution, SNIPPET_CHARS};
pub use lock::lock_path;
pub use metadata::{MetadataEdit, MetadataEditReport, TitledNote};
pub use notes::{FolderSize, NewNote, Note, NoteSummary, ReadStatus};
pub use reasons::{MAX_VIA, Reasons};
pub use recent::{
    DEFAULT_RECENT_OPENED_COUNT, LAST_OPENED_SETTING, MAX_RECENT_OPENED_COUNT, RECENT_OPENED_COUNT_SETTING,
    RECENT_OPENED_SETTING,
};
pub use retag::{RetagEnd, RetagPage, TAG_RULES_PROGRESS_SETTING, TAG_RULES_SETTING};
pub use review::{REVIEW_MIN_PROBABILITY, ReviewNote, ReviewOrder, SuggestionReview, review_min_score, sort_review};
pub use schema::{Upgrade, UpgradeStep};
pub use search::{DateField, Facets, HIGHLIGHT_END, HIGHLIGHT_START, NoteFilter, RANK_LIMIT, ReadFilter, SearchHit, SearchMode, SearchPage, SearchQuery, Why};
pub use tags::{SUGGEST_THRESHOLD_SETTING, SUGGESTIONS_PER_NOTE, TagFeedback, TagInfo, TagSuggestion};
pub use trash::{DEFAULT_TRASH_DAYS, FolderContents, MAX_TRASH_DAYS, TRASH_DAYS_SETTING, TrashEntry, TrashedNote};

pub type Result<T> = std::result::Result<T, Error>;

/// The shortest passphrase a new vault, or a passphrase change, accepts, counted in
/// characters. Opening never checks it, so vaults made before the minimum still open.
pub const MIN_PASSPHRASE_CHARS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wrong passphrase, or not a Hush vault")]
    WrongPassphrase,
    #[error("passphrase must not be empty")]
    EmptyPassphrase,
    #[error("passphrase must be at least {MIN_PASSPHRASE_CHARS} characters")]
    PassphraseTooShort,
    #[error("vault file not found")]
    VaultNotFound,
    #[error("a file already exists at that location")]
    VaultExists,
    #[error("this vault is already open in another Hush window")]
    VaultInUse,
    #[error("could not lock the vault file: {0}")]
    Lock(std::io::Error),
    #[error("vault was created by a newer version of Hush")]
    UnsupportedVersion,
    #[error("invalid title: {0}")]
    InvalidTitle(String),
    #[error("a note with this title already exists in this folder")]
    TitleTaken,
    #[error("note not found")]
    NoteNotFound,
    #[error("nothing was moved: the target folder would get more than one note named {}", quote_list(.0))]
    MoveClash(Vec<String>),
    #[error("{0}")]
    InvalidFolderMove(String),
    #[error("{0}")]
    InvalidFolder(String),
    #[error("invalid tag: {0}")]
    InvalidTag(String),
    #[error("{0}")]
    InvalidRule(String),
    #[error("trash retention must be between 0 and {} days", trash::MAX_TRASH_DAYS)]
    InvalidTrashDays,
    #[error("the number of recently opened notes must be between 0 and {}", recent::MAX_RECENT_OPENED_COUNT)]
    InvalidRecentOpenedCount,
    #[error("a rating must be a whole number from 1 to 5")]
    InvalidRating,
    #[error("the suggestion threshold must be a number between -1 and 1")]
    InvalidThreshold,
    #[error("not enough free disk space to compact the vault: it needs {} free on the vault's drive, and {} is free", compact::byte_text(*.needed), compact::byte_text(*.available))]
    NotEnoughDiskSpace { needed: u64, available: u64 },
    #[error("could not check the free disk space: {0}")]
    DiskSpace(std::io::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub struct Vault {
    conn: Connection,
    /// Held as long as the vault is open, so no other process opens it too. Released
    /// when dropped, and by the OS if the process exits or crashes.
    lock: lock::VaultLock,
    /// Rows changed through [`Vault::set_setting`] and [`Vault::delete_setting`], which
    /// [`Vault::change_count`] leaves out: opening a note records it there, and that
    /// is no reason for a background job to start over.
    setting_changes: Cell<u64>,
}

impl Vault {
    pub fn create(path: &Path, passphrase: &str) -> Result<Self> {
        if path.exists() {
            return Err(Error::VaultExists);
        }
        check_new_passphrase(passphrase)?;
        let lock = lock::VaultLock::acquire(path)?;
        let conn = connect(path, passphrase, OpenFlags::SQLITE_OPEN_CREATE)?;
        Self::init(conn, lock, &mut |_| {})
    }

    pub fn open(path: &Path, passphrase: &str) -> Result<Self> {
        Self::open_with_progress(path, passphrase, &mut |_| {})
    }

    /// [`Vault::open`], reporting the progress of a schema upgrade that rebuilds
    /// derived data (it can take minutes on a very large vault).
    pub fn open_with_progress(path: &Path, passphrase: &str, progress: &mut dyn FnMut(Upgrade)) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::VaultNotFound);
        }
        // Before the key is derived, so a vault open elsewhere fails fast.
        let lock = lock::VaultLock::acquire(path)?;
        let conn = connect(path, passphrase, OpenFlags::empty())?;
        Self::init(conn, lock, progress)
    }

    fn init(mut conn: Connection, lock: lock::VaultLock, progress: &mut dyn FnMut(Upgrade)) -> Result<Self> {
        schema::migrate(&mut conn, progress)?;
        notes::reset_bulk_writes(&conn)?;
        Ok(Self { conn, lock, setting_changes: Cell::new(0) })
    }

    /// Lets another thread stop the statement running on this connection.
    pub fn interrupt_handle(&self) -> rusqlite::InterruptHandle {
        self.conn.get_interrupt_handle()
    }

    /// Checks `passphrase` against the vault file on a separate, read-only connection.
    pub fn verify_passphrase(&self, passphrase: &str) -> Result<()> {
        let path = self.conn.path().ok_or(Error::VaultNotFound)?;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_key(&conn, passphrase)
    }

    pub fn change_passphrase(&self, new_passphrase: &str) -> Result<()> {
        check_new_passphrase(new_passphrase)?;
        self.conn.pragma_update(None, "rekey", new_passphrase)?;
        Ok(())
    }

    /// Closes the connection; SQLCipher wipes its key material on close.
    pub fn close(self) -> Result<()> {
        let Self { conn, lock, .. } = self;
        let closed = conn.close().map_err(|(_, e)| e.into());
        // Only after the connection is closed, so the next owner never sees it open.
        drop(lock);
        closed
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let n = self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        self.setting_changes.set(self.setting_changes.get() + n as u64);
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        let n = self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        self.setting_changes.set(self.setting_changes.get() + n as u64);
        Ok(())
    }
}

/// Checks a passphrase that is about to key a vault: on create and change, never on open.
fn check_new_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.is_empty() {
        return Err(Error::EmptyPassphrase);
    }
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(Error::PassphraseTooShort);
    }
    Ok(())
}

/// Whether vaults opened by this process turn on `cipher_memory_security` (#153).
static MEMORY_SECURITY: AtomicBool = AtomicBool::new(false);

/// Chooses, once at startup and before any vault opens, whether SQLCipher wipes the
/// memory it frees. Off by default: it makes word search about 3–4× and imports about
/// 2× slower (measured in #153). SQLCipher can't turn it off again within a process,
/// so a change made in Settings takes effect on the next start.
pub fn set_memory_security(on: bool) {
    MEMORY_SECURITY.store(on, Ordering::SeqCst);
}

/// Whether this process wipes freed SQLCipher memory, as chosen at startup.
pub fn memory_security() -> bool {
    MEMORY_SECURITY.load(Ordering::SeqCst)
}

fn connect(path: &Path, passphrase: &str, extra: OpenFlags) -> Result<Connection> {
    if passphrase.is_empty() {
        return Err(Error::EmptyPassphrase);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | extra,
    )?;
    // Before the key, as SQLCipher documents: SQLCipher then overwrites the memory it
    // frees, so decrypted pages don't stay in the heap after the vault locks (#153).
    // Opt-in (Settings → Security); process-wide, and it can't be turned off again.
    if memory_security() {
        run_pragma(&conn, "PRAGMA cipher_memory_security = ON")?;
    }
    apply_key(&conn, passphrase)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(conn)
}

/// Keys the connection and checks the key by reading from it.
fn apply_key(conn: &Connection, passphrase: &str) -> Result<()> {
    if passphrase.is_empty() {
        return Err(Error::EmptyPassphrase);
    }
    // SQLCipher logs decryption failures to stderr; keep logs silent.
    run_pragma(conn, "PRAGMA cipher_log_level = NONE")?;
    conn.pragma_update(None, "key", passphrase)?;
    // The key is only checked on the first read.
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)) {
        Ok(_) => Ok(()),
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::NotADatabase) => Err(Error::WrongPassphrase),
        Err(e) => Err(e.into()),
    }
}

/// Runs a pragma and discards whatever rows it returns.
fn run_pragma(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `“A”, “B”` for error messages.
fn quote_list(items: &[String]) -> String {
    items.iter().map(|t| format!("“{t}”")).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Memory wiping is opt-in (#153): no lib test turns it on, because it can't be
    /// turned off again in this process. `tests/vault.rs` checks the opt-in in a child
    /// process.
    #[test]
    fn open_vault_has_memory_security_off_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        Vault::create(&path, "test passphrase").unwrap().close().unwrap();
        let vault = Vault::open(&path, "test passphrase").unwrap();
        let on: String = vault.conn.query_row("PRAGMA cipher_memory_security", [], |r| r.get(0)).unwrap();
        assert_eq!(on, "0");
    }
}
