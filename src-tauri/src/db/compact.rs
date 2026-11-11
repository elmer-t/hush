//! Giving disk space back. Deleting notes frees pages inside the vault file, and SQLite
//! reuses them for new writes, but the file never shrinks by itself. [`Vault::compact`]
//! rewrites the file with `VACUUM`, which also switches the vault to
//! `auto_vacuum = INCREMENTAL`; from then on [`Vault::give_back_free_pages`] returns the
//! pages freed by emptying the trash without a full rewrite.
//!
//! Compacting first merges the full-text index, which keeps a delete marker for every
//! deleted note until its segments are merged.
//!
//! `VACUUM` runs on the vault's own connection, so SQLCipher writes the new file with the
//! same key and cipher settings. It is one transaction: if it fails part way (an error,
//! a full disk, a crash) the vault is left as it was. In WAL mode the new content is
//! written to the `-wal` file first and copied over the vault file by the checkpoint
//! that follows, so it needs about the vault's live size in free disk space.

use std::path::Path;

use serde::Serialize;

use super::{Error, Result, Vault, run_pragma};

/// Compacting is offered once at least this much can be given back...
pub const COMPACT_MIN_BYTES: u64 = 10 * 1024 * 1024;
/// ...and at least this share of the file (in percent): rewriting a large vault to give
/// back a sliver of it isn't worth the wait.
pub const COMPACT_MIN_PERCENT: u64 = 1;
/// Free disk space asked for on top of the live data, in percent of it...
const SPACE_MARGIN_PERCENT: u64 = 10;
/// ...and at least this much.
const SPACE_MARGIN_MIN: u64 = 64 * 1024 * 1024;

/// `PRAGMA auto_vacuum` of a vault compacted at least once.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;

/// How much of the vault file could be given back, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSpace {
    /// The vault file plus its `-wal` file.
    pub file_bytes: u64,
    /// Free pages inside the file: what compacting gives back now.
    pub reclaimable_bytes: u64,
    /// Notes in the trash, still taking space until the trash is emptied.
    pub trash_notes: u64,
    /// Their titles and bodies; the search index holds about as much again.
    pub trash_text_bytes: u64,
    /// Free disk space compacting needs on the vault's drive.
    pub needed_bytes: u64,
    /// Whether the vault gives freed space back by itself (compacted before).
    pub incremental: bool,
    /// Whether enough can be given back to offer "Compact now".
    pub worth_compacting: bool,
}

/// What a compaction did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compacted {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub millis: u64,
}

/// Free disk space for `needed` bytes on top of `live` bytes of data.
pub fn needed_bytes(live: u64) -> u64 {
    live + (live * SPACE_MARGIN_PERCENT / 100).max(SPACE_MARGIN_MIN)
}

/// Whether giving back `reclaimable` of a `file`-byte vault is worth a full rewrite.
pub fn worth_compacting(reclaimable: u64, file: u64) -> bool {
    reclaimable >= COMPACT_MIN_BYTES && reclaimable * 100 >= file * COMPACT_MIN_PERCENT
}

/// `1.5 MB`, `15 GB`: decimal units, as the frontend's `formatSize` shows them.
pub fn byte_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} B"),
        _ if value < 10.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

impl Vault {
    /// SQLCipher answers some pragmas itself, as text (`page_size` as `cipher_page_size`).
    fn pragma_u64(&self, name: &str) -> Result<u64> {
        use rusqlite::types::Value;
        let value = self.conn.pragma_query_value(None, name, |r| r.get::<_, Value>(0))?;
        Ok(match value {
            Value::Integer(n) => n.max(0) as u64,
            Value::Text(t) => t.trim().parse().unwrap_or(0),
            _ => 0,
        })
    }

    fn file_bytes(&self) -> u64 {
        let Some(path) = self.conn.path() else { return 0 };
        let size = |p: &str| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        size(path) + size(&format!("{path}-wal"))
    }

    /// Pages in use, in bytes: what a compacted vault would hold.
    fn live_bytes(&self) -> Result<u64> {
        let pages = self.pragma_u64("page_count")?.saturating_sub(self.pragma_u64("freelist_count")?);
        Ok(pages * self.pragma_u64("page_size")?)
    }

    fn incremental(&self) -> Result<bool> {
        Ok(self.pragma_u64("auto_vacuum")? == AUTO_VACUUM_INCREMENTAL as u64)
    }

