//! Vaults written by an older bundled SQLCipher must keep opening after a rusqlite /
//! libsqlite3-sys upgrade (issue #164).
//!
//! `tests/fixtures/sqlcipher-4.14/` holds a vault written by rusqlite 0.40.2 /
//! libsqlite3-sys 0.38.2 (SQLCipher 4.14.0), keyed with `common::PASS`, and
//! `dump.txt`: every row of every table, as the old build read it.
//!
//! The same procedure also makes a copy taken while the vault was still open, whose
//! `-wal` file holds frames that were never checkpointed (as after a crash). It is
//! not committed (about 300 KB); to check it, point `HUSH_COMPAT_WAL_FIXTURE` at it
//! and the tests below run on both:
//!
//! ```text
//! # on a checkout whose Cargo.lock still has the old crates:
//! HUSH_FIXTURE_OUT=src-tauri/tests/fixtures/sqlcipher-4.14 \
//!   cargo test --manifest-path src-tauri/Cargo.toml --test sqlcipher_compat \
//!   -- --ignored make_fixture
//! # writes vault.hush and dump.txt there, and the open copy to <out>/wal/
//! HUSH_COMPAT_WAL_FIXTURE=<out>/wal cargo test --manifest-path src-tauri/Cargo.toml \
//!   --test sqlcipher_compat
//! ```

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use hush_lib::db::{Error, SearchQuery, Vault};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

const MODEL: &str = "fixture-4";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sqlcipher-4.14")
}

/// The committed fixture, and the uncheckpointed one when `HUSH_COMPAT_WAL_FIXTURE` is set.
fn fixtures() -> Vec<PathBuf> {
    let mut dirs = vec![fixture_dir()];
    if let Some(wal) = std::env::var_os("HUSH_COMPAT_WAL_FIXTURE") {
        let wal = PathBuf::from(wal);
        assert!(fs::metadata(wal.join("vault.hush-wal")).unwrap().len() > 0, "WAL fixture has frames");
        dirs.push(wal);
    }
    dirs
}

fn vector(seed: f32) -> Vec<f32> {
    vec![seed, 0.5, -0.25, 1.0 - seed]
}

/// Notes, tags, frontmatter, a trashed note, embeddings and a tag description.
fn populate_checkpointed(vault: &mut Vault) {
    vault.create_note("journal", "Heron day", "---\ntopic: life\nauthor: Jan\nrating: 4\n---\nherons by the river #bird").unwrap();
    vault.create_note("work", "Plan", "ship the #release on friday\n\n[[Heron day]]").unwrap();
    let gone = vault.create_note("", "Old", "to be trashed #bird").unwrap();
    vault.trash_note(gone).unwrap();
    vault.set_tag_description("bird", "Birds seen outside").unwrap();
    vault.set_embedding_model(MODEL).unwrap();
    let ids: Vec<i64> = vault.search(&SearchQuery::default()).unwrap().into_iter().map(|h| h.note.id).collect();
    let texts = vault.embedding_texts(&ids).unwrap();
    let vectors: Vec<_> = texts.iter().map(|t| (t.id, t.hash, vector(t.id as f32 / 10.0))).collect();
    vault.write_embeddings(MODEL, &vectors).unwrap();
    vault.finish_embedding_model(MODEL).unwrap();
    vault
        .write_tag_description_vectors(MODEL, &[("bird".into(), "Birds seen outside".into(), vector(0.9))])
        .unwrap();
}

/// Written after the last checkpoint, so it only lives in the `-wal` file.
fn populate_wal(vault: &mut Vault) {
    vault.create_note("journal", "In the WAL", "only in the write-ahead log #wal").unwrap();
    let plan = vault.search(&SearchQuery { text: "friday".into(), ..Default::default() }).unwrap()[0].note.id;
    vault.update_body(plan, "ship the #release on monday\n\n[[Heron day]]").unwrap();
}

/// A raw connection with the same key and cipher settings as `db::connect`, without
/// running migrations, so the file is read exactly as it is.
fn raw(path: &Path, flags: OpenFlags) -> Connection {
    let conn = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).unwrap();
    conn.pragma_update(None, "key", common::PASS).unwrap();
    conn
}

