//! The encrypted vault: one SQLCipher database file per vault. All database access goes
//! through [`Vault`].

mod discover;
mod embeddings;
mod links;
mod lock;
mod metadata;
mod notes;
mod rules;
mod schema;
mod search;
mod tags;
mod trash;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};

pub use discover::NoteVector;
pub use embeddings::{EMBEDDING_MODEL_READY_SETTING, EMBEDDING_MODEL_SETTING, EmbeddingScan, NoteText};
pub use links::{Backlink, OutgoingLink, Resolution};
pub use lock::lock_path;
pub use metadata::{MetadataEdit, MetadataEditReport, TitledNote};
pub use notes::{NewNote, Note, NoteSummary};
pub use search::{DateField, Facets, HIGHLIGHT_END, HIGHLIGHT_START, SearchHit, SearchQuery};
pub use tags::{SUGGEST_THRESHOLD_SETTING, SUGGESTIONS_PER_NOTE, TagFeedback, TagInfo, TagSuggestion};
pub use trash::{DEFAULT_TRASH_DAYS, MAX_TRASH_DAYS, TRASH_DAYS_SETTING, TrashedNote};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wrong passphrase, or not a Hush vault")]
    WrongPassphrase,
    #[error("passphrase must not be empty")]
    EmptyPassphrase,
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
    #[error("invalid tag: {0}")]
    InvalidTag(String),
    #[error("{0}")]
    InvalidRule(String),
    #[error("trash retention must be between 0 and {} days", trash::MAX_TRASH_DAYS)]
    InvalidTrashDays,
    #[error("the suggestion threshold must be a number between -1 and 1")]
    InvalidThreshold,
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub struct Vault {
    conn: Connection,
    /// Held as long as the vault is open, so no other process opens it too. Released
    /// when dropped, and by the OS if the process exits or crashes.
    lock: lock::VaultLock,
}

impl Vault {
    pub fn create(path: &Path, passphrase: &str) -> Result<Self> {
        if path.exists() {
            return Err(Error::VaultExists);
        }
        let lock = lock::VaultLock::acquire(path)?;
        let conn = connect(path, passphrase, OpenFlags::SQLITE_OPEN_CREATE)?;
        Self::init(conn, lock)
    }

    pub fn open(path: &Path, passphrase: &str) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::VaultNotFound);
        }
        // Before the key is derived, so a vault open elsewhere fails fast.
        let lock = lock::VaultLock::acquire(path)?;
        let conn = connect(path, passphrase, OpenFlags::empty())?;
        Self::init(conn, lock)
    }

    fn init(mut conn: Connection, lock: lock::VaultLock) -> Result<Self> {
        schema::migrate(&mut conn)?;
        Ok(Self { conn, lock })
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
        if new_passphrase.is_empty() {
            return Err(Error::EmptyPassphrase);
        }
        self.conn.pragma_update(None, "rekey", new_passphrase)?;
        Ok(())
    }

    /// Closes the connection; SQLCipher wipes its key material on close.
    pub fn close(self) -> Result<()> {
        let Self { conn, lock } = self;
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
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

fn connect(path: &Path, passphrase: &str, extra: OpenFlags) -> Result<Connection> {
    if passphrase.is_empty() {
        return Err(Error::EmptyPassphrase);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | extra,
    )?;
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