    /// The vault's size and how much compacting would give back. Reads the trash
    /// sizes from the record headers (`octet_length` doesn't load the text).
    pub fn space(&self) -> Result<VaultSpace> {
        let reclaimable = self.pragma_u64("freelist_count")? * self.pragma_u64("page_size")?;
        let (trash_notes, trash_text): (i64, i64) = self.conn.query_row(
            "SELECT count(*), coalesce(sum(octet_length(title) + octet_length(body)), 0)
             FROM notes WHERE deleted_at IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let file_bytes = self.file_bytes();
        Ok(VaultSpace {
            file_bytes,
            reclaimable_bytes: reclaimable,
            trash_notes: trash_notes as u64,
            trash_text_bytes: trash_text as u64,
            needed_bytes: needed_bytes(self.live_bytes()?),
            incremental: self.incremental()?,
            worth_compacting: worth_compacting(reclaimable, file_bytes),
        })
    }

    /// Rewrites the vault file without its free pages and switches it to incremental
    /// vacuum. Checks first that the vault's drive has room for the rewrite, asking
    /// `available` (a function, so tests can pretend the disk is full) how much is free
    /// in the vault's folder. Takes as long as reading and writing the whole vault; it
    /// can't report progress.
    pub fn compact(&mut self, available: &dyn Fn(&Path) -> std::io::Result<u64>) -> Result<Compacted> {
        let started = std::time::Instant::now();
        let path = self.conn.path().ok_or(Error::VaultNotFound)?.to_owned();
        let folder = Path::new(&path).parent().unwrap_or(Path::new("."));
        let needed = needed_bytes(self.live_bytes()?);
        let free = available(folder).map_err(Error::DiskSpace)?;
        if free < needed {
            return Err(Error::NotEnoughDiskSpace { needed, available: free });
        }
        let before_bytes = self.file_bytes();
        // Deleting a note adds a delete marker to the search index rather than removing
        // its entries; merging the index into one segment drops them, freeing pages for
        // the VACUUM to give back. Search results don't change.
        self.conn.execute("INSERT INTO notes_fts(notes_fts) VALUES ('optimize')", [])?;
        // Start the VACUUM from an empty `-wal`, so it only ever holds the rewrite.
        run_pragma(&self.conn, "PRAGMA wal_checkpoint(TRUNCATE)")?;
        // Only takes effect through the VACUUM, which writes it into the file header.
        self.conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        if let Err(e) = self.conn.execute_batch("VACUUM") {
            // Back to what the file says, so nothing pending is left on the connection.
            let _ = self.conn.pragma_update(None, "auto_vacuum", "NONE");
            return Err(e.into());
        }
        run_pragma(&self.conn, "PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(Compacted { before_bytes, after_bytes: self.file_bytes(), millis: started.elapsed().as_millis() as u64 })
    }

    /// Gives the free pages back to the disk, without a full rewrite, once the vault
    /// was compacted (`auto_vacuum = INCREMENTAL`); a no-op before that. Run after
    /// notes are deleted for good.
    pub(super) fn give_back_free_pages(&self) -> Result<()> {
        if !self.incremental()? || self.pragma_u64("freelist_count")? == 0 {
            return Ok(());
        }
        run_pragma(&self.conn, "PRAGMA incremental_vacuum")?;
        // The file shrinks when the checkpoint copies the shorter vault over it.
        run_pragma(&self.conn, "PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }
}

/// Free disk space for this user in the folder `dir`.
pub fn available_space(dir: &Path) -> std::io::Result<u64> {
    imp::available_space(dir)
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn available_space(dir: &Path) -> std::io::Result<u64> {
        let c = CString::new(dir.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        #[allow(clippy::unnecessary_cast)]
        Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::PCWSTR;

    pub fn available_space(dir: &Path) -> std::io::Result<u64> {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "compact test passphrase";

    type Snapshot = (Vec<(i64, String, String, String)>, Vec<(i64, String, Vec<u8>, Vec<u8>)>, i64);

    fn plenty(_: &Path) -> std::io::Result<u64> {
        Ok(u64::MAX)
    }

    fn file_size(path: &Path) -> u64 {
        std::fs::metadata(path).unwrap().len()
    }

    fn pragma(v: &Vault, name: &str) -> i64 {
        v.pragma_u64(name).unwrap() as i64
    }

    /// Every live note, every vector, and what full-text search finds.
    fn snapshot(v: &Vault) -> Snapshot {
        let notes = v
            .conn
            .prepare("SELECT id, folder, title, body FROM notes WHERE deleted_at IS NULL ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let vectors = v
            .conn
            .prepare("SELECT note_id, model, content_hash, vector FROM note_embeddings ORDER BY note_id, model")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let found = v.conn.query_row("SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'keepme'", [], |r| r.get(0)).unwrap();
        (notes, vectors, found)
    }

    /// `n` notes of about 20 KB each, with a vector each; returns the vault, its path
    /// and the ids. The words come from a small vocabulary, as in real text, so the
    /// search index stays small next to the bodies (every deleted note adds a delete
    /// marker to it until FTS5 merges its segments).
    fn filled(n: usize) -> (tempfile::TempDir, std::path::PathBuf, Vault, Vec<i64>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.hush");
        let mut v = Vault::create(&path, PASS).unwrap();
        let mut ids = Vec::new();
        let mut seed = 7u64;
        for i in 0..n {
            let mut body = format!("keepme note {i}\n");
            while body.len() < 20_000 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                body.push_str(&format!("w{} ", (seed >> 33) % 500));
            }
            let id = v.create_note("f", &format!("Note {i}"), &body).unwrap();
            v.conn
                .execute("INSERT INTO note_embeddings VALUES (?1, 'm', x'01', ?2)", rusqlite::params![id, vec![i as u8; 1536]])
                .unwrap();
            ids.push(id);
        }
        (dir, path, v, ids)
    }

    /// Deletes all but every tenth note for good.
    fn delete_most(v: &mut Vault, ids: &[i64]) {
        let gone: Vec<i64> = ids.iter().copied().filter(|id| id % 10 != 0).collect();
        v.trash_notes(&gone).unwrap();
        v.empty_trash().unwrap();
    }

    #[test]
    fn compacting_shrinks_the_file_and_keeps_every_note() {
        let (_dir, path, mut v, ids) = filled(300);
        delete_most(&mut v, &ids);
        run_pragma(&v.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let space = v.space().unwrap();
        assert_eq!(pragma(&v, "auto_vacuum"), 0, "a new vault doesn't vacuum");
        assert!(space.reclaimable_bytes > 4_000_000, "{space:?}");
        assert!(!space.incremental, "{space:?}");
        let before = snapshot(&v);
        assert_eq!(before.0.len(), 30);
        assert_eq!(before.2, 30);
        let size_before = file_size(&path);

        let done = v.compact(&plenty).unwrap();
        assert!(done.after_bytes < done.before_bytes, "{done:?}");
        assert!(file_size(&path) < size_before / 2, "{} -> {}", size_before, file_size(&path));
        assert_eq!(pragma(&v, "freelist_count"), 0);
        assert_eq!(pragma(&v, "auto_vacuum"), AUTO_VACUUM_INCREMENTAL);
        assert_eq!(std::fs::metadata(format!("{}-wal", path.display())).map(|m| m.len()).unwrap_or(0), 0, "wal truncated");
        assert!(!v.space().unwrap().worth_compacting);
        v.close().unwrap();

        // Still encrypted, with the same passphrase, and nothing else lets it open.
        let header = std::fs::read(&path).unwrap();
        assert_ne!(&header[..16], b"SQLite format 3\0");
        assert!(matches!(Vault::open(&path, "wrong passphrase"), Err(Error::WrongPassphrase)));
        let v = Vault::open(&path, PASS).unwrap();
        assert_eq!(snapshot(&v), before);
        assert_eq!(pragma(&v, "auto_vacuum"), AUTO_VACUUM_INCREMENTAL);
        let check: String = v.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");
    }

    /// The cipher settings SQLCipher reports for the open vault don't change.
    #[test]
    fn compacting_keeps_the_cipher_settings() {
        let (_dir, path, mut v, ids) = filled(20);
        delete_most(&mut v, &ids);
        let settings = |v: &Vault| -> Vec<String> {
            ["cipher_page_size", "kdf_iter", "cipher_hmac_algorithm", "cipher_kdf_algorithm", "cipher_plaintext_header_size"]
                .iter()
                .map(|p| v.conn.pragma_query_value(None, p, |r| r.get::<_, String>(0)).unwrap())
                .collect()
        };
        let before = settings(&v);
        v.compact(&plenty).unwrap();
        v.close().unwrap();
        let v = Vault::open(&path, PASS).unwrap();
        assert_eq!(settings(&v), before);
    }

    /// Once compacted, emptying the trash gives the space back without another VACUUM.
    #[test]
    fn after_compacting_emptying_the_trash_shrinks_the_file() {
        let (_dir, path, mut v, ids) = filled(30);
        v.compact(&plenty).unwrap();
        let full = file_size(&path);
        let keep = snapshot(&v).0.into_iter().filter(|n| n.0 % 10 == 0).collect::<Vec<_>>();
        delete_most(&mut v, &ids);
        assert!(file_size(&path) < full * 2 / 3, "{full} -> {}", file_size(&path));
        assert_eq!(file_size(&path) as i64, pragma(&v, "page_count") * pragma(&v, "page_size"), "no free pages kept");
        assert_eq!(pragma(&v, "freelist_count"), 0);
        assert_eq!(snapshot(&v).0, keep);

        // The same for a trashed folder and for the purge on unlock.
        for i in 0..30 {
            v.create_note("g", &format!("G {i}"), &"x".repeat(20_000)).unwrap();
        }
        v.trash_folder("g").unwrap();
        let grown = file_size(&path);
        let group = v.trash_entries(0, 1).unwrap();
        let crate::db::TrashEntry::Folder { deleted_at, .. } = group[0] else { panic!("{group:?}") };
        v.delete_trash_folder(deleted_at).unwrap();
        assert!(file_size(&path) < grown, "folder: {grown} -> {}", file_size(&path));

        for i in 0..30 {
            let id = v.create_note("h", &format!("H {i}"), &"y".repeat(20_000)).unwrap();
            v.trash_note(id).unwrap();
        }
        run_pragma(&v.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let grown = file_size(&path);
        assert_eq!(v.purge_trash_at(i64::MAX).unwrap(), 30);
        assert!(file_size(&path) < grown, "purge: {grown} -> {}", file_size(&path));

        // And for one note deleted forever.
        let id = v.create_note("", "Big", &"z ".repeat(200_000)).unwrap();
        v.trash_note(id).unwrap();
        run_pragma(&v.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let grown = file_size(&path);
        v.delete_note(id).unwrap();
        assert!(file_size(&path) < grown, "one note: {grown} -> {}", file_size(&path));
        assert_eq!(pragma(&v, "freelist_count"), 0);
    }

    /// Before the first compaction, emptying the trash works as before and leaves the
    /// free pages for reuse.
    #[test]
    fn emptying_the_trash_of_a_vault_never_compacted_keeps_the_pages() {
        let (_dir, _path, mut v, ids) = filled(30);
        delete_most(&mut v, &ids);
        assert_eq!(pragma(&v, "auto_vacuum"), 0);
        assert!(pragma(&v, "freelist_count") > 0);
    }

    /// Calls back every 100 VM steps; stops the statement once `stop_at` calls passed
    /// (rusqlite's `hooks` feature, which wraps this, is off).
    struct Ticks {
        count: std::sync::atomic::AtomicUsize,
        stop_at: usize,
    }

    extern "C" fn tick(ticks: *mut std::ffi::c_void) -> std::ffi::c_int {
        let ticks = unsafe { &*(ticks as *const Ticks) };
        (ticks.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 > ticks.stop_at) as std::ffi::c_int
    }

    fn with_ticks<T>(v: &mut Vault, stop_at: usize, f: impl FnOnce(&mut Vault) -> T) -> (T, usize) {
        let ticks = Ticks { count: Default::default(), stop_at };
        let db = unsafe { v.conn.handle() };
        unsafe { rusqlite::ffi::sqlite3_progress_handler(db, 100, Some(tick), &ticks as *const _ as *mut _) };
        let out = f(v);
        unsafe { rusqlite::ffi::sqlite3_progress_handler(db, 0, None, std::ptr::null_mut()) };
        (out, ticks.count.into_inner())
    }

    /// A VACUUM stopped part way, at several points, leaves the vault as it was and
    /// readable with the same passphrase.
    #[test]
    fn a_failed_compaction_leaves_the_vault_intact() {
        let (_dir, path, mut v, ids) = filled(100);
        delete_most(&mut v, &ids);
        // Merge the search index first, so the stops below fall in the VACUUM.
        v.conn.execute("INSERT INTO notes_fts(notes_fts) VALUES ('optimize')", []).unwrap();
        run_pragma(&v.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let ((), merge_ticks) = with_ticks(&mut v, usize::MAX, |v| {
            v.conn.execute("INSERT INTO notes_fts(notes_fts) VALUES ('optimize')", []).unwrap();
        });
        assert!(merge_ticks < 5, "merging a merged index is next to free: {merge_ticks}");
        let (result, vacuum_ticks) = {
            let copy = tempfile::tempdir().unwrap();
            let copy_path = copy.path().join("v.hush");
            run_pragma(&v.conn, "PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
            std::fs::copy(&path, &copy_path).unwrap();
            let mut probe = Vault::open(&copy_path, PASS).unwrap();
            with_ticks(&mut probe, usize::MAX, |v| v.compact(&plenty).map(|_| ()))
        };
        result.unwrap();
        assert!(vacuum_ticks > 20, "{vacuum_ticks}");

        let before = snapshot(&v);
        let free = pragma(&v, "freelist_count");
        let size = file_size(&path);
        for stop_at in [merge_ticks + 1, vacuum_ticks / 4, vacuum_ticks / 2, vacuum_ticks * 9 / 10] {
            let (result, _) = with_ticks(&mut v, stop_at, |v| v.compact(&plenty));
            let err = result.unwrap_err();
            assert!(
                matches!(&err, Error::Sql(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted)),
                "{stop_at}: {err:?}"
            );
            assert_eq!(snapshot(&v), before, "{stop_at}");
            assert_eq!(pragma(&v, "freelist_count"), free, "{stop_at}");
            assert_eq!(pragma(&v, "auto_vacuum"), 0, "{stop_at}");
            assert_eq!(file_size(&path), size, "{stop_at}");
        }
        v.close().unwrap();

        let mut v = Vault::open(&path, PASS).unwrap();
        assert_eq!(snapshot(&v), before);
        assert_eq!(pragma(&v, "auto_vacuum"), 0);
        let check: String = v.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");
        // And it can still be compacted afterwards.
        v.compact(&plenty).unwrap();
        assert_eq!(snapshot(&v), before);
    }

    #[test]
    fn compacting_refuses_without_enough_disk_space() {
        let (dir, path, mut v, ids) = filled(30);
        delete_most(&mut v, &ids);
        let needed = v.space().unwrap().needed_bytes;
        let size = file_size(&path);
        let asked = std::cell::RefCell::new(None);
        let err = v
            .compact(&|p: &Path| {
                *asked.borrow_mut() = Some(p.to_owned());
                Ok(needed - 1)
            })
            .unwrap_err();
        assert_eq!(asked.into_inner().as_deref(), Some(dir.path()));
        assert!(matches!(err, Error::NotEnoughDiskSpace { needed: n, available } if n == needed && available == needed - 1));
        assert!(err.to_string().contains("free disk space"), "{err}");
        assert_eq!(pragma(&v, "auto_vacuum"), 0);
        assert_eq!(file_size(&path), size);

        let err = v.compact(&|_: &Path| Err(std::io::Error::other("no such drive"))).unwrap_err();
        assert!(matches!(err, Error::DiskSpace(_)), "{err:?}");

        // Exactly enough is enough.
        v.compact(&|_: &Path| Ok(needed)).unwrap();
    }

    #[test]
    fn space_counts_the_trash() {
        let (_dir, _path, mut v, ids) = filled(10);
        v.trash_notes(&ids[..4]).unwrap();
        let space = v.space().unwrap();
        assert_eq!(space.trash_notes, 4);
        assert!(space.trash_text_bytes >= 4 * 20_000, "{space:?}");
        assert!(space.needed_bytes >= SPACE_MARGIN_MIN);
        assert!(space.file_bytes > 200_000);
    }

    #[test]
    fn thresholds() {
        assert!(!worth_compacting(COMPACT_MIN_BYTES - 1, COMPACT_MIN_BYTES));
        assert!(worth_compacting(COMPACT_MIN_BYTES, 100 * COMPACT_MIN_BYTES));
        assert!(!worth_compacting(COMPACT_MIN_BYTES, 101 * COMPACT_MIN_BYTES));
        assert_eq!(needed_bytes(0), SPACE_MARGIN_MIN);
        assert_eq!(needed_bytes(10_000_000_000), 11_000_000_000);
        assert_eq!(byte_text(999), "999 B");
        assert_eq!(byte_text(1_500_000), "1.5 MB");
        assert_eq!(byte_text(15_000_000_000), "15 GB");
    }

    #[test]
    fn available_space_reads_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(available_space(dir.path()).unwrap() > 0);
        assert!(available_space(&dir.path().join("missing")).is_err());
    }
}