/// Every row of every ordinary table, one line each, sorted within a table.
fn dump(conn: &Connection) -> String {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND sql NOT LIKE 'CREATE VIRTUAL%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut out = String::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let n = stmt.column_count();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| {
                let cells: Vec<String> = (0..n)
                    .map(|i| match r.get_ref(i).unwrap() {
                        ValueRef::Null => "NULL".into(),
                        ValueRef::Integer(v) => v.to_string(),
                        ValueRef::Real(v) => format!("{v:?}"),
                        ValueRef::Text(t) => format!("{:?}", String::from_utf8_lossy(t)),
                        ValueRef::Blob(b) => format!("x'{}'", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
                    })
                    .collect();
                Ok(cells.join(" | "))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows.sort();
        out.push_str(&format!("== {table} ({} rows)\n", rows.len()));
        for row in rows {
            out.push_str(&row);
            out.push('\n');
        }
    }
    out
}

/// `PRAGMA cipher_settings` of a vault: what SQLCipher 4.14 used for every Hush vault.
const VAULT_CIPHER_SETTINGS: [&str; 6] = [
    "PRAGMA kdf_iter = 256000;",
    "PRAGMA cipher_page_size = 4096;",
    "PRAGMA cipher_use_hmac = 1;",
    "PRAGMA cipher_plaintext_header_size = 0;",
    "PRAGMA cipher_hmac_algorithm = HMAC_SHA512;",
    "PRAGMA cipher_kdf_algorithm = PBKDF2_HMAC_SHA512;",
];

fn rows(conn: &Connection, sql: &str) -> Vec<String> {
    conn.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn pragma(conn: &Connection, name: &str) -> String {
    conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, String>(0)).unwrap()
}

/// Copies the vault (and its `-wal`, if any) from `from`, so the test can't change it.
fn copy_fixture(from: &Path, to: &Path) -> PathBuf {
    for name in ["vault.hush", "vault.hush-wal"] {
        if from.join(name).exists() {
            fs::copy(from.join(name), to.join(name)).unwrap();
        }
    }
    to.join("vault.hush")
}

/// Run only by hand, with the old crates, to (re)make the fixture.
#[test]
#[ignore]
fn make_fixture() {
    let out = PathBuf::from(std::env::var("HUSH_FIXTURE_OUT").expect("set HUSH_FIXTURE_OUT"));
    fs::create_dir_all(&out).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");

    let mut vault = Vault::create(&path, common::PASS).unwrap();
    populate_checkpointed(&mut vault);
    vault.close().unwrap();
    let mut vault = Vault::open(&path, common::PASS).unwrap();
    populate_wal(&mut vault);
    // Copied while the vault is open: the WAL is not checkpointed, as after a crash.
    let wal = out.join("wal");
    fs::create_dir_all(&wal).unwrap();
    for name in ["vault.hush", "vault.hush-wal"] {
        fs::copy(dir.path().join(name), wal.join(name)).unwrap();
    }
    assert!(fs::metadata(wal.join("vault.hush-wal")).unwrap().len() > 0);
    // Closing checkpoints the WAL into the vault file.
    vault.close().unwrap();
    fs::copy(&path, out.join("vault.hush")).unwrap();

    let read = |from: &Path| {
        let scratch = tempfile::tempdir().unwrap();
        dump(&raw(&copy_fixture(from, scratch.path()), OpenFlags::SQLITE_OPEN_READ_ONLY))
    };
    let checkpointed = read(&out);
    assert_eq!(read(&wal), checkpointed, "both copies hold the same rows");
    fs::write(out.join("dump.txt"), checkpointed).unwrap();
}

/// Pins the bundled versions and the cipher defaults a vault's format depends on, so
/// an upgrade that changes one fails here instead of on a user's vault.
#[test]
fn bundled_sqlcipher_versions_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let conn = raw(&dir.path().join("v.db"), OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE);
    conn.execute("CREATE TABLE t(x)", []).unwrap();

    let sqlite: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0)).unwrap();
    let cipher = pragma(&conn, "cipher_version");
    let provider = pragma(&conn, "cipher_provider");
    println!("sqlite_version() = {sqlite}\ncipher_version = {cipher}\ncipher_provider = {provider}");

    let defaults = rows(&conn, "PRAGMA cipher_default_settings");
    let settings = rows(&conn, "PRAGMA cipher_settings");
    let other: Vec<String> = ["cipher", "cipher_hmac_pgno", "cipher_hmac_salt_mask"]
        .iter()
        .map(|name| format!("{name} = {}", pragma(&conn, name)))
        .collect();
    println!("{}\n{}\n{}", defaults.join("\n"), settings.join("\n"), other.join("\n"));
    assert_eq!(
        defaults,
        [
            "PRAGMA cipher_default_kdf_iter = 256000;",
            "PRAGMA cipher_default_page_size = 4096;",
            "PRAGMA cipher_default_use_hmac = 1;",
            "PRAGMA cipher_default_plaintext_header_size = 0;",
            "PRAGMA cipher_default_hmac_algorithm = HMAC_SHA512;",
            "PRAGMA cipher_default_kdf_algorithm = PBKDF2_HMAC_SHA512;",
        ]
    );
    assert_eq!(settings, VAULT_CIPHER_SETTINGS);
    assert_eq!(other, ["cipher = AES-256-CBC", "cipher_hmac_pgno = le", "cipher_hmac_salt_mask = 3a"]);
    let parts: Vec<u32> = sqlite.split('.').map(|p| p.parse().unwrap()).collect();
    assert!(parts >= vec![3, 54, 0], "RUSTSEC-2026-0459 is fixed in SQLite 3.54.0, bundled: {sqlite}");
}

#[test]
fn vault_from_sqlcipher_4_14_reads_identically() {
    for fixture in fixtures() {
        reads_identically(&fixture);
    }
}

fn reads_identically(fixture: &Path) {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(fixture, dir.path());

    let conn = raw(&path, OpenFlags::SQLITE_OPEN_READ_ONLY);
    let expected = fs::read_to_string(fixture_dir().join("dump.txt")).unwrap();
    assert_eq!(dump(&conn), expected);
    assert_eq!(rows(&conn, "PRAGMA cipher_settings"), VAULT_CIPHER_SETTINGS);
    assert_eq!(pragma(&conn, "integrity_check"), "ok");
    let cipher_errors = rows(&conn, "PRAGMA cipher_integrity_check");
    assert!(cipher_errors.is_empty(), "{cipher_errors:?}");
}

#[test]
fn vault_from_sqlcipher_4_14_opens_and_works() {
    for fixture in fixtures() {
        opens_and_works(&fixture);
    }
}

fn opens_and_works(fixture: &Path) {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(fixture, dir.path());

    assert!(matches!(Vault::open(&path, "wrong passphrase"), Err(Error::WrongPassphrase)));

    let mut vault = Vault::open(&path, common::PASS).unwrap();
    let hit = |text: &str| -> Vec<String> {
        vault
            .search(&SearchQuery { text: text.into(), ..Default::default() })
            .unwrap()
            .into_iter()
            .map(|h| h.note.title)
            .collect()
    };
    assert_eq!(hit("herons"), ["Heron day"]);
    assert_eq!(hit("write-ahead"), ["In the WAL"], "a note written after the first close");
    assert_eq!(hit("monday"), ["Plan"], "an edit written after the first close");
    assert!(hit("friday").is_empty());
    let heron = vault.search(&SearchQuery { text: "herons".into(), ..Default::default() }).unwrap()[0].note.id;
    let note = vault.note(heron).unwrap();
    assert_eq!((note.topic.as_deref(), note.author.as_deref(), note.rating), (Some("life"), Some("Jan"), Some(4)));
    assert_eq!(note.tags, ["bird"]);
    assert_eq!(vault.trashed_notes().unwrap().into_iter().map(|n| n.title).collect::<Vec<_>>(), ["Old"]);
    assert_eq!(vault.embedding_model_ready().unwrap().as_deref(), Some(MODEL));
    assert_eq!(vault.note_embedding(heron).unwrap(), Some(vector(heron as f32 / 10.0)));
    assert_eq!(vault.tag_description_vector("bird").unwrap(), Some((MODEL.into(), vector(0.9))));

    let id = vault.create_note("", "Written by the new build", "after the upgrade").unwrap();
    vault.close().unwrap();
    let vault = Vault::open(&path, common::PASS).unwrap();
    assert_eq!(vault.note(id).unwrap().body, "after the upgrade");
    vault.close().unwrap();

    let conn = raw(&path, OpenFlags::SQLITE_OPEN_READ_ONLY);
    assert_eq!(pragma(&conn, "integrity_check"), "ok");
}
